//! "How it works": what a mood is and what its categories include.

use leptos::prelude::*;
use leptos_router::components::A;

use crate::i18n::{Key, use_text};
use crate::nav::BackLink;

#[component]
pub fn AmbiancesAboutScreen() -> impl IntoView {
    let t = use_text();
    view! {
        <div class="sheet ambiances">
            <header class="sheet__header">
                <BackLink />
                <h1 class="sheet__title">{move || t(Key::AmbiancesHowItWorks)}</h1>
            </header>
            <section class="sheet__panel ambiances__about">
                <p>{move || t(Key::AmbiancesAbout1)}</p>
                <p>{move || t(Key::AmbiancesAbout2)}</p>
                <p>{move || t(Key::AmbiancesAbout3)}</p>
            </section>
            <A href="/ambiances" attr:class="btn ambiances__link">
                {move || t(Key::Ambiances)}
            </A>
        </div>
    }
}
