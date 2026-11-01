//! The small curve of a sensor's last 24 hours: no axes, just the shape.

use leptos::prelude::*;

use crate::api::series_cache::{cached_day_series, day_series};

/// Room left above and below the line so the stroke is not clipped.
const PAD: f64 = 2.0;

/// The SVG `points` of a series `(time, value)` drawn across `width` × `height`: time left to
/// right, the highest value at the top. `None` under two points; a flat series sits mid-height.
pub fn sparkline_points(points: &[(i64, f64)], width: f64, height: f64) -> Option<String> {
    let (first, last) = (points.first()?, points.last()?);
    if points.len() < 2 {
        return None;
    }
    let span = (last.0 - first.0).max(1) as f64;
    let (lo, hi) = points
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.1), hi.max(p.1))
        });
    let y = |v: f64| {
        if hi > lo {
            PAD + (hi - v) / (hi - lo) * (height - 2.0 * PAD)
        } else {
            height / 2.0
        }
    };
    let out: Vec<String> = points
        .iter()
        .map(|&(t, v)| {
            let x = (t - first.0) as f64 / span * width;
            format!("{},{}", round2(x), round2(y(v)))
        })
        .collect();
    Some(out.join(" "))
}

fn round2(n: f64) -> f64 {
    (n * 100.0).round() / 100.0
}

/// The drawing box, stretched to the space the card gives it.
const WIDTH: f64 = 100.0;
const HEIGHT: f64 = 28.0;

/// The curve of `entity_id`'s last 24 hours, or nothing when the hub has no history for it.
/// The series is fetched the first time the curve comes on screen, through the shared cache.
/// The wrapper keeps its height while loading and takes `spark-wrap--empty` once there is no
/// curve to draw.
#[component]
pub fn Sparkline(entity_id: String) -> impl IntoView {
    let known = cached_day_series(&entity_id);
    let loaded = RwSignal::new(known.is_some());
    let series = RwSignal::new(known.clone().flatten());
    let node = NodeRef::<leptos::html::Span>::new();
    if known.is_none() {
        let load = move || {
            let id = entity_id.clone();
            leptos::task::spawn_local(async move {
                let points = day_series(&id).await;
                series.try_set(points);
                loaded.try_set(true);
            });
        };
        when_visible(node, load);
    }
    let points = Memo::new(move |_| {
        series.with(|s| {
            s.as_deref()
                .and_then(|p| sparkline_points(p, WIDTH, HEIGHT))
        })
    });
    let empty = move || loaded.get() && points.with(Option::is_none);
    view! {
        <span class="spark-wrap" class:spark-wrap--empty=empty node_ref=node>
            {move || {
                points
                    .get()
                    .map(|points| {
                        view! {
                            <svg
                                class="spark"
                                viewBox=format!("0 0 {WIDTH} {HEIGHT}")
                                preserveAspectRatio="none"
                                aria-hidden="true"
                            >
                                <polyline points=points></polyline>
                            </svg>
                        }
                    })
            }}
        </span>
    }
}

/// Runs `f` once, the first time `node` is on screen (at once where the browser cannot tell).
/// The observer goes with the component.
fn when_visible(node: NodeRef<leptos::html::Span>, f: impl FnOnce() + 'static) {
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;
    use web_sys::{IntersectionObserver, IntersectionObserverEntry};

    let f = std::cell::Cell::new(Some(f));
    let held =
        StoredValue::new_local(None::<(IntersectionObserver, Closure<dyn FnMut(js_sys::Array)>)>);
    let fire = std::rc::Rc::new(move || {
        if let Some(f) = f.take() {
            f();
        }
    });
    Effect::new(move |_| {
        let Some(el) = node.get() else { return };
        let on_seen_fire = fire.clone();
        let on_seen = Closure::<dyn FnMut(js_sys::Array)>::new(move |entries: js_sys::Array| {
            let seen = entries.iter().any(|e| {
                e.dyn_into::<IntersectionObserverEntry>()
                    .is_ok_and(|e| e.is_intersecting())
            });
            if seen {
                on_seen_fire();
            }
        });
        match IntersectionObserver::new(on_seen.as_ref().unchecked_ref()) {
            Ok(observer) => {
                observer.observe(&el);
                held.set_value(Some((observer, on_seen)));
            }
            Err(_) => fire(),
        }
    });
    on_cleanup(move || {
        if let Some(Some((observer, _closure))) = held.try_update_value(Option::take) {
            observer.disconnect();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::sparkline_points;

    fn coords(points: &str) -> Vec<(f64, f64)> {
        points
            .split(' ')
            .map(|p| {
                let (x, y) = p.split_once(',').unwrap();
                (x.parse().unwrap(), y.parse().unwrap())
            })
            .collect()
    }

    #[test]
    fn sparkline_points_cases() {
        assert_eq!(sparkline_points(&[(0, 20.0)], 100.0, 28.0), None);
        assert_eq!(sparkline_points(&[], 100.0, 28.0), None);

        let flat =
            coords(&sparkline_points(&[(0, 20.0), (5, 20.0), (10, 20.0)], 100.0, 28.0).unwrap());
        assert!(flat.iter().all(|&(_, y)| y == 14.0), "{flat:?}");
        assert_eq!(flat[0].0, 0.0);
        assert_eq!(flat[2].0, 100.0);
        assert_eq!(flat[1].0, 50.0);

        let rising = coords(&sparkline_points(&[(0, 18.0), (60, 22.0)], 100.0, 28.0).unwrap());
        assert_eq!(rising, vec![(0.0, 26.0), (100.0, 2.0)]);
    }
}
