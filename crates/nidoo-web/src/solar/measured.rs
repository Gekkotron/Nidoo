//! What the house measured of today's production, shared with the solar card and page.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use leptos::prelude::*;
use leptos::task::spawn_local;
use nidoo_api::{EnergyKind, HistoryRange};

use super::model::{SolarSources, day_bounds, local_date, measured_today, resample, solar_sources};
use crate::api::history::{fetch_series, fetch_today};
use crate::components::now::unix_now;
use crate::energy::energy_entities;
use crate::house::House;
use crate::screens::energy::KwhToday;
use crate::screens::flow_canvas::sun::house_place;

/// Measured production today, provided as context by whoever loads it.
#[derive(Clone, Copy)]
pub struct SolarMeasured {
    /// kWh since local midnight; `None` while unknown (never a made-up zero).
    pub today_kwh: RwSignal<Option<f64>>,
    /// `(period end, kW)` of the measured curve.
    pub periods: RwSignal<Vec<(u64, f64)>>,
}

/// Counts refreshes so a slow reply from an older one never overwrites a newer one.
#[derive(Clone, Default)]
pub(crate) struct Generation(Rc<Cell<u64>>);

impl Generation {
    /// Starts a refresh and returns its number.
    pub(crate) fn next(&self) -> u64 {
        let g = self.0.get() + 1;
        self.0.set(g);
        g
    }

    /// Whether `g` is still the latest refresh.
    pub(crate) fn is_current(&self, g: u64) -> bool {
        self.0.get() == g
    }
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const REFRESH_INTERVAL_MS: u32 = 300_000;

/// What the last refresh learned, kept so a change of `KwhToday` can recompute today's kWh
/// without fetching again.
#[derive(Default)]
struct Cache {
    sources: SolarSources,
    power_kwh: BTreeMap<String, Option<f64>>,
}

/// Provides [`SolarMeasured`] and keeps it fresh: when the entity map first fills and the
/// forecast is known, on every change of the forecast, every 5 min while the tab is visible and
/// when it becomes visible again. Call once from `App`, after `install_history_refresh`, so the
/// `KwhToday` context exists.
pub fn install_solar_measured(house: House) {
    let measured = SolarMeasured {
        today_kwh: RwSignal::new(None),
        periods: RwSignal::new(Vec::new()),
    };
    provide_context(measured);
    let kwh_today = use_context::<KwhToday>();
    let generation = Generation::default();
    let cache = Rc::new(RefCell::new(Cache::default()));

    let counter_kwh = move || kwh_today.map(|k| k.0.get_untracked()).unwrap_or_default();

    let fire: Rc<dyn Fn()> = {
        let cache = cache.clone();
        Rc::new(move || {
            let gen_id = generation.next();
            let all = house.entities.with_untracked(energy_entities);
            let sources = solar_sources(&all);
            if sources.counters.is_empty() && sources.powers.is_empty() {
                *cache.borrow_mut() = Cache::default();
                measured.today_kwh.set(None);
                measured.periods.set(Vec::new());
                return;
            }
            let periods = house
                .solar
                .with_untracked(|s| s.as_ref().map(|s| s.periods.clone()))
                .unwrap_or_default();
            let zone = house
                .flows_config
                .with_untracked(|c| house_place(c.as_ref()).1);
            let cache = cache.clone();
            let generation = generation.clone();
            spawn_local(async move {
                let now = unix_now();
                let from = day_bounds(zone, local_date(zone, now)).0;
                let mut power_kwh = BTreeMap::new();
                if sources.counters.is_empty() {
                    for p in &sources.powers {
                        let v = fetch_today(&p.id, EnergyKind::Power)
                            .await
                            .ok()
                            .and_then(|t| t.kwh);
                        power_kwh.insert(p.id.clone(), v);
                    }
                }
                let mut series = Vec::new();
                for p in &sources.powers {
                    // A failed source is left out of the curve, never an error toast.
                    if let Ok(s) = fetch_series(&p.id, HistoryRange::Day).await {
                        series.push((p.clone(), s));
                    }
                }
                if !generation.is_current(gen_id) {
                    return;
                }
                let today = measured_today(&sources, &counter_kwh(), &power_kwh);
                let curve = resample(&series, &periods, from, now);
                *cache.borrow_mut() = Cache { sources, power_kwh };
                measured.today_kwh.set(today);
                measured.periods.set(curve);
            });
        })
    };

    let has_entities = Memo::new(move |_| house.entities.with(|m| !m.is_empty()));
    let fire_effect = fire.clone();
    Effect::new(move |_| {
        let has = has_entities.get();
        let has_solar = house.solar.with(Option::is_some);
        if has && has_solar {
            fire_effect();
        }
    });

    // The Energy page's counters refresh on their own; follow them without another fetch.
    if let Some(k) = kwh_today {
        let cache = cache.clone();
        Effect::new(move |_| {
            let map = k.0.get();
            let c = cache.borrow();
            if c.sources.counters.is_empty() && c.sources.powers.is_empty() {
                return;
            }
            measured
                .today_kwh
                .set(measured_today(&c.sources, &map, &c.power_kwh));
        });
    }

    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        fn is_hidden() -> bool {
            web_sys::window()
                .and_then(|w| w.document())
                .is_some_and(|d| d.hidden())
        }
        let fire_tick = fire.clone();
        let interval = gloo_timers::callback::Interval::new(REFRESH_INTERVAL_MS, move || {
            if !is_hidden() && house.solar.with_untracked(Option::is_some) {
                fire_tick();
            }
        });
        interval.forget();

        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            let fire_vis = fire;
            let closure = wasm_bindgen::closure::Closure::wrap(Box::new(move || {
                if !is_hidden() && house.solar.with_untracked(Option::is_some) {
                    fire_vis();
                }
            }) as Box<dyn Fn()>);
            let _ = doc.add_event_listener_with_callback(
                "visibilitychange",
                closure.as_ref().unchecked_ref(),
            );
            closure.forget();
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = fire;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_generation_is_dropped() {
        let g = Generation::default();
        let a = g.next();
        let b = g.next();
        assert!(!g.is_current(a));
        assert!(g.is_current(b));
    }
}
