use serde::{Deserialize, Serialize};

pub const HOUSE_ID: &str = "house";
pub const HOUSE_NAME: &str = "House";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Dashboard {
    pub id: String,
    pub name: String,
    pub order: u32,
    pub widgets: Vec<Widget>,
    /// Shows the house overview header (greeting, chips, weather) above the widgets.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub show_header: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Widget {
    pub id: String,
    pub kind: WidgetKind,
    pub span: Span,
    pub config: WidgetConfig,
    /// The widget's top-left cell on the desktop grid. Without one the widget flows into the
    /// first free cells in list order (see `layout`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<Cell>,
}

/// A cell of the [`GRID_COLUMNS`]-wide desktop grid, 0-based. Ordered row first, so sorting
/// cells gives reading order.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Cell {
    pub row: u16,
    pub col: u16,
}

/// Columns of the desktop grid that widget positions refer to. Narrower grids (4 on a tablet,
/// 2 on a phone) flow the widgets in reading order instead.
pub const GRID_COLUMNS: u16 = 6;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "snake_case")]
pub enum WidgetKind {
    Light,
    Cover,
    Climate,
    Lock,
    Sensor,
    RoomGroup,
    FlowButton,
    EnergyStat,
    HistoryChart,
    Camera,
    WaterHeater,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum Span {
    #[serde(rename = "1x1")]
    X1x1,
    #[serde(rename = "2x1")]
    X2x1,
    #[serde(rename = "2x2")]
    X2x2,
    #[serde(rename = "1x2")]
    X1x2,
    /// Wide: four columns, two rows (full width on a phone).
    #[serde(rename = "4x2")]
    X4x2,
    /// Full width: six columns, three rows (four on a tablet, two on a phone).
    #[serde(rename = "6x3")]
    X6x3,
}

impl Span {
    pub fn as_str(&self) -> &'static str {
        match self {
            Span::X1x1 => "1x1",
            Span::X2x1 => "2x1",
            Span::X2x2 => "2x2",
            Span::X1x2 => "1x2",
            Span::X4x2 => "4x2",
            Span::X6x3 => "6x3",
        }
    }

    /// Columns and rows the span covers on the desktop grid.
    pub fn size(&self) -> (u16, u16) {
        match self {
            Span::X1x1 => (1, 1),
            Span::X2x1 => (2, 1),
            Span::X2x2 => (2, 2),
            Span::X1x2 => (1, 2),
            Span::X4x2 => (4, 2),
            Span::X6x3 => (6, 3),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum DashboardHistoryRange {
    #[serde(rename = "1h")]
    H1,
    #[serde(rename = "6h")]
    H6,
    #[serde(rename = "24h")]
    H24,
    #[serde(rename = "7d")]
    D7,
    #[serde(rename = "30d")]
    D30,
}

/// Untagged: `History` is listed before `Entity` because both carry
/// `entity_id`, and `History` is the one that requires `range`.
/// Do not reorder: see `widget_config_history_decodes_before_entity`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum WidgetConfig {
    History {
        entity_id: String,
        range: DashboardHistoryRange,
    },
    Entity {
        entity_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    Area {
        area_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    Flow {
        flow_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    Energy {
        metric: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    /// Frigate camera ids, in display order.
    Cameras {
        cameras: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
}

/// The most cameras one Camera widget shows.
pub const MAX_CAMERAS: usize = 4;

pub fn allowed_spans(kind: WidgetKind) -> &'static [Span] {
    use Span::*;
    match kind {
        WidgetKind::Light
        | WidgetKind::Cover
        | WidgetKind::Climate
        | WidgetKind::Lock
        | WidgetKind::Sensor
        | WidgetKind::FlowButton
        | WidgetKind::WaterHeater
        | WidgetKind::EnergyStat => &[X1x1, X2x1],
        WidgetKind::RoomGroup => &[X1x1, X2x1, X2x2],
        WidgetKind::HistoryChart => &[X2x1, X2x2, X1x2],
        WidgetKind::Camera => &[X2x1, X2x2, X1x2, X4x2, X6x3],
    }
}

pub fn is_valid_dashboard_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id != HOUSE_ID
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_round_trips_as_str() {
        for (s, w) in [
            (Span::X1x1, "1x1"),
            (Span::X2x1, "2x1"),
            (Span::X2x2, "2x2"),
            (Span::X1x2, "1x2"),
            (Span::X4x2, "4x2"),
            (Span::X6x3, "6x3"),
        ] {
            assert_eq!(s.as_str(), w);
            assert_eq!(serde_json::to_string(&s).unwrap(), format!("\"{w}\""));
            assert_eq!(
                serde_json::from_str::<Span>(&format!("\"{w}\"")).unwrap(),
                s
            );
        }
    }

    #[test]
    fn allowed_spans_match_spec() {
        assert_eq!(allowed_spans(WidgetKind::Light), &[Span::X1x1, Span::X2x1]);
        assert_eq!(
            allowed_spans(WidgetKind::RoomGroup),
            &[Span::X1x1, Span::X2x1, Span::X2x2]
        );
        assert_eq!(
            allowed_spans(WidgetKind::HistoryChart),
            &[Span::X2x1, Span::X2x2, Span::X1x2]
        );
        assert_eq!(
            allowed_spans(WidgetKind::Camera),
            &[Span::X2x1, Span::X2x2, Span::X1x2, Span::X4x2, Span::X6x3]
        );
    }

    #[test]
    fn house_id_reserved() {
        assert!(!is_valid_dashboard_id("house"));
        assert!(is_valid_dashboard_id("morning_glance"));
        assert!(!is_valid_dashboard_id(""));
        assert!(!is_valid_dashboard_id("bad id"));
        assert!(is_valid_dashboard_id(&"a".repeat(64)));
        assert!(!is_valid_dashboard_id(&"a".repeat(65)));
    }

    #[test]
    fn dashboard_serde_round_trip() {
        let d = Dashboard {
            id: "d_1".into(),
            name: "Morning".into(),
            order: 1,
            widgets: vec![
                Widget {
                    id: "w1".into(),
                    kind: WidgetKind::Light,
                    span: Span::X1x1,
                    config: WidgetConfig::Entity {
                        entity_id: "lamp".into(),
                        title: Some("Lamp".into()),
                    },
                    pos: Some(Cell { row: 0, col: 2 }),
                },
                Widget {
                    id: "w2".into(),
                    kind: WidgetKind::HistoryChart,
                    span: Span::X2x2,
                    config: WidgetConfig::History {
                        entity_id: "temp".into(),
                        range: DashboardHistoryRange::D7,
                    },
                    pos: None,
                },
            ],
            show_header: true,
        };
        let json = serde_json::to_string(&d).unwrap();
        assert_eq!(serde_json::from_str::<Dashboard>(&json).unwrap(), d);
    }

    #[test]
    fn show_header_defaults_off_and_is_omitted_when_off() {
        let d: Dashboard =
            serde_json::from_str(r#"{ "id": "d", "name": "D", "order": 1, "widgets": [] }"#)
                .unwrap();
        assert!(!d.show_header);
        assert!(!serde_json::to_string(&d).unwrap().contains("show_header"));
        let on = Dashboard {
            show_header: true,
            ..d
        };
        assert!(
            serde_json::to_string(&on)
                .unwrap()
                .contains(r#""show_header":true"#)
        );
    }

    #[test]
    fn entity_widget_decodes_from_json() {
        let w: Widget = serde_json::from_str(
            r#"{ "id": "w", "kind": "light", "span": "1x1", "config": { "entity_id": "lamp" } }"#,
        )
        .unwrap();
        assert_eq!(w.kind, WidgetKind::Light);
        assert_eq!(w.span, Span::X1x1);
        assert_eq!(w.pos, None);
        assert!(!serde_json::to_string(&w).unwrap().contains("pos"));
        assert_eq!(
            w.config,
            WidgetConfig::Entity {
                entity_id: "lamp".into(),
                title: None
            }
        );
    }

    #[test]
    fn widget_config_history_decodes_before_entity() {
        let h: WidgetConfig = serde_json::from_str(r#"{"entity_id":"t","range":"24h"}"#).unwrap();
        assert_eq!(
            h,
            WidgetConfig::History {
                entity_id: "t".into(),
                range: DashboardHistoryRange::H24
            }
        );
        let e: WidgetConfig = serde_json::from_str(r#"{"entity_id":"t"}"#).unwrap();
        assert_eq!(
            e,
            WidgetConfig::Entity {
                entity_id: "t".into(),
                title: None
            }
        );
    }

    #[test]
    fn widget_config_cameras_round_trips() {
        let json = r#"{"cameras":["front_door","garden"],"title":"Outside"}"#;
        let c: WidgetConfig = serde_json::from_str(json).unwrap();
        assert_eq!(
            c,
            WidgetConfig::Cameras {
                cameras: vec!["front_door".into(), "garden".into()],
                title: Some("Outside".into())
            }
        );
        assert_eq!(serde_json::to_string(&c).unwrap(), json);
    }

    #[test]
    fn history_range_round_trips_wire_strings() {
        for (r, w) in [
            (DashboardHistoryRange::H1, "1h"),
            (DashboardHistoryRange::H6, "6h"),
            (DashboardHistoryRange::H24, "24h"),
            (DashboardHistoryRange::D7, "7d"),
            (DashboardHistoryRange::D30, "30d"),
        ] {
            assert_eq!(serde_json::to_string(&r).unwrap(), format!("\"{w}\""));
            assert_eq!(
                serde_json::from_str::<DashboardHistoryRange>(&format!("\"{w}\"")).unwrap(),
                r
            );
        }
    }
}
