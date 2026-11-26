//! Pairs a power sensor with the lifetime energy counter (kWh) its card shows, and water with its
//! meter index. A port of the app's `EnergyMeters.kt`.

use std::collections::BTreeMap;

use super::units::{ConsumerIcon, EnergyRole, EnergyUnits, PowerUnits, VolumeUnits};

/// The slice of an entity the energy maths need; built by the caller (the web from `EntityView`).
#[derive(Clone, Debug, PartialEq)]
pub struct EnergyEntity {
    pub id: String,
    pub device_id: Option<String>,
    /// Effective name.
    pub name: String,
    /// Discovery label, for `auto` pairing.
    pub label: Option<String>,
    pub unit: Option<String>,
    /// `StateValue::Number`, else `None`.
    pub value: Option<f64>,
    /// Availability is not `Offline`.
    pub available: bool,
    pub role: Option<EnergyRole>,
    pub invert: bool,
    pub order: Option<i32>,
    pub icon: Option<ConsumerIcon>,
    /// `ui.energy_meter`.
    pub meter: Option<String>,
}

/// What a derived house's total adds (`plus`) and subtracts (`minus`), each counted once.
#[derive(Clone, Debug, PartialEq)]
pub struct HouseMeters<'a> {
    pub plus: Vec<&'a EnergyEntity>,
    pub minus: Vec<&'a EnergyEntity>,
}

const PRODUCED: &str = "energy_produced";

pub struct EnergyMeters;

/// The lifetime counters of what crosses the grid meter: `bought` from the grid, `sold` to it.
/// Ids sorted, each counted once; `factors` holds kWh per counter unit (Wh is 0.001).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GridCounters {
    pub bought: Vec<String>,
    pub sold: Vec<String>,
    pub factors: BTreeMap<String, f64>,
}

impl GridCounters {
    /// kWh per unit of counter `id`; 1 for an unknown id.
    pub fn factor(&self, id: &str) -> f64 {
        self.factors.get(id).copied().unwrap_or(1.0)
    }
}

impl EnergyMeters {
    /// The grid's counters: every `grid` sensor's own counter is bought; its reverse twin and
    /// every `grid_export` sensor's counter are sold.
    pub fn grid_counters(all: &[EnergyEntity]) -> GridCounters {
        let with = |role: EnergyRole| all.iter().filter(move |e| e.role == Some(role));
        let ids = |it: Vec<&EnergyEntity>| {
            let mut v: Vec<String> = it.into_iter().map(|e| e.id.clone()).collect();
            v.sort();
            v.dedup();
            v
        };
        let bought: Vec<&EnergyEntity> = with(EnergyRole::Grid)
            .filter_map(|g| Self::meter_for(g, all))
            .collect();
        let sold: Vec<&EnergyEntity> = with(EnergyRole::Grid)
            .filter_map(|g| Self::reverse_for(g, all))
            .chain(with(EnergyRole::GridExport).filter_map(|x| Self::meter_for(x, all)))
            .collect();
        let factors = bought
            .iter()
            .chain(&sold)
            .filter_map(|e| Some((e.id.clone(), EnergyUnits::factor(e.unit.as_deref())?)))
            .collect();
        GridCounters {
            bought: ids(bought),
            sold: ids(sold),
            factors,
        }
    }

    /// Every entity reporting an energy unit.
    pub fn counters(all: &[EnergyEntity]) -> Vec<&EnergyEntity> {
        all.iter().filter(|e| is_counter(e)).collect()
    }

    /// The counter for `power`: the user's explicit choice, else [`auto`](Self::auto). A dangling
    /// explicit choice pairs nothing.
    pub fn meter_for<'a>(
        power: &EnergyEntity,
        all: &'a [EnergyEntity],
    ) -> Option<&'a EnergyEntity> {
        match &power.meter {
            None => Self::auto(power, all),
            Some(chosen) => all.iter().find(|e| &e.id == chosen && is_counter(e)),
        }
    }

    /// What automatic pairing picks, ignoring the override: a counter on the same device whose
    /// label is `power`'s with "power" read as "energy", else one whose id is (Z2M names `power_a`
    /// and `energy_a` after the device alone, so only `..._power_a_...` <-> `..._energy_a_...`
    /// tells them apart), else the device's only counter when `power` is also its only power
    /// entity.
    pub fn auto<'a>(power: &EnergyEntity, all: &'a [EnergyEntity]) -> Option<&'a EnergyEntity> {
        let device = power.device_id.as_ref()?;
        let on_device: Vec<&EnergyEntity> = all
            .iter()
            .filter(|e| e.device_id.as_ref() == Some(device))
            .collect();
        let counters: Vec<&EnergyEntity> = on_device
            .iter()
            .copied()
            .filter(|e| is_counter(e))
            .collect();
        // A clamp on the inverter line counts the production as "produced"; its plain counter is
        // the standby draw.
        if power.role == Some(EnergyRole::Solar)
            && let Some(t) = twin(power, &counters, PRODUCED)
        {
            return Some(t);
        }
        let wanted = normalize(label_of(power)).replace("power", "energy");
        if let Some(c) = counters.iter().find(|c| normalize(label_of(c)) == wanted) {
            return Some(c);
        }
        if let Some(t) = twin(power, &counters, "energy") {
            return Some(t);
        }
        let powers = on_device
            .iter()
            .filter(|e| PowerUnits::factor(e.unit.as_deref()).is_some())
            .count();
        if counters.len() == 1 && powers == 1 {
            Some(counters[0])
        } else {
            None
        }
    }

    /// Every entity reporting a water volume unit, for the water meter picker.
    pub fn volumes(all: &[EnergyEntity]) -> Vec<&EnergyEntity> {
        all.iter()
            .filter(|e| VolumeUnits::is_volume(e.unit.as_deref()))
            .collect()
    }

    /// The meter index for a `water` sensor: the user's explicit choice, else
    /// [`water_auto`](Self::water_auto). A dangling explicit choice pairs nothing.
    pub fn water_meter_for<'a>(
        water: &EnergyEntity,
        all: &'a [EnergyEntity],
    ) -> Option<&'a EnergyEntity> {
        match &water.meter {
            None => Self::water_auto(water, all),
            Some(chosen) => all.iter().find(|e| {
                &e.id == chosen && e.id != water.id && VolumeUnits::is_volume(e.unit.as_deref())
            }),
        }
    }

    /// What automatic pairing picks for `water`: another sensor on its device in a meter-scale
    /// unit (m³, ft³, CCF), unless `water` is itself in one, being the meter. Same-unit pairs (L
    /// and L, gal and gal) can't be told apart from a day's usage, so they need the explicit
    /// choice. Ties take the smallest id.
    pub fn water_auto<'a>(
        water: &EnergyEntity,
        all: &'a [EnergyEntity],
    ) -> Option<&'a EnergyEntity> {
        let device = water.device_id.as_ref()?;
        if VolumeUnits::is_meter_scale(water.unit.as_deref()) {
            return None;
        }
        all.iter()
            .filter(|e| {
                e.device_id.as_ref() == Some(device)
                    && e.id != water.id
                    && VolumeUnits::is_meter_scale(e.unit.as_deref())
            })
            .min_by(|a, b| a.id.cmp(&b.id))
    }

    /// The counter of the opposite direction on the same device, subtracted from a derived
    /// house: a grid sensor's export (`..._power_a_...` <-> `..._energy_produced_a_...`), or the
    /// inverter's draw when [`meter_for`](Self::meter_for) picked a solar sensor's production twin
    /// (`..._energy_b_...`). `None` for any other role or pairing.
    pub fn reverse_for<'a>(
        power: &EnergyEntity,
        all: &'a [EnergyEntity],
    ) -> Option<&'a EnergyEntity> {
        let device = power.device_id.as_ref()?;
        let counters: Vec<&EnergyEntity> = all
            .iter()
            .filter(|e| e.device_id.as_ref() == Some(device) && is_counter(e))
            .collect();
        match power.role {
            Some(EnergyRole::Grid) => twin(power, &counters, PRODUCED),
            Some(EnergyRole::Solar) => {
                let produced = twin(power, &counters, PRODUCED).map(|e| e.id.as_str());
                let paired = Self::meter_for(power, all).map(|e| e.id.as_str());
                if paired == produced {
                    twin(power, &counters, "energy")
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// The power sensor whose [`meter_for`](Self::meter_for) pairs with `counter`. Scans in
    /// registration order, so the first power wins when more than one would pair to the same
    /// counter. `None` when no power sensor pairs to it.
    pub fn counter_power<'a>(
        counter: &EnergyEntity,
        all: &'a [EnergyEntity],
    ) -> Option<&'a EnergyEntity> {
        all.iter().find(|p| {
            PowerUnits::factor(p.unit.as_deref()).is_some()
                && Self::meter_for(p, all).map(|c| c.id.as_str()) == Some(counter.id.as_str())
        })
    }

    /// A node's lifetime meters: the paired counters of its power `sensors`, each counted once.
    pub fn node_meters<'a>(
        sensors: &[&EnergyEntity],
        all: &'a [EnergyEntity],
    ) -> Vec<&'a EnergyEntity> {
        let paired = sensors
            .iter()
            .filter(|s| s.role.is_some_and(EnergyRole::power))
            .filter_map(|s| Self::meter_for(s, all));
        distinct(paired)
    }

    /// The meters of a house without its own sensor: solar and grid counters, minus grid export
    /// and inverter draw, like its live watts. Every solar and grid sensor needs a counter, and
    /// solar needs an export counter too (else the export would count as consumption); a battery
    /// has no counter pairing yet. `None` when the house has no total.
    pub fn house_meters<'a>(
        by_role: &BTreeMap<EnergyRole, Vec<&'a EnergyEntity>>,
        all: &'a [EnergyEntity],
    ) -> Option<HouseMeters<'a>> {
        let of = |r: EnergyRole| by_role.get(&r).map(Vec::as_slice).unwrap_or(&[]);
        if !of(EnergyRole::Battery).is_empty() {
            return None;
        }
        let solar = of(EnergyRole::Solar);
        let grid = of(EnergyRole::Grid);
        let grid_export = of(EnergyRole::GridExport);
        if solar.is_empty() && grid.is_empty() {
            return None;
        }
        let mut added = Vec::new();
        for s in solar.iter().chain(grid) {
            added.push(Self::meter_for(s, all)?);
        }
        let mut exports: Vec<&EnergyEntity> = grid
            .iter()
            .filter_map(|g| Self::reverse_for(g, all))
            .collect();
        for x in grid_export {
            exports.push(Self::meter_for(x, all)?);
        }
        if !solar.is_empty() && exports.is_empty() {
            return None;
        }
        let draws = solar.iter().filter_map(|s| Self::reverse_for(s, all));
        let minus = distinct(exports.iter().copied().chain(draws));
        Some(HouseMeters {
            plus: distinct(added),
            minus,
        })
    }
}

fn is_counter(e: &EnergyEntity) -> bool {
    EnergyUnits::factor(e.unit.as_deref()).is_some()
}

fn distinct<'a>(it: impl IntoIterator<Item = &'a EnergyEntity>) -> Vec<&'a EnergyEntity> {
    let mut out: Vec<&EnergyEntity> = Vec::new();
    for e in it {
        if !out.iter().any(|o| o.id == e.id) {
            out.push(e);
        }
    }
    out
}

fn label_of(e: &EnergyEntity) -> &str {
    e.label.as_deref().unwrap_or(&e.name)
}

fn normalize(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// The counter whose id is `power`'s with its "power" word replaced, ignoring case.
fn twin<'a>(
    power: &EnergyEntity,
    counters: &[&'a EnergyEntity],
    replacement: &str,
) -> Option<&'a EnergyEntity> {
    let wanted = replace_power_token(&power.id, replacement);
    if wanted == power.id {
        return None;
    }
    let wanted = wanted.to_lowercase();
    counters
        .iter()
        .copied()
        .find(|c| c.id.to_lowercase() == wanted)
}

/// Replaces "power" as a word of an id (`_power_a_`), not inside one (`powers/...`): not
/// preceded by a letter or digit and not followed by a letter, ignoring case.
fn replace_power_token(id: &str, replacement: &str) -> String {
    const TOKEN: &[u8] = b"power";
    let b = id.as_bytes();
    let mut out = String::with_capacity(id.len() + replacement.len());
    let mut last = 0;
    let mut i = 0;
    while i + TOKEN.len() <= b.len() {
        let hit = b[i..i + TOKEN.len()].eq_ignore_ascii_case(TOKEN)
            && (i == 0 || !b[i - 1].is_ascii_alphanumeric())
            && b.get(i + TOKEN.len())
                .is_none_or(|c| !c.is_ascii_alphabetic());
        if hit {
            out.push_str(&id[last..i]);
            out.push_str(replacement);
            i += TOKEN.len();
            last = i;
        } else {
            i += 1;
        }
    }
    out.push_str(&id[last..]);
    out
}

#[cfg(test)]
mod tests;
