//! The history chart widget: the entity's history over a fixed range, in a card.

use leptos::prelude::*;
use nidoo_api::{EntityView, HistoryRange};
use nidoo_core::dashboard::DashboardHistoryRange;

use crate::components::history_chart::HistoryChart;
use crate::format::entity_title;
use crate::house::House;
use crate::screens::history_detail::{flavor_for_unit, power_pair_of};

/// The chart range that matches a widget's.
pub fn chart_range(range: DashboardHistoryRange) -> HistoryRange {
    match range {
        DashboardHistoryRange::H1 => HistoryRange::Hour,
        DashboardHistoryRange::H6 => HistoryRange::SixHours,
        DashboardHistoryRange::H24 => HistoryRange::Day,
        DashboardHistoryRange::D7 => HistoryRange::Week,
        DashboardHistoryRange::D30 => HistoryRange::Month,
    }
}

#[component]
pub fn HistoryWidget(entity: EntityView, range: DashboardHistoryRange) -> impl IntoView {
    let house = expect_context::<House>();
    let flavor = flavor_for_unit(entity.unit.as_deref());
    let id = entity.id.clone();
    let lang = crate::i18n::use_lang();
    let title_entity = entity.clone();
    let power_pair = Memo::new(move |_| house.entities.with(|m| power_pair_of(m, &id)));
    view! {
        <article class="card widget-history">
            <span class="widget-history__title">{move || entity_title(lang(), &title_entity)}</span>
            <HistoryChart
                entity_id=entity.id.clone()
                range=chart_range(range)
                flavor=flavor
                kind=format!("{:?}", entity.kind)
                unit=entity.unit.clone()
                power_pair=power_pair
            />
        </article>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widget_ranges_map_to_the_matching_chart_range() {
        assert_eq!(chart_range(DashboardHistoryRange::H1), HistoryRange::Hour);
        assert_eq!(
            chart_range(DashboardHistoryRange::H6),
            HistoryRange::SixHours
        );
        assert_eq!(chart_range(DashboardHistoryRange::H24), HistoryRange::Day);
        assert_eq!(chart_range(DashboardHistoryRange::D7), HistoryRange::Week);
        assert_eq!(chart_range(DashboardHistoryRange::D30), HistoryRange::Month);
    }
}
