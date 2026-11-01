pub mod camera_widget;
pub mod config_sheet;
pub mod edit;
pub mod energy_stat;
pub mod flow_button;
pub mod grid;
pub mod history_widget;
pub mod library;
pub mod room_group;
pub mod widget;

use leptos::prelude::*;
use nidoo_api::DashboardView;
use nidoo_core::dashboard::Dashboard;

use crate::api;
use crate::house::House;
use crate::i18n::{Key, Lang, text};

/// Saves an empty "New dashboard" after the last one and adds it to `house`; returns its id.
/// The name is written in `lang`, the language active at the click, and is never re-translated.
/// `None` when the save fails: the list stays as it was and the edit-mode task owns error display.
pub(crate) async fn create_dashboard(house: House, lang: Lang) -> Option<String> {
    let order = house
        .dashboards
        .with_untracked(|list| list.iter().map(|d| d.dashboard.order).max().unwrap_or(0) + 1);
    let d = Dashboard {
        id: format!("d_{}", gen_id()),
        name: text(lang, Key::NewDashboard).to_owned(),
        order,
        widgets: Vec::new(),
        show_header: false,
    };
    let saved = api::put_dashboard(&d).await.ok()?;
    let id = saved.id.clone();
    house.dashboards.update(|list| {
        list.push(DashboardView {
            dashboard: saved,
            synthesized: false,
        });
    });
    Some(id)
}

/// A short URL-safe id from the clock; avoids a uuid dependency for a single-user client.
fn gen_id() -> String {
    format!("{:x}", js_sys::Date::now() as u64)
}
