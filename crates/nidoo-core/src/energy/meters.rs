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
mod tests {
    use super::super::testkit::e;
    use super::*;

    const DEV_G: &str = "zigbee2mqtt_0xa4c13898428fae3a";
    const DEV_C: &str = "zigbee2mqtt_0xa4c13870d8ba6a60";

    /// One PJ-1203A clamp: power_a/power_b plus the four counters, ids as Z2M publishes them.
    fn clamp(hex: &str, device: &str) -> Vec<EnergyEntity> {
        let id = |kind: &str| format!("{hex}_{kind}_zigbee2mqtt");
        let mk = |kind: &str, unit: &str| EnergyEntity {
            device_id: Some(device.to_string()),
            label: Some(kind.to_string()),
            unit: Some(unit.to_string()),
            ..e(&id(kind))
        };
        vec![
            mk("power_a", "W"),
            mk("power_b", "W"),
            mk("energy_a", "kWh"),
            mk("energy_b", "kWh"),
            mk("energy_produced_a", "kWh"),
            mk("energy_produced_b", "kWh"),
        ]
    }

    fn house() -> Vec<EnergyEntity> {
        let mut all = clamp("0xa4c13898428fae3a", DEV_G);
        all[0].role = Some(EnergyRole::Grid);
        all[1].role = Some(EnergyRole::Solar);
        all.extend(clamp("0xa4c13870d8ba6a60", DEV_C));
        all[6].role = Some(EnergyRole::Consumer);
        all[7].role = Some(EnergyRole::Consumer);
        all
    }

    fn id(x: Option<&EnergyEntity>) -> Option<&str> {
        x.map(|e| e.id.as_str())
    }

    fn by_id<'a>(all: &'a [EnergyEntity], id: &str) -> &'a EnergyEntity {
        all.iter().find(|e| e.id == id).unwrap()
    }

    #[test]
    fn grid_counters_sum_phases_once() {
        let mut all = clamp("0x1111", "dev_one");
        all[0].role = Some(EnergyRole::Grid);
        let mut two = clamp("0x2222", "dev_two");
        two[0].role = Some(EnergyRole::Grid);
        // An export sensor pointed at a counter the grid already counts as sold.
        two[1].role = Some(EnergyRole::GridExport);
        two[1].meter = Some("0x2222_energy_produced_a_zigbee2mqtt".to_string());
        all.extend(two);
        let c = EnergyMeters::grid_counters(&all);
        assert_eq!(
            c.bought,
            vec!["0x1111_energy_a_zigbee2mqtt", "0x2222_energy_a_zigbee2mqtt"]
        );
        assert_eq!(
            c.sold,
            vec![
                "0x1111_energy_produced_a_zigbee2mqtt",
                "0x2222_energy_produced_a_zigbee2mqtt"
            ]
        );
    }

    #[test]
    fn grid_counters_carry_unit_factor() {
        let mut all = clamp("0x1111", "dev_one");
        all[0].role = Some(EnergyRole::Grid);
        all[2].unit = Some("Wh".to_string()); // energy_a
        let c = EnergyMeters::grid_counters(&all);
        assert_eq!(c.factor("0x1111_energy_a_zigbee2mqtt"), 0.001);
        assert_eq!(c.factor("0x1111_energy_produced_a_zigbee2mqtt"), 1.0);
    }

    #[test]
    fn grid_counters_without_export() {
        let mut all: Vec<EnergyEntity> = clamp("0x1111", "dev_one")
            .into_iter()
            .filter(|e| !e.id.contains("produced"))
            .collect();
        all[0].role = Some(EnergyRole::Grid);
        let c = EnergyMeters::grid_counters(&all);
        assert_eq!(c.bought, vec!["0x1111_energy_a_zigbee2mqtt"]);
        assert!(c.sold.is_empty());
        assert_eq!(EnergyMeters::grid_counters(&[]), GridCounters::default());
    }

    #[test]
    fn grid_pairs_with_energy_a() {
        let all = house();
        let grid = &all[0];
        assert_eq!(
            id(EnergyMeters::meter_for(grid, &all)),
            Some("0xa4c13898428fae3a_energy_a_zigbee2mqtt")
        );
    }

    #[test]
    fn consumers_on_the_other_device_pair_by_label() {
        let all = house();
        assert_eq!(
            id(EnergyMeters::meter_for(&all[6], &all)),
            Some("0xa4c13870d8ba6a60_energy_a_zigbee2mqtt")
        );
        assert_eq!(
            id(EnergyMeters::meter_for(&all[7], &all)),
            Some("0xa4c13870d8ba6a60_energy_b_zigbee2mqtt")
        );
    }

    #[test]
    fn solar_pairs_with_its_production_twin_and_draws_from_energy_b() {
        let all = house();
        let solar = &all[1];
        assert_eq!(
            id(EnergyMeters::meter_for(solar, &all)),
            Some("0xa4c13898428fae3a_energy_produced_b_zigbee2mqtt")
        );
        assert_eq!(
            id(EnergyMeters::reverse_for(solar, &all)),
            Some("0xa4c13898428fae3a_energy_b_zigbee2mqtt")
        );
    }

    #[test]
    fn grid_reverse_is_its_produced_twin() {
        let all = house();
        assert_eq!(
            id(EnergyMeters::reverse_for(&all[0], &all)),
            Some("0xa4c13898428fae3a_energy_produced_a_zigbee2mqtt")
        );
        assert_eq!(EnergyMeters::reverse_for(&all[6], &all), None);
    }

    #[test]
    fn solar_reverse_needs_the_production_pairing() {
        let mut all = house();
        all[1].meter = Some("0xa4c13898428fae3a_energy_a_zigbee2mqtt".into());
        let solar = all[1].clone();
        assert_eq!(EnergyMeters::reverse_for(&solar, &all), None);
    }

    #[test]
    fn explicit_meter_wins() {
        let mut all = house();
        all[0].meter = Some("0xa4c13870d8ba6a60_energy_b_zigbee2mqtt".into());
        let grid = all[0].clone();
        assert_eq!(
            id(EnergyMeters::meter_for(&grid, &all)),
            Some("0xa4c13870d8ba6a60_energy_b_zigbee2mqtt")
        );
        assert_eq!(
            id(EnergyMeters::auto(&grid, &all)),
            Some("0xa4c13898428fae3a_energy_a_zigbee2mqtt")
        );
    }

    #[test]
    fn dangling_or_non_counter_explicit_meter_pairs_nothing() {
        let mut all = house();
        all[0].meter = Some("gone".into());
        let grid = all[0].clone();
        assert_eq!(EnergyMeters::meter_for(&grid, &all), None);
        let mut power_pointing_at_power = grid.clone();
        power_pointing_at_power.meter = Some(all[1].id.clone());
        assert_eq!(
            EnergyMeters::meter_for(&power_pointing_at_power, &all),
            None
        );
    }

    #[test]
    fn lone_power_and_lone_counter_pair_whatever_their_labels() {
        let all = vec![
            EnergyEntity {
                device_id: Some("d".into()),
                ..e("p")
            },
            EnergyEntity {
                device_id: Some("d".into()),
                unit: Some("kWh".into()),
                ..e("c")
            },
        ];
        assert_eq!(id(EnergyMeters::meter_for(&all[0], &all)), Some("c"));
    }

    #[test]
    fn two_unrelated_powers_pair_nothing_and_other_devices_are_ignored() {
        let mk = |i: &str, unit: &str, dev: &str| EnergyEntity {
            device_id: Some(dev.into()),
            unit: Some(unit.into()),
            ..e(i)
        };
        let all = vec![
            mk("a", "W", "d"),
            mk("b", "W", "d"),
            mk("c1", "kWh", "d"),
            mk("c2", "kWh", "d"),
            mk("far", "kWh", "x"),
        ];
        assert_eq!(EnergyMeters::meter_for(&all[0], &all), None);
        let no_device = EnergyEntity {
            device_id: None,
            ..e("n")
        };
        assert_eq!(EnergyMeters::meter_for(&no_device, &all), None);
    }

    #[test]
    fn label_beats_name() {
        let mk = |i: &str, name: &str, label: Option<&str>, unit: &str| EnergyEntity {
            device_id: Some("d".into()),
            name: name.into(),
            label: label.map(String::from),
            unit: Some(unit.into()),
            ..e(i)
        };
        let all = vec![
            mk("p", "Plug Power", Some("Power"), "W"),
            mk("e", "Plug Energy", Some("Energy"), "kWh"),
            mk("x", "Other", None, "kWh"),
        ];
        assert_eq!(id(EnergyMeters::meter_for(&all[0], &all)), Some("e"));
    }

    #[test]
    fn twin_token_is_a_word_not_a_substring() {
        assert_eq!(replace_power_token("a_power_b", "energy"), "a_energy_b");
        assert_eq!(replace_power_token("Power_a", "x"), "x_a");
        assert_eq!(replace_power_token("powers_a", "x"), "powers_a");
        assert_eq!(replace_power_token("mypower_a", "x"), "mypower_a");
        assert_eq!(replace_power_token("p_power", "x"), "p_x");
        assert_eq!(replace_power_token("power2_power", "x"), "x2_x");
    }

    #[test]
    fn counter_power_pairs_back() {
        let all = house();
        let counter = by_id(&all, "0xa4c13898428fae3a_energy_a_zigbee2mqtt");
        assert_eq!(
            id(EnergyMeters::counter_power(counter, &all)),
            Some("0xa4c13898428fae3a_power_a_zigbee2mqtt")
        );
    }

    #[test]
    fn counter_power_is_none_when_no_power_pairs() {
        let all = vec![EnergyEntity {
            unit: Some("kWh".into()),
            ..e("c")
        }];
        assert_eq!(EnergyMeters::counter_power(&all[0], &all), None);
    }

    #[test]
    fn counter_power_first_registered_wins_on_ties() {
        let mk = |i: &str, unit: &str, dev: &str| EnergyEntity {
            device_id: Some(dev.into()),
            unit: Some(unit.into()),
            ..e(i)
        };
        let all = vec![mk("p1", "W", "d"), mk("p2", "W", "d"), mk("c", "kWh", "d")];
        // Two powers + one counter: auto-pairing refuses the ambiguity, so neither power maps to c
        // and counter_power(c) is None.
        assert_eq!(EnergyMeters::counter_power(&all[2], &all), None);

        // Labelled match for p1 only; counter_power must then find p1 (first-registered wins).
        let mut all = all;
        all[0].label = Some("Power".into());
        all[2].label = Some("Energy".into());
        assert_eq!(id(EnergyMeters::counter_power(&all[2], &all)), Some("p1"));
    }

    fn water(id: &str, unit: &str, device: Option<&str>) -> EnergyEntity {
        EnergyEntity {
            device_id: device.map(String::from),
            unit: Some(unit.into()),
            role: Some(EnergyRole::Water),
            ..e(id)
        }
    }

    #[test]
    fn water_auto_pairs_a_meter_scale_sensor_on_its_device() {
        for unit in ["m³", "m3", "ft³", "CCF"] {
            let all = vec![
                water("daily", "L", Some("d")),
                water("index", unit, Some("d")),
                water("far", unit, Some("x")),
            ];
            assert_eq!(
                id(EnergyMeters::water_meter_for(&all[0], &all)),
                Some("index")
            );
        }
    }

    #[test]
    fn water_meter_pairs_nothing_for_meters_same_units_or_no_device() {
        let index = water("index", "m³", Some("d"));
        let all = vec![index.clone(), water("other", "m³", Some("d"))];
        assert_eq!(EnergyMeters::water_meter_for(&index, &all), None);
        let gal = water("daily", "gal", Some("d"));
        let all = vec![gal.clone(), water("total", "gal", Some("d"))];
        assert_eq!(EnergyMeters::water_meter_for(&gal, &all), None);
        let loose = water("daily", "L", None);
        let all = vec![loose.clone(), water("index", "m³", None)];
        assert_eq!(EnergyMeters::water_meter_for(&loose, &all), None);
    }

    #[test]
    fn water_auto_takes_the_smallest_id_among_meters() {
        let all = vec![
            water("daily", "L", Some("d")),
            water("b_index", "m³", Some("d")),
            water("a_index", "m³", Some("d")),
        ];
        assert_eq!(id(EnergyMeters::water_auto(&all[0], &all)), Some("a_index"));
    }

    #[test]
    fn water_explicit_choice_takes_any_volume_but_not_itself_or_a_non_volume() {
        let mut w = water("daily", "gal", None);
        w.meter = Some("utility".into());
        let all = vec![w.clone(), water("utility", "gal", None)];
        assert_eq!(id(EnergyMeters::water_meter_for(&w, &all)), Some("utility"));
        let mut own = water("daily", "L", Some("d"));
        own.meter = Some("daily".into());
        assert_eq!(
            EnergyMeters::water_meter_for(&own, std::slice::from_ref(&own)),
            None
        );
        let mut kwh = water("daily", "L", Some("d"));
        kwh.meter = Some("e".into());
        let all = vec![
            kwh.clone(),
            EnergyEntity {
                unit: Some("kWh".into()),
                ..e("e")
            },
        ];
        assert_eq!(EnergyMeters::water_meter_for(&kwh, &all), None);
    }

    #[test]
    fn counters_and_volumes_filter_by_unit() {
        let all = house();
        assert_eq!(EnergyMeters::counters(&all).len(), 8);
        assert!(EnergyMeters::volumes(&all).is_empty());
        let w = vec![water("v", "L", None), water("m", "m³", None), e("p")];
        assert_eq!(EnergyMeters::volumes(&w).len(), 2);
    }

    #[test]
    fn node_meters_count_each_counter_once_and_skip_water() {
        let all = house();
        let sensors = vec![&all[6], &all[6], &all[7]];
        let ids: Vec<&str> = EnergyMeters::node_meters(&sensors, &all)
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(
            ids,
            [
                "0xa4c13870d8ba6a60_energy_a_zigbee2mqtt",
                "0xa4c13870d8ba6a60_energy_b_zigbee2mqtt"
            ]
        );
        let w = vec![water("v", "L", Some("d")), water("m", "m³", Some("d"))];
        assert!(EnergyMeters::node_meters(&[&w[0]], &w).is_empty());
    }

    fn roles(all: &[EnergyEntity]) -> BTreeMap<EnergyRole, Vec<&EnergyEntity>> {
        let mut m: BTreeMap<EnergyRole, Vec<&EnergyEntity>> = BTreeMap::new();
        for x in all {
            if let Some(r) = x.role {
                m.entry(r).or_default().push(x);
            }
        }
        m
    }

    fn solar_grid() -> Vec<EnergyEntity> {
        let mut all = clamp("0xa4c13898428fae3a", DEV_G);
        all[0].role = Some(EnergyRole::Grid);
        all[1].role = Some(EnergyRole::Solar);
        all
    }

    #[test]
    fn house_meters_without_a_house() {
        let all = solar_grid();
        let m = EnergyMeters::house_meters(&roles(&all), &all).unwrap();
        let ids = |v: &[&EnergyEntity]| v.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
        // solar first, then grid, as the app lists `solar + grid`.
        assert_eq!(
            ids(&m.plus),
            [
                "0xa4c13898428fae3a_energy_produced_b_zigbee2mqtt",
                "0xa4c13898428fae3a_energy_a_zigbee2mqtt"
            ]
        );
        assert_eq!(
            ids(&m.minus),
            [
                "0xa4c13898428fae3a_energy_produced_a_zigbee2mqtt",
                "0xa4c13898428fae3a_energy_b_zigbee2mqtt"
            ]
        );
    }

    #[test]
    fn house_meters_give_no_total_with_a_battery_no_export_or_nothing() {
        let all = solar_grid();
        let mut with_battery = roles(&all);
        let bat = EnergyEntity {
            role: Some(EnergyRole::Battery),
            ..e("bat")
        };
        with_battery.insert(EnergyRole::Battery, vec![&bat]);
        assert_eq!(EnergyMeters::house_meters(&with_battery, &all), None);

        let no_export: Vec<EnergyEntity> = all
            .iter()
            .filter(|x| !x.id.contains("produced_a"))
            .cloned()
            .collect();
        assert_eq!(
            EnergyMeters::house_meters(&roles(&no_export), &no_export),
            None
        );

        let only_consumers = house();
        let mut m = BTreeMap::new();
        m.insert(
            EnergyRole::Consumer,
            vec![by_id(&only_consumers, &only_consumers[6].id)],
        );
        assert_eq!(EnergyMeters::house_meters(&m, &only_consumers), None);
    }

    #[test]
    fn house_meters_need_a_counter_for_every_input() {
        let mut all = solar_grid();
        all.retain(|x| x.id != "0xa4c13898428fae3a_energy_a_zigbee2mqtt");
        assert_eq!(EnergyMeters::house_meters(&roles(&all), &all), None);
    }
}
