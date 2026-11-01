use nidoo_api::{AreaView, EntityView};
use nidoo_core::dashboard::{Dashboard, HOUSE_ID, HOUSE_NAME};

/// Returns the fixed House dashboard envelope.
/// Rationale (why this is empty): House is rendered on the client by the existing
/// rooms-overview screen (`screens::home::HomeScreen`), not by the generic dashboard
/// grid. The hub ships only the envelope so the client's picker lists House at
/// index 0; the client dispatches on `id == "house"` and renders directly from
/// the entity/area state it already has.
pub fn synthesize_house(_areas: &[AreaView], _entities: &[EntityView]) -> Dashboard {
    Dashboard {
        id: HOUSE_ID.to_owned(),
        name: HOUSE_NAME.to_owned(),
        order: 0,
        widgets: Vec::new(),
        show_header: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthesize_house_returns_fixed_shape() {
        let d = synthesize_house(&[], &[]);
        assert_eq!(d.id, "house");
        assert_eq!(d.name, "House");
        assert_eq!(d.order, 0);
        assert!(d.widgets.is_empty());
    }
}
