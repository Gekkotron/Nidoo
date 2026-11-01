//! The learner (spec §6.2–§6.3): from the devices' target history to a proposed rule set, plus the
//! plain-English diff between two rule sets and a fingerprint to remember rejections.
//!
//! Pure and deterministic. The hub builds the inputs from history and the command ledger.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime};

use crate::decide::{DeviceCaps, fit, mode_str};
use crate::doc::{Block, Day, Evidence, HhMm, Mode, ModeRules, Rules, Setpoint, Source};

/// Slots per day (15 minutes each).
pub const SLOTS: usize = 96;
const SLOT_MIN: i64 = 15;
/// Two temperatures this close count as the same value.
const SAME_TOL: f64 = 0.5;
/// A device target this close to the engine's command is the engine's own.
const ENGINE_TOL: f64 = 0.25;
/// A day counts as data when at least this many slots are known.
const MIN_KNOWN_SLOTS: usize = 48;
/// A day deviates from its group's median when it differs on more than this share of slots.
const DEVIATION_SHARE: f64 = 0.25;
/// Runs shorter than this many slots (30 min) are absorbed by a neighbour.
const MIN_RUN: usize = 2;
/// A day agrees with a block when it starts and ends within this many slots (30 min) of it.
const EDGE_SLACK: usize = 2;
/// `differs` / `diff` thresholds: 30 minutes, 0.5 degrees.
const SHIFT_MIN: u16 = 30;
const SHIFT_TEMP: f64 = 0.5;

/// One recorded target of a device, in local time. A `None` mode means the mode being learned.
#[derive(Clone, Debug, PartialEq)]
pub struct TargetSample {
    pub at: NaiveDateTime,
    pub target: Option<f64>,
    pub mode: Option<String>,
}

/// A device's samples, sorted by time. The first may predate the window and carries the value in
/// force at its start.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceHistory {
    pub entity: String,
    pub samples: Vec<TargetSample>,
}

/// A command the engine applied, from its decision log.
#[derive(Clone, Debug, PartialEq)]
pub struct EngineCommand {
    pub entity: String,
    pub at: NaiveDateTime,
    pub setpoint: Option<f64>,
    pub mode: String,
    pub source: Source,
    /// The correction added to `setpoint` when the device was commanded: it received
    /// `setpoint + adjust`. `None` when the command was sent as is.
    pub adjust: Option<f64>,
}

pub struct LearnInput<'a> {
    pub mode: Mode,
    pub devices: &'a [DeviceHistory],
    pub engine: &'a [EngineCommand],
    /// The rules in force, if any. A slot keeps its approved value unless enough days point to
    /// another one (§6.2); callers compare the result with [`differs`].
    pub approved: Option<&'a ModeRules>,
    pub first_day: NaiveDate,
    pub days: u32,
    pub min_days: u32,
    pub support: f64,
    pub frost: f64,
    pub caps: &'a DeviceCaps,
}

/// The effective target of the zone during one slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Slot {
    None,
    Off,
    Temp(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub enum LearnOutcome {
    NotEnough { days: u32, needed: u32 },
    Rules(ModeRules),
}

fn same(a: Slot, b: Slot) -> bool {
    match (a, b) {
        (Slot::Off, Slot::Off) => true,
        (Slot::Temp(x), Slot::Temp(y)) => (x - y).abs() <= SAME_TOL,
        _ => false,
    }
}

/// Order of values from "lowest" to "highest" for ranking: `Off` is below every temperature in
/// heat and above every temperature in cool.
fn rank(s: Slot, mode: Mode) -> f64 {
    match (s, mode) {
        (Slot::Temp(t), _) => t,
        (Slot::Off, Mode::Heat) => f64::NEG_INFINITY,
        (Slot::Off, Mode::Cool) => f64::INFINITY,
        (Slot::None, _) => f64::NAN,
    }
}

// ---------------------------------------------------------------------------------------------
// Timeline
// ---------------------------------------------------------------------------------------------

/// The zone's effective target per day and slot (§6.2).
pub fn timeline(input: &LearnInput) -> Vec<(NaiveDate, [Slot; SLOTS])> {
    let commands: Vec<Vec<&EngineCommand>> = input
        .devices
        .iter()
        .map(|d| {
            let mut v: Vec<&EngineCommand> = input
                .engine
                .iter()
                .filter(|c| c.entity == d.entity)
                .collect();
            v.sort_by_key(|c| c.at);
            v
        })
        .collect();
    let samples: Vec<Vec<TargetSample>> = input
        .devices
        .iter()
        .map(|d| {
            let mut v = d.samples.clone();
            v.sort_by_key(|s| s.at);
            v
        })
        .collect();
    (0..input.days)
        .map(|i| {
            let date = input.first_day + Duration::days(i64::from(i));
            let midnight = date.and_time(NaiveTime::MIN);
            let mut slots = [Slot::None; SLOTS];
            for (n, slot) in slots.iter_mut().enumerate() {
                let start = midnight + Duration::minutes(SLOT_MIN * n as i64);
                *slot = combine(
                    samples
                        .iter()
                        .zip(&commands)
                        .map(|(d, c)| device_slot(d, c, input.mode, start).unwrap_or(Slot::None)),
                );
            }
            (date, slots)
        })
        .collect()
}

/// The mean of the devices' temperatures, `Off` when every contributing device is off.
fn combine(parts: impl Iterator<Item = Slot>) -> Slot {
    let (mut temps, mut off) = (Vec::new(), false);
    for p in parts {
        match p {
            Slot::Temp(t) => temps.push(t),
            Slot::Off => off = true,
            Slot::None => {}
        }
    }
    if !temps.is_empty() {
        temps.sort_by(f64::total_cmp);
        Slot::Temp(temps.iter().sum::<f64>() / temps.len() as f64)
    } else if off {
        Slot::Off
    } else {
        Slot::None
    }
}

/// What one device held for most of the slot starting at `start`.
fn device_slot(
    samples: &[TargetSample],
    commands: &[&EngineCommand],
    mode: Mode,
    start: NaiveDateTime,
) -> Option<Slot> {
    let end = start + Duration::minutes(SLOT_MIN);
    let idx = samples.partition_point(|s| s.at <= start);
    let mut cur = idx.checked_sub(1);
    let mut from = start;
    let mut best: (Option<usize>, NaiveDateTime, i64) = (None, start, -1);
    let mut consider = |s: Option<usize>, from: NaiveDateTime, to: NaiveDateTime| {
        let d = (to - from).num_seconds();
        if d > best.2 {
            best = (s, from, d);
        }
    };
    for (i, next) in samples.iter().enumerate().skip(idx) {
        if next.at >= end {
            break;
        }
        consider(cur, from, next.at);
        cur = Some(i);
        from = next.at;
    }
    consider(cur, from, end);
    let (sample, held_from, _) = best;
    let sample = &samples[sample?];

    let wanted = mode_str(mode);
    let device_mode = sample.mode.as_deref().unwrap_or(wanted);
    let value = if device_mode == "off" {
        Slot::Off
    } else if device_mode == wanted {
        Slot::Temp(sample.target?)
    } else {
        return None;
    };
    let engine = commands.iter().rev().find(|c| c.at <= held_from);
    if let Some(c) = engine
        && c.mode == device_mode
        && (device_mode == "off"
            || matches!((c.setpoint, sample.target),
                (Some(a), Some(b)) if (a + c.adjust.unwrap_or(0.0) - b).abs() <= ENGINE_TOL))
        && matches!(
            c.source,
            Source::Safety
                | Source::Away
                | Source::Eco
                | Source::Empty
                | Source::Holiday
                | Source::Override
        )
    {
        return None;
    }
    // A corrected command moved the device off the target on purpose: the target is what the
    // slot held.
    if let Some(c) = engine
        && let (Some(adjust), Some(setpoint)) = (c.adjust, c.setpoint)
        && c.mode == device_mode
        && device_mode != "off"
        && matches!(sample.target, Some(b) if (setpoint + adjust - b).abs() <= ENGINE_TOL)
    {
        return Some(Slot::Temp(setpoint));
    }
    Some(value)
}

// ---------------------------------------------------------------------------------------------
// Learning
// ---------------------------------------------------------------------------------------------

struct DayData {
    day: Day,
    slots: [Slot; SLOTS],
}

#[derive(Clone, Copy, Debug)]
struct Run {
    start: usize,
    len: usize,
    value: Slot,
}

/// Which edges of a run a day must keep to agree with it.
#[derive(Clone, Copy)]
struct Edges {
    start: bool,
    end: bool,
}

impl Edges {
    const BOTH: Edges = Edges {
        start: true,
        end: true,
    };
}

struct Group {
    days: Vec<Day>,
    members: Vec<usize>,
    runs: Vec<Run>,
}

fn is_weekend(d: Day) -> bool {
    matches!(d, Day::Sat | Day::Sun)
}

/// Learn the rules of one mode from the device history (§6.3).
pub fn learn(input: &LearnInput) -> LearnOutcome {
    let counted: Vec<DayData> = timeline(input)
        .into_iter()
        .filter(|(_, s)| s.iter().filter(|x| **x != Slot::None).count() >= MIN_KNOWN_SLOTS)
        .map(|(d, slots)| DayData {
            day: Day::from_chrono(d.weekday()),
            slots,
        })
        .collect();
    let n = u32::try_from(counted.len()).unwrap_or(u32::MAX);
    if n < input.min_days {
        return LearnOutcome::NotEnough {
            days: n,
            needed: input.min_days,
        };
    }

    let mut groups = make_groups(&counted, input);
    for g in &mut groups {
        let median = filled(median_day(&counted, &g.members, input.mode));
        g.runs = runs_of(&median, input.mode);
    }
    let Some(base) = pick_base(&groups, input.mode) else {
        return LearnOutcome::Rules(ModeRules {
            base: Setpoint::Off,
            blocks: Vec::new(),
        });
    };
    let base_sp = finalize(base, input);
    if let Some(approved) = input.approved {
        return LearnOutcome::Rules(revise(&counted, &groups, base_sp, approved, input));
    }

    let mut blocks: Vec<Block> = Vec::new();
    for g in &groups {
        let total = g.members.len() as u32;
        for run in g.runs.iter().filter(|r| !same(r.value, base)) {
            let setpoint = finalize(run.value, input);
            if setpoint == base_sp {
                continue;
            }
            // Days agree with the block as it will be set, not with the raw median.
            let block_run = Run {
                value: slot_of(setpoint),
                ..*run
            };
            let agree = g
                .members
                .iter()
                .filter(|&&m| day_agrees(&counted[m].slots, &block_run, Edges::BOTH))
                .count() as u32;
            if f64::from(agree) < input.support * f64::from(total) {
                continue;
            }
            let block = Block {
                days: g.days.clone(),
                from: HhMm((run.start as u16) * SLOT_MIN as u16),
                to: HhMm(((run.start + run.len) as u16) * SLOT_MIN as u16),
                setpoint,
                evidence: Some(Evidence { agree, of: total }),
            };
            merge_block(&mut blocks, block);
        }
    }
    for b in &mut blocks {
        b.days.sort();
    }
    blocks.sort_by_key(|b| (b.days[0], b.from));
    LearnOutcome::Rules(ModeRules {
        base: base_sp,
        blocks,
    })
}

/// Identical blocks of different groups become one block over the union of days.
fn merge_block(blocks: &mut Vec<Block>, block: Block) {
    let found = blocks
        .iter_mut()
        .find(|b| b.from == block.from && b.to == block.to && b.setpoint == block.setpoint);
    match found {
        Some(b) => {
            b.days.extend(block.days);
            if let (Some(e), Some(add)) = (b.evidence.as_mut(), block.evidence) {
                e.agree += add.agree;
                e.of += add.of;
            }
        }
        None => blocks.push(block),
    }
}

/// The learned rules against the approved ones (§6.2): a slot keeps its approved value unless
/// the group's median differs from it there and enough of the group's days hold the median over
/// that stretch. Scattered hand changes therefore never remove a block, and days without data
/// keep theirs.
fn revise(
    counted: &[DayData],
    groups: &[Group],
    learned_base: Setpoint,
    approved: &ModeRules,
    input: &LearnInput,
) -> ModeRules {
    let base = if close(approved.base, learned_base) {
        approved.base
    } else {
        learned_base
    };
    let mut plan: Vec<[Setpoint; SLOTS]> = Day::ALL
        .iter()
        .map(|&d| {
            let mut day = [base; SLOTS];
            for (s, v) in day.iter_mut().enumerate() {
                if let Some(b) = block_at(approved, d, s) {
                    *v = b.setpoint;
                }
            }
            day
        })
        .collect();
    for g in groups {
        let total = g.members.len() as f64;
        for run in &g.runs {
            let learned = finalize(run.value, input);
            let end = run.start + run.len;
            for &d in &g.days {
                let day = &mut plan[day_index(d)];
                let mut s = run.start;
                while s < end {
                    if close(day[s], learned) {
                        s += 1;
                        continue;
                    }
                    let e = (s + 1..end)
                        .find(|&e| close(day[e], learned))
                        .unwrap_or(end);
                    let stretch = Run {
                        start: s,
                        len: e - s,
                        value: slot_of(learned),
                    };
                    // Only the run's own edges are edges a day must keep: inside the run the
                    // stretch borders slots already at the learned value.
                    let edges = Edges {
                        start: s == run.start,
                        end: e == end,
                    };
                    let agree = g
                        .members
                        .iter()
                        .filter(|&&m| day_agrees(&counted[m].slots, &stretch, edges))
                        .count();
                    if agree as f64 >= input.support * total {
                        day[s..e].fill(learned);
                    }
                    s = e;
                }
            }
        }
    }

    // Each day's stretches off the base; the same stretch on several days is one block.
    let mut found: Vec<(HhMm, HhMm, Setpoint, Vec<Day>)> = Vec::new();
    for (day, d) in plan.iter().zip(Day::ALL) {
        let mut s = 0;
        while s < SLOTS {
            let value = day[s];
            let e = (s + 1..SLOTS).find(|&e| day[e] != value).unwrap_or(SLOTS);
            if value != base {
                let (from, to) = (slot_time(s), slot_time(e));
                match found
                    .iter_mut()
                    .find(|f| f.0 == from && f.1 == to && f.2 == value)
                {
                    Some(f) => f.3.push(d),
                    None => found.push((from, to, value, vec![d])),
                }
            }
            s = e;
        }
    }
    let mut blocks: Vec<Block> = found
        .into_iter()
        .map(|(from, to, setpoint, days)| {
            // An approved block left as it was keeps its evidence.
            let evidence = match approved.blocks.iter().find(|b| {
                same_days(&b.days, &days) && b.from == from && b.to == to && b.setpoint == setpoint
            }) {
                Some(b) => b.evidence.clone(),
                None => evidence_for(counted, &days, from, to, setpoint),
            };
            Block {
                days,
                from,
                to,
                setpoint,
                evidence,
            }
        })
        .collect();
    blocks.sort_by_key(|b| (b.days[0], b.from));
    ModeRules { base, blocks }
}

/// Two setpoints within 0.5 °C of each other, or both off.
fn close(a: Setpoint, b: Setpoint) -> bool {
    same(slot_of(a), slot_of(b))
}

fn slot_of(s: Setpoint) -> Slot {
    match s {
        Setpoint::Temp(t) => Slot::Temp(t),
        Setpoint::Off => Slot::Off,
    }
}

fn slot_time(slot: usize) -> HhMm {
    HhMm((slot as u16) * SLOT_MIN as u16)
}

fn day_index(d: Day) -> usize {
    Day::ALL.iter().position(|x| *x == d).unwrap_or(0)
}

/// The block of `rules` covering `slot` on `day`, if any.
fn block_at(rules: &ModeRules, day: Day, slot: usize) -> Option<&Block> {
    let minute = slot_time(slot).0;
    rules
        .blocks
        .iter()
        .find(|b| b.days.contains(&day) && b.from.0 <= minute && minute < b.to.0)
}

/// How many of the counted days on `days` agree with a block; `None` without any such day.
fn evidence_for(
    counted: &[DayData],
    days: &[Day],
    from: HhMm,
    to: HhMm,
    setpoint: Setpoint,
) -> Option<Evidence> {
    let run = Run {
        start: usize::from(from.0) / SLOT_MIN as usize,
        len: usize::from(to.0 - from.0) / SLOT_MIN as usize,
        value: slot_of(setpoint),
    };
    let on_days: Vec<&DayData> = counted.iter().filter(|c| days.contains(&c.day)).collect();
    let of = u32::try_from(on_days.len()).unwrap_or(u32::MAX);
    let agree = on_days
        .iter()
        .filter(|c| day_agrees(&c.slots, &run, Edges::BOTH))
        .count() as u32;
    (of > 0).then_some(Evidence { agree, of })
}

fn finalize(value: Slot, input: &LearnInput) -> Setpoint {
    match value {
        Slot::Temp(t) => {
            let t = match input.mode {
                Mode::Heat => t.max(input.frost),
                Mode::Cool => t,
            };
            Setpoint::Temp(fit(t, input.caps))
        }
        Slot::Off | Slot::None => Setpoint::Off,
    }
}

/// Weekdays and weekend days; a group whose days disagree splits into one group per weekday.
fn make_groups(counted: &[DayData], input: &LearnInput) -> Vec<Group> {
    let mut groups = Vec::new();
    for weekend in [false, true] {
        let members: Vec<usize> = (0..counted.len())
            .filter(|&i| is_weekend(counted[i].day) == weekend)
            .collect();
        if members.is_empty() {
            continue;
        }
        if disagrees(counted, &members, input) {
            for d in Day::ALL {
                let same_day: Vec<usize> = members
                    .iter()
                    .copied()
                    .filter(|&i| counted[i].day == d)
                    .collect();
                if !same_day.is_empty() {
                    groups.push(Group {
                        days: vec![d],
                        members: same_day,
                        runs: Vec::new(),
                    });
                }
            }
        } else {
            let mut days: Vec<Day> = Vec::new();
            for &i in &members {
                if !days.contains(&counted[i].day) {
                    days.push(counted[i].day);
                }
            }
            days.sort();
            groups.push(Group {
                days,
                members,
                runs: Vec::new(),
            });
        }
    }
    groups
}

/// More than `1 - support` of the days differ from the group's median on over a quarter of the
/// slots they share with it. A few odd days lower a block's evidence without splitting the group.
fn disagrees(counted: &[DayData], members: &[usize], input: &LearnInput) -> bool {
    let median = median_day(counted, members, input.mode);
    let deviant = members
        .iter()
        .filter(|&&m| {
            let (mut compared, mut differing) = (0u32, 0u32);
            for (a, b) in counted[m].slots.iter().zip(&median) {
                if *a != Slot::None && *b != Slot::None {
                    compared += 1;
                    differing += u32::from(!same(*a, *b));
                }
            }
            f64::from(differing) > DEVIATION_SHARE * f64::from(compared)
        })
        .count();
    deviant as f64 > (1.0 - input.support) * members.len() as f64
}

/// The per-slot median of the group's known values; `Slot::None` where no day knows. With an even
/// count the lower middle value wins.
fn median_day(counted: &[DayData], members: &[usize], mode: Mode) -> [Slot; SLOTS] {
    let mut out = [Slot::None; SLOTS];
    for (s, slot) in out.iter_mut().enumerate() {
        let mut values: Vec<Slot> = members
            .iter()
            .map(|&m| counted[m].slots[s])
            .filter(|v| *v != Slot::None)
            .collect();
        values.sort_by(|a, b| rank(*a, mode).total_cmp(&rank(*b, mode)));
        if !values.is_empty() {
            *slot = values[(values.len() - 1) / 2];
        }
    }
    out
}

/// Unknown slots take the previous known value (the first ones, the next).
fn filled(day: [Slot; SLOTS]) -> Vec<Slot> {
    let first = day.iter().copied().find(|s| *s != Slot::None);
    let mut last = first.unwrap_or(Slot::Off);
    day.iter()
        .map(|s| {
            if *s != Slot::None {
                last = *s;
            }
            last
        })
        .collect()
}

/// Cut a day into runs of similar slots, absorb runs shorter than 30 minutes, and give each run
/// the median of its slots.
fn runs_of(day: &[Slot], mode: Mode) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::new();
    for (i, &v) in day.iter().enumerate() {
        match runs.last_mut() {
            Some(r) if same(r.value, v) => r.len += 1,
            _ => runs.push(Run {
                start: i,
                len: 1,
                value: v,
            }),
        }
    }
    loop {
        let short = runs.iter().position(|r| r.len < MIN_RUN);
        let similar = (1..runs.len()).find(|&i| same(runs[i - 1].value, runs[i].value));
        let (a, b) = match (short, similar) {
            (Some(i), _) if runs.len() > 1 => {
                if i > 0 {
                    (i - 1, i)
                } else {
                    (0, 1)
                }
            }
            (_, Some(i)) => (i - 1, i),
            _ => break,
        };
        // The longer run keeps its value; on a tie, the earlier one.
        let value = if runs[b].len > runs[a].len {
            runs[b].value
        } else {
            runs[a].value
        };
        runs[a].len += runs[b].len;
        runs[a].value = value;
        runs.remove(b);
    }
    for r in &mut runs {
        let mut values: Vec<Slot> = day[r.start..r.start + r.len].to_vec();
        values.sort_by(|a, b| rank(*a, mode).total_cmp(&rank(*b, mode)));
        r.value = values[(values.len() - 1) / 2];
    }
    runs
}

/// The run value held for the most slot-days across all groups. Ties go to the lowest value
/// in heat and the highest in cool (`Off` counts as a value).
fn pick_base(groups: &[Group], mode: Mode) -> Option<Slot> {
    let mut tally: Vec<(Slot, usize)> = Vec::new();
    for g in groups {
        for r in &g.runs {
            match tally.iter_mut().find(|(v, _)| same(*v, r.value)) {
                Some((_, n)) => *n += r.len * g.members.len(),
                None => tally.push((r.value, r.len * g.members.len())),
            }
        }
    }
    let key = |s: Slot| match mode {
        Mode::Heat => rank(s, mode),
        Mode::Cool => -rank(s, mode),
    };
    tally
        .into_iter()
        .min_by(|(a, na), (b, nb)| nb.cmp(na).then(key(*a).total_cmp(&key(*b))))
        .map(|(v, _)| v)
}

/// A day agrees with a run when at least half of its slots are known, 80 % of the known ones
/// are within 0.5 °C of the run's value, and at each of `edges` the day reaches that value
/// within 30 min of the edge without holding it from further away (§6.3: start and end within
/// 30 min). An unknown slot never counts against the day.
fn day_agrees(slots: &[Slot; SLOTS], run: &Run, edges: Edges) -> bool {
    let end = run.start + run.len;
    let range = &slots[run.start..end];
    let known = range.iter().filter(|s| **s != Slot::None).count();
    let matching = range.iter().filter(|s| same(**s, run.value)).count();
    if known * 2 < run.len || matching * 5 < known * 4 {
        return false;
    }
    let holds = |s: usize| slots.get(s).is_some_and(|v| same(*v, run.value));
    let near = |edge: usize| {
        (edge.saturating_sub(EDGE_SLACK)..=(edge + EDGE_SLACK).min(SLOTS - 1)).any(holds)
    };
    let starts = !edges.start
        || (near(run.start) && !run.start.checked_sub(EDGE_SLACK + 1).is_some_and(holds));
    let ends = !edges.end || (near(end - 1) && !holds(end + EDGE_SLACK));
    starts && ends
}

// ---------------------------------------------------------------------------------------------
// Diff
// ---------------------------------------------------------------------------------------------

fn same_days(a: &[Day], b: &[Day]) -> bool {
    a.len() == b.len() && a.iter().all(|d| b.contains(d))
}

fn setpoints_differ(a: Setpoint, b: Setpoint) -> bool {
    match (a, b) {
        (Setpoint::Off, Setpoint::Off) => false,
        (Setpoint::Temp(x), Setpoint::Temp(y)) => (x - y).abs() >= SHIFT_TEMP,
        _ => true,
    }
}

struct Pairing {
    pairs: Vec<(usize, usize)>,
    removed: Vec<usize>,
    added: Vec<usize>,
}

/// Match each old block with an unused new block on the same days that overlaps it in time.
fn pair(old: &ModeRules, new: &ModeRules) -> Pairing {
    let mut used = vec![false; new.blocks.len()];
    let (mut pairs, mut removed) = (Vec::new(), Vec::new());
    for (i, o) in old.blocks.iter().enumerate() {
        let found = new.blocks.iter().enumerate().find(|(j, n)| {
            !used[*j] && same_days(&o.days, &n.days) && o.from < n.to && n.from < o.to
        });
        match found {
            Some((j, _)) => {
                used[j] = true;
                pairs.push((i, j));
            }
            None => removed.push(i),
        }
    }
    let added = (0..new.blocks.len()).filter(|j| !used[*j]).collect();
    Pairing {
        pairs,
        removed,
        added,
    }
}

fn start_moved(o: &Block, n: &Block) -> bool {
    o.from.0.abs_diff(n.from.0) >= SHIFT_MIN
}

fn end_moved(o: &Block, n: &Block) -> bool {
    o.to.0.abs_diff(n.to.0) >= SHIFT_MIN
}

/// Whether the new rules of a mode differ from the old ones enough to propose a change (§6.3).
pub fn differs(old: &ModeRules, new: &ModeRules) -> bool {
    if setpoints_differ(old.base, new.base) {
        return true;
    }
    let p = pair(old, new);
    !p.removed.is_empty()
        || !p.added.is_empty()
        || p.pairs.iter().any(|&(i, j)| {
            let (o, n) = (&old.blocks[i], &new.blocks[j]);
            start_moved(o, n) || end_moved(o, n) || setpoints_differ(o.setpoint, n.setpoint)
        })
}

fn days_text(days: &[Day]) -> String {
    let has = |d: Day| days.contains(&d);
    let weekdays = [Day::Mon, Day::Tue, Day::Wed, Day::Thu, Day::Fri];
    if days.len() == 7 {
        return "every day".into();
    }
    if days.len() == 5 && weekdays.iter().all(|d| has(*d)) {
        return "weekdays".into();
    }
    if days.len() == 2 && has(Day::Sat) && has(Day::Sun) {
        return "weekends".into();
    }
    let mut sorted = days.to_vec();
    sorted.sort();
    sorted
        .iter()
        .map(|d| format!("{d:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

fn time_text(t: HhMm) -> String {
    format!("{:02}:{:02}", t.0 / 60, t.0 % 60)
}

fn setpoint_text(s: Setpoint) -> String {
    match s {
        Setpoint::Temp(t) => format!("{t:.1} °C"),
        Setpoint::Off => "off".into(),
    }
}

fn block_text(b: &Block, evidence: bool) -> String {
    let mut s = format!(
        "{} {}–{} at {}",
        days_text(&b.days),
        time_text(b.from),
        time_text(b.to),
        setpoint_text(b.setpoint)
    );
    if let Some(e) = b.evidence.as_ref().filter(|_| evidence) {
        s.push_str(&format!(" ({} of {} days)", e.agree, e.of));
    }
    s
}

/// Plain-English differences between the approved rules and a proposal.
pub fn diff(old: Option<&Rules>, new: &Rules) -> Vec<String> {
    let modes = [Mode::Heat, Mode::Cool];
    let present = |m: Mode| new.get(m).is_some() || old.is_some_and(|o| o.get(m).is_some());
    let prefixed = modes.iter().filter(|m| present(**m)).count() > 1;
    let mut lines = Vec::new();
    for mode in modes {
        let mut mode_lines = Vec::new();
        match (old.and_then(|o| o.get(mode)), new.get(mode)) {
            (_, Some(n)) => diff_mode(old.and_then(|o| o.get(mode)), n, &mut mode_lines),
            (Some(_), None) => mode_lines.push(format!("Removed: all {} rules", mode_str(mode))),
            (None, None) => {}
        }
        for l in mode_lines {
            lines.push(if prefixed {
                format!("{}: {l}", if mode == Mode::Heat { "Heat" } else { "Cool" })
            } else {
                l
            });
        }
    }
    lines
}

fn diff_mode(old: Option<&ModeRules>, new: &ModeRules, out: &mut Vec<String>) {
    let Some(old) = old else {
        out.push(format!("Base: {}", setpoint_text(new.base)));
        out.extend(
            new.blocks
                .iter()
                .map(|b| format!("New: {}", block_text(b, true))),
        );
        return;
    };
    if setpoints_differ(old.base, new.base) {
        out.push(format!(
            "Base: {} → {}",
            setpoint_text(old.base),
            setpoint_text(new.base)
        ));
    }
    let p = pair(old, new);
    for &(i, j) in &p.pairs {
        let (o, n) = (&old.blocks[i], &new.blocks[j]);
        let days = days_text(&n.days);
        if start_moved(o, n) {
            out.push(format!(
                "Changed: {days} start {} → {}",
                time_text(o.from),
                time_text(n.from)
            ));
        }
        if end_moved(o, n) {
            out.push(format!(
                "Changed: {days} end {} → {}",
                time_text(o.to),
                time_text(n.to)
            ));
        }
        if setpoints_differ(o.setpoint, n.setpoint) {
            out.push(format!(
                "Changed: {days} setpoint {} → {}",
                setpoint_text(o.setpoint),
                setpoint_text(n.setpoint)
            ));
        }
    }
    out.extend(
        p.removed
            .iter()
            .map(|&i| format!("Removed: {}", block_text(&old.blocks[i], false))),
    );
    out.extend(
        p.added
            .iter()
            .map(|&j| format!("New: {}", block_text(&new.blocks[j], true))),
    );
}

// ---------------------------------------------------------------------------------------------
// Fingerprint
// ---------------------------------------------------------------------------------------------

/// FNV-1a 64-bit over the rules' JSON without evidence, as 16 lowercase hex digits.
pub fn fingerprint(rules: &Rules) -> String {
    let mut bare = rules.clone();
    for m in [bare.heat.as_mut(), bare.cool.as_mut()]
        .into_iter()
        .flatten()
    {
        for b in &mut m.blocks {
            b.evidence = None;
        }
    }
    let json = serde_json::to_string(&bare).expect("Rules serialize");
    let hash = json.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, byte| {
        (h ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, 5).unwrap()
    }

    fn at(h: u32, m: u32) -> NaiveDateTime {
        day().and_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn combine_does_not_depend_on_device_order() {
        assert_ne!(0.1 + 0.2 + 0.3, 0.3 + 0.2 + 0.1);
        let temps = |v: [f64; 3]| combine(v.into_iter().map(Slot::Temp));
        let (Slot::Temp(a), Slot::Temp(b)) = (temps([0.1, 0.2, 0.3]), temps([0.3, 0.2, 0.1]))
        else {
            panic!("expected temperatures");
        };
        assert_eq!(a.to_bits(), b.to_bits());
    }

    fn sample(when: NaiveDateTime, target: Option<f64>, mode: &str) -> TargetSample {
        TargetSample {
            at: when,
            target,
            mode: Some(mode.into()),
        }
    }

    fn device(name: &str, samples: Vec<TargetSample>) -> DeviceHistory {
        DeviceHistory {
            entity: name.into(),
            samples,
        }
    }

    fn caps() -> DeviceCaps {
        DeviceCaps {
            hvac_modes: vec![],
            fan_modes: vec![],
            min_temp: None,
            max_temp: None,
            temp_step: None,
        }
    }

    /// The first day's slots for the given devices and engine commands, learning heat.
    fn slots(devices: &[DeviceHistory], engine: &[EngineCommand]) -> [Slot; SLOTS] {
        let caps = caps();
        let input = LearnInput {
            mode: Mode::Heat,
            devices,
            engine,
            approved: None,
            first_day: day(),
            days: 1,
            min_days: 1,
            support: 0.6,
            frost: 7.0,
            caps: &caps,
        };
        timeline(&input).remove(0).1
    }

    const NOON: usize = 12 * 4;

    fn flat(name: &str, mode: &str, t: Option<f64>) -> DeviceHistory {
        device(name, vec![sample(at(0, 0), t, mode)])
    }

    #[test]
    fn two_devices_average() {
        let d = [flat("a", "heat", Some(20.0)), flat("b", "heat", Some(21.0))];
        assert_eq!(slots(&d, &[])[NOON], Slot::Temp(20.5));
    }

    #[test]
    fn off_device_does_not_pull_the_mean() {
        let d = [flat("a", "off", None), flat("b", "heat", Some(20.0))];
        assert_eq!(slots(&d, &[])[NOON], Slot::Temp(20.0));
    }

    #[test]
    fn all_devices_off_is_off() {
        let d = [flat("a", "off", None), flat("b", "off", None)];
        assert_eq!(slots(&d, &[])[NOON], Slot::Off);
    }

    #[test]
    fn other_mode_contributes_nothing() {
        let only = [flat("a", "cool", Some(24.0))];
        assert_eq!(slots(&only, &[])[NOON], Slot::None);
        let both = [flat("a", "cool", Some(24.0)), flat("b", "heat", Some(20.0))];
        assert_eq!(slots(&both, &[])[NOON], Slot::Temp(20.0));
    }

    #[test]
    fn longer_held_value_wins_the_slot() {
        let d = [device(
            "a",
            vec![
                sample(at(0, 0), Some(18.0), "heat"),
                sample(at(12, 10), Some(22.0), "heat"),
            ],
        )];
        let s = slots(&d, &[]);
        assert_eq!(s[NOON], Slot::Temp(18.0));
        assert_eq!(s[NOON + 1], Slot::Temp(22.0));
    }

    #[test]
    fn first_sample_before_the_window_carries_in() {
        let before = at(0, 0) - Duration::hours(2);
        let d = [device("a", vec![sample(before, Some(19.0), "heat")])];
        let s = slots(&d, &[]);
        assert_eq!(s[0], Slot::Temp(19.0));
        assert_eq!(s[SLOTS - 1], Slot::Temp(19.0));
    }

    #[test]
    fn engine_commands_other_than_schedule_and_idle_are_no_data() {
        let d = [flat("a", "heat", Some(16.0))];
        let command = |source| EngineCommand {
            entity: "a".into(),
            at: at(0, 0),
            setpoint: Some(16.0),
            mode: "heat".into(),
            source,
            adjust: None,
        };
        for source in [
            Source::Safety,
            Source::Away,
            Source::Eco,
            Source::Holiday,
            Source::Override,
        ] {
            assert_eq!(
                slots(&d, &[command(source)])[NOON],
                Slot::None,
                "{source:?}"
            );
        }
        for source in [Source::Schedule, Source::Idle] {
            assert_eq!(
                slots(&d, &[command(source)])[NOON],
                Slot::Temp(16.0),
                "{source:?}"
            );
        }
    }

    #[test]
    fn runs_absorb_a_one_slot_spike() {
        let mut v = vec![Slot::Temp(17.0); SLOTS];
        v[40] = Slot::Temp(25.0);
        let runs = runs_of(&v, Mode::Heat);
        assert_eq!(runs.len(), 1);
        assert_eq!((runs[0].start, runs[0].len), (0, SLOTS));
        assert_eq!(runs[0].value, Slot::Temp(17.0));
    }
}
