//! Where a card is shown. Room sections and sheets say so through context; the card reads it to
//! drop a room line its heading already gives, to choose its width and to show the controls it
//! keeps for its sheet.

use leptos::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CardPlace {
    /// Anywhere that does not say otherwise: Favorites, dashboards.
    #[default]
    Overview,
    /// Under its own room's heading (a room section on Home or on `/rooms/:id`).
    Room,
    /// A sheet's controls panel: the card is the sheet's only control, so it shows them all.
    Sheet,
}

/// Tells the cards built below the current owner where they are.
pub fn provide_card_place(place: CardPlace) {
    provide_context(place);
}

/// Where the card being built is shown; [`CardPlace::Overview`] when nobody said.
pub fn use_card_place() -> CardPlace {
    use_context::<CardPlace>().unwrap_or_default()
}

/// Renders `children` as a sheet's controls: the cards in it are in [`CardPlace::Sheet`].
#[component]
pub fn InSheet(children: Children) -> impl IntoView {
    provide_card_place(CardPlace::Sheet);
    children()
}
