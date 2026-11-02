//! The small curve of a sensor's last 24 hours: no axes, just the shape.

use leptos::prelude::*;

use crate::api::series_cache::{cached_day_series, day_series};

/// Room left above and below the line so the stroke is not clipped.
const PAD: f64 = 2.0;

/// The shared extent of some series: first and last time, lowest and highest value. Series under
/// two points draw nothing, so they take no part in it.
fn extent(series: &[&[(i64, f64)]]) -> Option<(i64, i64, f64, f64)> {
    series
        .iter()
        .filter(|s| s.len() >= 2)
        .flat_map(|s| s.iter())
        .fold(None, |acc, &(t, v)| match acc {
            None => Some((t, t, v, v)),
            Some((t_lo, t_hi, v_lo, v_hi)) => {
                Some((t_lo.min(t), t_hi.max(t), v_lo.min(v), v_hi.max(v)))
            }
        })
}

/// The SVG `points` of one series on the scale `(t_lo, t_hi, v_lo, v_hi)`.
fn polyline(
    points: &[(i64, f64)],
    (t_lo, t_hi, v_lo, v_hi): (i64, i64, f64, f64),
    width: f64,
    height: f64,
) -> Option<String> {
    if points.len() < 2 {
        return None;
    }
    let span = (t_hi - t_lo).max(1) as f64;
    let y = |v: f64| {
        if v_hi > v_lo {
            PAD + (v_hi - v) / (v_hi - v_lo) * (height - 2.0 * PAD)
        } else {
            height / 2.0
        }
    };
    let out: Vec<String> = points
        .iter()
        .map(|&(t, v)| {
            let x = (t - t_lo) as f64 / span * width;
            format!("{},{}", round2(x), round2(y(v)))
        })
        .collect();
    Some(out.join(" "))
}

/// The SVG `points` of a series `(time, value)` drawn across `width` × `height`: time left to
/// right, the highest value at the top. `None` under two points; a flat series sits mid-height.
pub fn sparkline_points(points: &[(i64, f64)], width: f64, height: f64) -> Option<String> {
    polyline(points, extent(&[points])?, width, height)
}

/// The `points` of two series drawn on one shared time and value scale, so their heights compare.
/// A series under two points gives `None` and leaves the other's scale alone.
pub fn sparkline_pair_points(
    a: &[(i64, f64)],
    b: &[(i64, f64)],
    width: f64,
    height: f64,
) -> (Option<String>, Option<String>) {
    match extent(&[a, b]) {
        Some(scale) => (
            polyline(a, scale, width, height),
            polyline(b, scale, width, height),
        ),
        None => (None, None),
    }
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

/// The curves of two entities' last 24 hours on one scale: `server` and, when given, `hub`
/// (classes `spark__server` and `spark__hub`). Nothing shows while the hub has no history for
/// `server`; a missing hub series just leaves its line out.
#[component]
pub fn SparklePair(server: String, #[prop(optional)] hub: Option<String>) -> impl IntoView {
    type Points = Option<Vec<(i64, f64)>>;
    let known_server = cached_day_series(&server);
    let known_hub = match &hub {
        Some(id) => cached_day_series(id),
        None => Some(None),
    };
    let known = known_server.is_some() && known_hub.is_some();
    let loaded = RwSignal::new(known);
    let series = RwSignal::new((
        known_server.clone().flatten() as Points,
        known_hub.clone().flatten() as Points,
    ));
    let node = NodeRef::<leptos::html::Span>::new();
    if !known {
        let load = move || {
            leptos::task::spawn_local(async move {
                let server_points = day_series(&server).await;
                let hub_points = match &hub {
                    Some(id) => day_series(id).await,
                    None => None,
                };
                series.try_set((server_points, hub_points));
                loaded.try_set(true);
            });
        };
        when_visible(node, load);
    }
    let lines = Memo::new(move |_| {
        series.with(|(s, h)| {
            sparkline_pair_points(
                s.as_deref().unwrap_or_default(),
                h.as_deref().unwrap_or_default(),
                WIDTH,
                HEIGHT,
            )
        })
    });
    let empty = move || loaded.get() && lines.with(|(s, h)| s.is_none() && h.is_none());
    view! {
        <span class="spark-wrap" class:spark-wrap--empty=empty node_ref=node>
            {move || {
                let (server, hub) = lines.get();
                (server.is_some() || hub.is_some())
                    .then(|| {
                        view! {
                            <svg
                                class="spark"
                                viewBox=format!("0 0 {WIDTH} {HEIGHT}")
                                preserveAspectRatio="none"
                                aria-hidden="true"
                            >
                                {server.map(|p| view! { <polyline class="spark__server" points=p></polyline> })}
                                {hub.map(|p| view! { <polyline class="spark__hub" points=p></polyline> })}
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
    use super::{sparkline_pair_points, sparkline_points};

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

    #[test]
    fn pair_points_share_one_scale() {
        let a: Vec<(i64, f64)> = vec![(0, 50.0), (10, 50.0), (20, 50.0)];
        let b: Vec<(i64, f64)> = vec![(0, 10.0), (10, 10.0), (20, 10.0)];
        let (pa, pb) = sparkline_pair_points(&a, &b, 100.0, 28.0);
        let (pa, pb) = (coords(&pa.unwrap()), coords(&pb.unwrap()));
        assert!(pa[0].1 < pb[0].1, "{pa:?} {pb:?}");
        assert_eq!((pa[0].1, pb[0].1), (2.0, 26.0));
        assert_eq!(pa.last().unwrap().0, 100.0);
    }

    #[test]
    fn pair_points_without_hub_matches_single() {
        let a: Vec<(i64, f64)> = vec![(0, 18.0), (30, 20.0), (60, 22.0)];
        let (pa, pb) = sparkline_pair_points(&a, &[], 100.0, 28.0);
        assert_eq!(pa, sparkline_points(&a, 100.0, 28.0));
        assert_eq!(pb, None);
        assert_eq!(sparkline_pair_points(&[], &[], 100.0, 28.0), (None, None));
    }
}
