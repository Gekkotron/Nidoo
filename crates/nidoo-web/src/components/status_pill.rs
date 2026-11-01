//! The connection pill at the foot of the sidebar.

use leptos::prelude::*;
use nidoo_api::ConnectionStatus;

use crate::house::House;
use crate::i18n::{Key, use_text};

#[component]
pub fn StatusPill() -> impl IntoView {
    let t = use_text();
    let house = expect_context::<House>();
    let connection = house.connection;
    let class = move || match connection.get() {
        ConnectionStatus::Connecting => "pill pill--connecting",
        ConnectionStatus::Online => "pill pill--online",
        ConnectionStatus::Offline { .. } => "pill pill--offline",
    };
    let text = move || match connection.get() {
        ConnectionStatus::Connecting => t(Key::Reconnecting),
        ConnectionStatus::Online => t(Key::Online),
        ConnectionStatus::Offline { .. } => t(Key::Offline),
    };
    let title = move || match connection.get() {
        ConnectionStatus::Offline { reason } => reason,
        _ => String::new(),
    };
    view! {
        <span class=class title=title role="status">
            <span class="pill__dot"></span>
            {text}
        </span>
    }
}
