//! The three sensor checks (stuck, silent, impossible) and the learned baseline they use.
//!
//! All times are epoch seconds. Nothing here reads a clock or a database: the hub hands in
//! plain readings and gets findings back.

use serde::{Deserialize, Serialize};

const HOUR: u64 = 3600;
const DAY: u64 = 24 * HOUR;

/// History a sensor needs before it gets a baseline.
const MIN_HISTORY: u64 = 7 * DAY;
/// Changes in the window a sensor needs before it gets a baseline.
const MIN_CHANGES: u32 = 20;
/// Floor of the stuck and silent thresholds.
const MIN_THRESHOLD: u64 = 6 * HOUR;
/// How long an entity may be unavailable before it counts as silent.
const OFFLINE_LIMIT: u64 = HOUR;

/// What looks wrong with a sensor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    /// Still reporting, the value never changes.
    Stuck,
    /// Stopped reporting, or unavailable.
    Silent,
    /// The value is physically out of range.
    Impossible,
}

impl Check {
    /// The wire name (`stuck`, `silent`, `impossible`).
    pub fn as_str(self) -> &'static str {
        match self {
            Check::Stuck => "stuck",
            Check::Silent => "silent",
            Check::Impossible => "impossible",
        }
    }
}

/// How quickly the stuck and silent checks fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    Low,
    #[default]
    Normal,
    High,
}

impl Sensitivity {
    /// The multiplier applied to a sensor's longest usual gap.
    pub fn factor(self) -> f64 {
        match self {
            Sensitivity::Low => 3.0,
            Sensitivity::Normal => 2.0,
            Sensitivity::High => 1.5,
        }
    }
}

/// A sensor's learned normal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Baseline {
    /// Longest gap between two changes in the window, seconds.
    pub max_gap: u64,
    /// Changes in the window.
    pub changes: u32,
}

/// One sensor's change statistics over the baseline window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GapStats {
    pub entity_id: String,
    /// Changes recorded in the window.
    pub changes: u32,
    /// Time of the first recorded change in the window.
    pub first: u64,
    /// The largest gaps between consecutive changes, as (start, end), at most five.
    pub top_gaps: Vec<(u64, u64)>,
}

/// The sensor's baseline, or `None` while it has less than 7 days of history or fewer than
/// 20 changes. Gaps overlapping an `excluded` interval (a recorded problem of that sensor)
/// are left out; when all of them overlap, the smallest one is used.
pub fn baseline(stats: &GapStats, now: u64, excluded: &[(u64, u64)]) -> Option<Baseline> {
    if now.saturating_sub(stats.first) < MIN_HISTORY || stats.changes < MIN_CHANGES {
        return None;
    }
    let len = |&(start, end): &(u64, u64)| end.saturating_sub(start);
    let overlaps = |gap: &(u64, u64)| excluded.iter().any(|ex| gap.0 < ex.1 && ex.0 < gap.1);
    let max_gap = stats
        .top_gaps
        .iter()
        .filter(|g| !overlaps(g))
        .map(len)
        .max()
        .or_else(|| stats.top_gaps.iter().map(len).min())?;
    Some(Baseline {
        max_gap,
        changes: stats.changes,
    })
}

/// The physically possible range for a `device_class` in its expected unit, or `None` when the
/// class (or unit) is not range-checked. An unbounded side is infinite.
pub fn physical_range(device_class: Option<&str>, unit: Option<&str>) -> Option<(f64, f64)> {
    let (range, expected_unit) = match device_class? {
        "temperature" => ((-50.0, 150.0), Some("°C")),
        "humidity" | "battery" | "moisture" => ((0.0, 100.0), Some("%")),
        "illuminance" => ((0.0, f64::INFINITY), None),
        "pressure" => ((800.0, 1200.0), Some("hPa")),
        "voltage" => ((0.0, 300.0), Some("V")),
        "co2" => ((0.0, 10_000.0), Some("ppm")),
        _ => return None,
    };
    match expected_unit {
        Some(expected) if unit != Some(expected) => None,
        _ => Some(range),
    }
}

/// What the hub knows about one supervised sensor at check time.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub entity_id: String,
    pub name: String,
    pub device_class: Option<String>,
    pub unit: Option<String>,
    /// Current numeric value, if any.
    pub value: Option<f64>,
    /// When the value last changed (history).
    pub last_change: Option<u64>,
    /// When the hub last read a value for it (live).
    pub last_report: Option<u64>,
    /// Since when it has been unavailable, if it is.
    pub unavailable_since: Option<u64>,
    pub baseline: Option<Baseline>,
}

/// The run's context.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ctx {
    pub now: u64,
    /// When the report clocks started (hub start or broker link back); no clock is older.
    pub armed_at: u64,
    /// Sensitivity factor.
    pub factor: f64,
    /// The broker link or the Zigbee bridge is down: the silent check neither opens nor closes.
    pub silent_paused: bool,
}

/// What a check saw.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub value: Option<f64>,
    pub unit: Option<String>,
    /// How long the condition has lasted, seconds.
    pub for_secs: u64,
    /// The sensor's usual longest gap, when the check used the baseline.
    pub usual_secs: Option<u64>,
}

/// One check that fired.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub check: Check,
    pub evidence: Evidence,
}

/// `max(max_gap × factor, 6 h)`.
fn threshold(b: &Baseline, factor: f64) -> u64 {
    ((b.max_gap as f64 * factor) as u64).max(MIN_THRESHOLD)
}

/// The checks that fire for `r`.
pub fn evaluate(r: &Reading, ctx: &Ctx) -> Vec<Finding> {
    let now = ctx.now;
    let evidence = |for_secs: u64, usual_secs: Option<u64>| Evidence {
        value: r.value,
        unit: r.unit.clone(),
        for_secs,
        usual_secs,
    };
    let mut found = Vec::new();

    if let (Some(b), Some(changed), Some(report)) = (&r.baseline, r.last_change, r.last_report) {
        let flat_for = now.saturating_sub(changed);
        let recent = now.saturating_sub(report) <= b.max_gap.max(HOUR);
        if flat_for > threshold(b, ctx.factor) && recent {
            found.push(Finding {
                check: Check::Stuck,
                evidence: evidence(flat_for, Some(b.max_gap)),
            });
        }
    }

    if !ctx.silent_paused {
        let offline_for = r.unavailable_since.map(|since| now.saturating_sub(since));
        if let Some(offline_for) = offline_for.filter(|&d| d > OFFLINE_LIMIT) {
            found.push(Finding {
                check: Check::Silent,
                evidence: evidence(offline_for, None),
            });
        } else if let Some(b) = &r.baseline {
            let clock = r.last_report.unwrap_or(ctx.armed_at).max(ctx.armed_at);
            let quiet_for = now.saturating_sub(clock);
            if quiet_for > threshold(b, ctx.factor) {
                found.push(Finding {
                    check: Check::Silent,
                    evidence: evidence(quiet_for, Some(b.max_gap)),
                });
            }
        }
    }

    let range = physical_range(r.device_class.as_deref(), r.unit.as_deref());
    if let (Some((lo, hi)), Some(v)) = (range, r.value)
        && !(lo..=hi).contains(&v)
    {
        let since = r.last_change.map_or(0, |c| now.saturating_sub(c));
        found.push(Finding {
            check: Check::Impossible,
            evidence: evidence(since, None),
        });
    }

    found
}
