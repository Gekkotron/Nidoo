//! The Energy card: which sensors measure the grid and the solar panels, and the off-peak
//! hours. The same pieces make the Energy panel on the Home settings page.

use leptos::prelude::*;
use nidoo_api::{EntityView, OverridePatch};
use nidoo_core::energy::EnergyRole;
use serde_json::json;

use crate::house::House;
use crate::i18n::setup::{SetupText, setup_text, use_setup_text};
use crate::i18n::{HomeText, Lang, home_text, use_lang};
use crate::screens::home_settings_panel::{
    Draft, OffPeakEditor, off_peak_device, off_peak_windows,
};

/// Sensors that can stand for a role: those that read energy or power, sorted by name.
pub fn role_candidates(entities: &[EntityView]) -> Vec<EntityView> {
    let mut list: Vec<EntityView> = entities
        .iter()
        .filter(|e| matches!(e.device_class.as_deref(), Some("energy" | "power")))
        .filter(|e| e.kind == nidoo_core::model::Kind::Sensor)
        .cloned()
        .collect();
    list.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    list
}

/// The entity whose `energy_role` override reads as `role`.
pub fn holder(entities: &[EntityView], role: EnergyRole) -> Option<String> {
    entities
        .iter()
        .find(|e| {
            e.ui.energy_role
                .as_deref()
                .and_then(EnergyRole::parse)
                .is_some_and(|r| r == role)
        })
        .map(|e| e.id.clone())
}

/// The options of the picker for `role`: candidates that hold no other role, plus the current
/// holder of `role` even when it would not qualify, so the select never misreports it.
pub fn role_options(entities: &[EntityView], role: EnergyRole) -> Vec<EntityView> {
    let mut list: Vec<EntityView> = role_candidates(entities)
        .into_iter()
        .filter(|e| {
            e.ui.energy_role
                .as_deref()
                .and_then(EnergyRole::parse)
                .is_none_or(|r| r == role)
        })
        .collect();
    if let Some(h) = holder(entities, role)
        && !list.iter().any(|e| e.id == h)
        && let Some(e) = entities.iter().find(|e| e.id == h)
    {
        list.push(e.clone());
    }
    list
}

/// The stored name a role is written under.
fn role_name(role: EnergyRole) -> &'static str {
    match role {
        EnergyRole::Grid => "grid",
        EnergyRole::GridExport => "grid_export",
        EnergyRole::Solar => "solar",
        EnergyRole::Battery => "battery",
        EnergyRole::House => "house",
        EnergyRole::Consumer => "consumer",
        EnergyRole::Water => "water",
    }
}

fn role_patch(role: Option<EnergyRole>) -> OverridePatch {
    let mut patch = OverridePatch::new();
    patch.insert("energyRole".to_owned(), json!(role.map(role_name)));
    patch
}

fn role_label(role: EnergyRole) -> SetupText {
    match role {
        EnergyRole::GridExport => SetupText::RoleGridExport,
        EnergyRole::Solar => SetupText::RoleSolar,
        _ => SetupText::RoleGrid,
    }
}

/// The house's entities as a list.
fn all_entities(house: House) -> Vec<EntityView> {
    house.entities.with(|m| m.values().cloned().collect())
}

/// A select that names the sensor holding `role`. Choosing another sensor clears the role on
/// the old holder first and then sets it on the new one; no hub restart is involved. The
/// select always shows the live holder, so a failed move snaps back to it.
#[component]
pub fn EnergyRolePicker(role: EnergyRole) -> impl IntoView {
    let house = expect_context::<House>();
    let t = use_setup_text();
    let lang = use_lang();
    let failure = RwSignal::new(None::<String>);
    let select = NodeRef::<leptos::html::Select>::new();
    let current = Memo::new(move |_| {
        house
            .entities
            .with(|m| holder(&m.values().cloned().collect::<Vec<_>>(), role))
    });
    let options = Memo::new(move |_| role_options(&all_entities(house), role));
    let busy = RwSignal::new(false);
    let on_change = move |ev: leptos::ev::Event| {
        let value = event_target_value(&ev);
        let previous = current.get_untracked();
        if busy.get_untracked() || previous.as_deref().unwrap_or_default() == value {
            return;
        }
        failure.set(None);
        busy.set(true);
        leptos::task::spawn_local(async move {
            // Each reply goes into the house at once, so the holder is right before any echo.
            let mut cleared = false;
            let mut result = Ok(());
            if let Some(old) = previous {
                match crate::api::set_override(&old, role_patch(None)).await {
                    Ok(view) => {
                        house.entities.update(|m| {
                            m.insert(view.id.clone(), view);
                        });
                        cleared = true;
                    }
                    Err(e) => result = Err(e),
                }
            }
            if result.is_ok() && !value.is_empty() {
                match crate::api::set_override(&value, role_patch(Some(role))).await {
                    Ok(view) => {
                        house.entities.update(|m| {
                            m.insert(view.id.clone(), view);
                        });
                    }
                    Err(e) => result = Err(e),
                }
            }
            if let Err(e) = result {
                let key = if cleared {
                    SetupText::RoleClearedNotSet
                } else {
                    SetupText::RoleMoveFailed
                };
                failure.try_set(Some(setup_text(lang(), key).replace("{error}", &e.error)));
            }
            if let Some(el) = select.try_get_untracked().flatten() {
                el.set_value(&current.try_get_untracked().flatten().unwrap_or_default());
            }
            busy.try_set(false);
        });
    };
    let id = format!("energy-{}", role_name(role));
    view! {
        <div class="climate-field energy__field">
            <label class="climate-field__label" for=id.clone()>
                {move || t(role_label(role))}
            </label>
            <select id=id node_ref=select class="select energy__role"
                prop:disabled=move || busy.get()
                on:change=on_change
            >
                <option value="" prop:selected=move || current.with(Option::is_none)>
                    {move || home_text(lang(), HomeText::NoneChoice)}
                </option>
                <For
                    each=move || options.get()
                    key=|e| (e.id.clone(), e.name.clone())
                    children=move |e| {
                        let id = e.id.clone();
                        let value = e.id.clone();
                        view! {
                            <option
                                value=value
                                prop:selected=move || current.with(|c| c.as_deref() == Some(id.as_str()))
                            >
                                {e.name}
                            </option>
                        }
                    }
                />
            </select>
            <p class="loc__problem" role="alert">{move || failure.get()}</p>
        </div>
    }
}

/// The three role pickers.
#[component]
fn RolePickers() -> impl IntoView {
    view! {
        <EnergyRolePicker role=EnergyRole::Grid />
        <EnergyRolePicker role=EnergyRole::GridExport />
        <EnergyRolePicker role=EnergyRole::Solar />
    }
}

/// The Energy panel of the Home settings page: the role pickers.
#[component]
pub fn EnergyPanel() -> impl IntoView {
    let t = use_setup_text();
    view! {
        <section class="sheet__panel home-settings">
            <h2 class="sheet__heading">{move || t(SetupText::SvcEnergy)}</h2>
            <p class="settings__hint">{move || t(SetupText::DescEnergy)}</p>
            <RolePickers />
        </section>
    }
}

/// The off-peak editor with its own Save. A save reads the settings document again and puts
/// it back with only `off_peak` and `off_peak_device` replaced, so the other home settings
/// travel unchanged.
#[component]
fn OffPeakCard() -> impl IntoView {
    let house = use_context::<House>();
    let lang = use_lang();
    let draft = RwSignal::new(Draft::default());
    let loaded = RwSignal::new(None::<Result<(), String>>);
    let problem = RwSignal::new(None::<String>);
    let saved = RwSignal::new(false);
    let saving = RwSignal::new(false);
    // An edit after a save leaves the "Saved" note stale.
    Effect::new(move |previous: Option<Draft>| {
        let now = draft.get();
        if previous.is_some_and(|p| p != now) {
            saved.set(false);
        }
        now
    });
    leptos::task::spawn_local(async move {
        match crate::api::home::get_home_settings().await {
            Ok(s) => {
                let fresh = Draft::new(Lang::En, &s);
                draft.try_update(|d| {
                    d.off_peak = fresh.off_peak;
                    d.off_peak_device = fresh.off_peak_device;
                });
                loaded.try_set(Some(Ok(())));
            }
            Err(e) => {
                loaded.try_set(Some(Err(e.error)));
            }
        }
    });
    let save = move |_| {
        if saving.get_untracked() {
            return;
        }
        saving.set(true);
        saved.set(false);
        problem.set(None);
        let edited = draft.get_untracked();
        let rows = edited.off_peak.clone();
        let device = off_peak_device(&edited);
        leptos::task::spawn_local(async move {
            let result = async {
                let mut settings = crate::api::home::get_home_settings().await?;
                settings.off_peak = off_peak_windows(&rows);
                settings.off_peak_device = device;
                crate::api::home::put_home_settings(&settings).await
            }
            .await;
            match result {
                Ok(stored) => {
                    if let Some(house) = house {
                        house.home_settings.set(stored);
                    }
                    saved.try_set(true);
                }
                Err(e) => {
                    problem.try_set(Some(e.error));
                }
            }
            saving.try_set(false);
        });
    };
    move || match loaded.get() {
        None => ().into_any(),
        Some(Err(e)) => view! {
            <p class="settings__empty" role="alert">
                {format!("{} {e}", setup_text(lang(), SetupText::LoadFailed))}
            </p>
        }
        .into_any(),
        Some(Ok(())) => view! {
            <OffPeakEditor draft=draft>
                <p class="home-settings__error" role="alert">{move || problem.get()}</p>
            </OffPeakEditor>
            <p class="settings__hint" role="status">
                {move || saved.get().then(|| home_text(lang(), HomeText::Saved))}
            </p>
            <button
                type="button"
                class="chip home-settings__save"
                prop:disabled=move || saving.get()
                on:click=save
            >
                {move || home_text(lang(), HomeText::Save)}
            </button>
        }
        .into_any(),
    }
}

/// The Energy card's body: the role pickers and the off-peak hours.
#[component]
pub fn EnergyForm() -> impl IntoView {
    view! {
        <div class="svc__form energy__form">
            <RolePickers />
            <OffPeakCard />
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nidoo_core::model::{Capabilities, EntityCategory, EntityState, Kind, UiOverride};

    fn sensor(id: &str, device_class: &str, role: Option<&str>) -> EntityView {
        EntityView {
            id: id.into(),
            device_id: None,
            name: id.into(),
            device_name: None,
            label: None,
            kind: Kind::Sensor,
            component: Kind::Sensor.component().to_string(),
            protocol_kind: Kind::Sensor,
            device_class: Some(device_class.into()),
            unit: None,
            icon: None,
            category: EntityCategory::Primary,
            enabled_by_default: true,
            area: "salon".into(),
            part_of: None,
            capabilities: Capabilities::default(),
            ui: UiOverride {
                energy_role: role.map(Into::into),
                ..Default::default()
            },
            state: EntityState::default(),
        }
    }

    #[test]
    fn candidates_are_energy_and_power_sensors_by_name() {
        let list = [
            sensor("p_house", "power", None),
            sensor("t_hall", "temperature", None),
            sensor("e_meter", "energy", None),
        ];
        let ids: Vec<String> = role_candidates(&list).into_iter().map(|e| e.id).collect();
        assert_eq!(ids, ["e_meter", "p_house"]);
    }

    #[test]
    fn holder_is_the_entity_whose_override_parses_to_the_role() {
        let list = [
            sensor("a", "power", Some("grid")),
            sensor("b", "power", Some("solar")),
            sensor("c", "power", Some("nonsense")),
        ];
        assert_eq!(holder(&list, EnergyRole::Grid).as_deref(), Some("a"));
        assert_eq!(holder(&list, EnergyRole::Solar).as_deref(), Some("b"));
        assert_eq!(holder(&list, EnergyRole::GridExport), None);
    }

    #[test]
    fn options_leave_out_sensors_holding_another_role() {
        let list = [
            sensor("a_free", "power", None),
            sensor("b_solar", "power", Some("solar")),
            sensor("c_grid", "power", Some("grid")),
            sensor("d_temp_grid", "temperature", Some("grid")),
        ];
        let ids = |role| -> Vec<String> {
            role_options(&list, role)
                .into_iter()
                .map(|e| e.id)
                .collect()
        };
        assert_eq!(ids(EnergyRole::Grid), ["a_free", "c_grid"]);
        assert_eq!(ids(EnergyRole::Solar), ["a_free", "b_solar"]);
        // A holder that is no energy or power sensor stays listed.
        let mut with_odd = list.to_vec();
        with_odd[3] = sensor("d_temp_grid", "temperature", Some("grid"));
        assert!(
            role_options(&with_odd[3..], EnergyRole::Grid)
                .iter()
                .any(|e| e.id == "d_temp_grid")
        );
    }
}
