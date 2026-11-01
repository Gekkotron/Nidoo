//! Energy view switch: the Classic, Flux (Sankey) or Orbite diagram. The choice lives in the
//! hub's home settings, so every browser draws the same one.

use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use leptos::task::spawn_local;
pub use nidoo_api::EnergyView;

#[cfg(target_arch = "wasm32")]
use crate::components::toast::Toasts;
use crate::house::House;
#[cfg(target_arch = "wasm32")]
use crate::i18n::{HomeText, Lang, home_text};

/// The house's choice, as last received from the hub.
pub fn energy_view(house: House) -> EnergyView {
    house.home_settings.with(|s| s.energy_view)
}

/// Shows `view` at once and saves it in the home settings; a refused save puts the stored
/// choice back and says so in a toast.
pub fn set_energy_view(house: House, view: EnergyView) {
    let previous = house.home_settings.get_untracked();
    if previous.energy_view == view {
        return;
    }
    let settings = nidoo_api::HomeSettings {
        energy_view: view,
        ..previous.clone()
    };
    house.home_settings.set(settings.clone());
    #[cfg(target_arch = "wasm32")]
    {
        let toasts = use_context::<Toasts>();
        let lang = use_context::<RwSignal<Lang>>();
        spawn_local(async move {
            match crate::api::home::put_home_settings(&settings).await {
                Ok(stored) => house.home_settings.set(stored),
                Err(e) => {
                    house.home_settings.set(previous);
                    if let Some(toasts) = toasts {
                        let l = lang.map_or(Lang::En, |l| l.get_untracked());
                        toasts.push(format!(
                            "{}: {}",
                            home_text(l, HomeText::CouldNotSave),
                            e.error
                        ));
                    }
                }
            }
        });
    }
    #[cfg(not(target_arch = "wasm32"))]
    let _ = previous;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_shows_the_choice_at_once() {
        let house = House::new();
        assert_eq!(energy_view(house), EnergyView::Classic);
        set_energy_view(house, EnergyView::Flux);
        assert_eq!(energy_view(house), EnergyView::Flux);
    }
}
