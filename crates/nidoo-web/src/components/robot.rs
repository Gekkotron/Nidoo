//! The robot vacuum drawn from above: rings around a round body with its top bump and one side
//! brush at the front right. Generic on purpose, no brand. The activity, as a class, lets the
//! stylesheet animate or tint it: while cleaning the brush spins and dust is drawn in.

use leptos::prelude::*;

/// The dust in front of the robot while it cleans: `(cx, cy, r, dx, dy, delay)` in the drawing's
/// units, `delay` in seconds. Each speck heads for the centre and fades out where it meets the
/// body or the brush's reach, so none is ever drawn over them; the delays keep the stream going.
const DUST: [(f32, f32, f32, f32, f32, f32); 22] = [
    (34.0, 20.0, 1.6, 11.6, 15.4, 0.19),
    (50.0, 4.0, 1.3, 8.6, 25.0, 1.19),
    (17.0, 1.0, 1.6, 27.8, 35.1, 1.2),
    (91.0, 8.0, 1.6, -4.6, 19.5, 1.28),
    (72.0, 11.0, 1.3, 0.7, 14.9, 1.42),
    (52.0, 9.0, 1.9, 6.8, 19.5, 0.06),
    (87.0, 4.0, 1.3, -3.7, 22.4, 1.69),
    (64.0, 3.0, 1.6, 3.7, 23.7, 1.56),
    (46.0, 3.0, 1.9, 10.7, 26.2, 0.03),
    (27.0, 28.0, 1.3, 13.0, 12.6, 0.12),
    (35.0, 29.0, 1.6, 7.9, 9.0, 0.84),
    (62.0, 16.0, 1.6, 2.6, 11.4, 0.0),
    (11.0, 7.0, 1.3, 30.5, 32.0, 0.08),
    (28.0, 18.0, 1.3, 15.6, 19.3, 1.55),
    (54.0, 12.0, 1.3, 5.5, 16.6, 0.15),
    (37.0, 25.0, 1.3, 8.1, 10.6, 1.2),
    (112.0, 3.0, 2.2, -12.5, 23.9, 0.28),
    (132.0, 21.0, 1.6, -13.1, 12.3, 0.29),
    (117.0, 16.0, 2.2, -7.6, 10.6, 1.05),
    (146.0, 10.0, 1.6, -26.6, 24.3, 0.66),
    (148.0, 25.0, 1.9, -26.5, 18.0, 0.27),
    (126.0, 3.0, 2.2, -17.3, 24.5, 1.3),
];

#[component]
pub fn Robot(activity: Option<String>) -> impl IntoView {
    let cleaning = activity.as_deref() == Some("cleaning");
    let class = format!("robot robot--{}", activity.as_deref().unwrap_or("unknown"));
    let dust = cleaning.then(|| {
        DUST.iter()
            .map(|&(cx, cy, r, dx, dy, delay)| {
                let style = format!("--dx:{dx}px;--dy:{dy}px;animation-delay:-{delay}s");
                view! { <circle class="robot__dust" cx=cx cy=cy r=r style=style></circle> }
            })
            .collect_view()
    });
    view! {
        <svg class=class viewBox="0 0 150 150" aria-hidden="true">
            <circle class="robot__ring robot__ring--outer" cx="75" cy="75" r="72"></circle>
            <circle class="robot__ring" cx="75" cy="75" r="60"></circle>
            {dust}
            <circle class="robot__body" cx="75" cy="75" r="48"></circle>
            <circle class="robot__top" cx="75" cy="64" r="15"></circle>
            <rect class="robot__sensor" x="71" y="35" width="8" height="2" rx="1"></rect>
            <g class="robot__brush" transform="translate(106 42)">
                <g class="robot__brush-spin">
                    // Invisible, so the spin turns around the hub: it centres the box.
                    <circle class="robot__brush-reach" r="14"></circle>
                    <path
                        class="robot__brush-arms"
                        d="M0 0V-13M0 0L11.3 6.5M0 0L-11.3 6.5M-1.8 -13L1.8 -13M10.4 8.1L12.2 5M-12.2 5L-10.4 8.1"
                    ></path>
                    <circle class="robot__brush-hub" r="2.8"></circle>
                </g>
            </g>
        </svg>
    }
}
