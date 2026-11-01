//! The card order of a room being edited, and the positions Save writes. Pure; the room page
//! lives in `screens::room`.

/// The ids to show while editing: the draft's ids that are still in the room, in draft order,
/// then any card that arrived since the edit began, in Home order.
pub fn arranged(draft: &[String], live: &[String]) -> Vec<String> {
    let mut ids: Vec<String> = draft
        .iter()
        .filter(|id| live.contains(id))
        .cloned()
        .collect();
    ids.extend(live.iter().filter(|id| !draft.contains(id)).cloned());
    ids
}

/// Moves `id` to the place `onto` holds; the cards between them shift by one. Unknown ids
/// leave the order as it is.
pub fn move_onto(order: &mut Vec<String>, id: &str, onto: &str) {
    let (Some(from), Some(to)) = (
        order.iter().position(|x| x == id),
        order.iter().position(|x| x == onto),
    ) else {
        return;
    };
    let moved = order.remove(from);
    order.insert(to, moved);
}

/// Moves `id` one place earlier (`-1`) or later (`1`), within the list.
pub fn move_by(order: &mut Vec<String>, id: &str, step: isize) {
    let Some(from) = order.iter().position(|x| x == id) else {
        return;
    };
    let to = from.saturating_add_signed(step).min(order.len() - 1);
    let moved = order.remove(from);
    order.insert(to, moved);
}

/// The `order` each card needs so Home keeps `ids` as they are: 1, 2, 3… Only the cards whose
/// stored `order` differs are listed.
pub fn order_changes(ids: &[String], stored: impl Fn(&str) -> Option<i32>) -> Vec<(String, i32)> {
    ids.iter()
        .zip(1..)
        .filter(|(id, at)| stored(id) != Some(*at))
        .map(|(id, at)| (id.clone(), at))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_card_takes_the_place_of_the_one_it_is_dropped_on() {
        let mut order = ids(&["a", "b", "c", "d"]);
        move_onto(&mut order, "a", "c");
        assert_eq!(order, ids(&["b", "c", "a", "d"]));
        move_onto(&mut order, "d", "b");
        assert_eq!(order, ids(&["d", "b", "c", "a"]));
        move_onto(&mut order, "x", "b");
        assert_eq!(order, ids(&["d", "b", "c", "a"]));
    }

    #[test]
    fn a_step_stays_inside_the_list() {
        let mut order = ids(&["a", "b", "c"]);
        move_by(&mut order, "a", -1);
        assert_eq!(order, ids(&["a", "b", "c"]));
        move_by(&mut order, "a", 1);
        assert_eq!(order, ids(&["b", "a", "c"]));
        move_by(&mut order, "c", 1);
        assert_eq!(order, ids(&["b", "a", "c"]));
        move_by(&mut order, "c", -1);
        assert_eq!(order, ids(&["b", "c", "a"]));
    }

    #[test]
    fn the_draft_drops_gone_cards_and_appends_new_ones() {
        let draft = ids(&["c", "a", "b"]);
        let live = ids(&["a", "b", "d", "e"]);
        assert_eq!(arranged(&draft, &live), ids(&["a", "b", "d", "e"]));
        let live = ids(&["e", "a", "b", "c"]);
        assert_eq!(arranged(&draft, &live), ids(&["c", "a", "b", "e"]));
    }

    #[test]
    fn save_writes_only_the_positions_that_changed() {
        let stored = |id: &str| match id {
            "a" => Some(1),
            "b" => Some(5),
            _ => None,
        };
        assert_eq!(
            order_changes(&ids(&["a", "b", "c"]), stored),
            vec![("b".to_owned(), 2), ("c".to_owned(), 3)]
        );
        assert!(order_changes(&ids(&["a"]), stored).is_empty());
    }
}
