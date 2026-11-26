use super::*;
use crate::flow::Scalar;
use crate::model::AlarmMode;

#[test]
fn cover_without_stop_payload_offers_no_stop() {
    let caps = Capabilities {
        open_close: true,
        stop: false,
        position: true,
        ..Default::default()
    };
    let cmds: Vec<_> = actions(Kind::Cover, "cover", &caps)
        .into_iter()
        .map(|a| a.command)
        .collect();
    assert_eq!(
        cmds,
        vec![
            Command::Open,
            Command::Close,
            Command::SetPosition { position: 0 }
        ]
    );
}

#[test]
fn colour_light_turn_on_has_brightness_colour_and_effect() {
    let caps = Capabilities {
        brightness: true,
        color_modes: ["xy".to_owned()].into(),
        effects: vec!["blink".into()],
        ..Default::default()
    };
    let list = actions(Kind::Light, "light", &caps);
    assert_eq!(
        list[0].params.iter().map(Param::key).collect::<Vec<_>>(),
        ["brightness", "color", "effect"]
    );
}

#[test]
fn alarm_with_code_has_a_secret_code_param() {
    let caps = Capabilities {
        alarm_modes: [AlarmMode::Away].into(),
        code_arm_required: true,
        ..Default::default()
    };
    let list = actions(Kind::AlarmPanel, "alarm_control_panel", &caps);
    assert_eq!(list[0].command, Command::Disarm { code: None });
    assert_eq!(list[0].params, vec![]);
    assert_eq!(
        list[1].params,
        vec![Param::Text {
            key: "code".into(),
            secret: true,
            optional: false
        }]
    );
}

#[test]
fn select_gives_one_action_per_option() {
    let caps = Capabilities {
        options: vec!["a".into(), "b".into()],
        ..Default::default()
    };
    assert_eq!(actions(Kind::Select, "select", &caps).len(), 2);
}

#[test]
fn temperature_starts_mid_range_on_a_step() {
    let caps = Capabilities {
        min_temp: Some(5.0),
        max_temp: Some(35.0),
        temp_step: Some(0.5),
        ..Default::default()
    };
    assert_eq!(
        actions(Kind::Climate, "climate", &caps)[0].command,
        Command::SetTemperature { temperature: 20.0 }
    );
    let heater = Capabilities {
        min_temp: Some(30.0),
        max_temp: Some(61.0),
        temp_step: Some(1.0),
        ..Default::default()
    };
    assert_eq!(
        actions(Kind::WaterHeater, "water_heater", &heater)[0].command,
        Command::SetTemperature { temperature: 46.0 }
    );
}

#[test]
fn sensors_accept_nothing() {
    assert!(actions(Kind::Sensor, "sensor", &Capabilities::default()).is_empty());
}

#[test]
fn cover_reports_the_words_core_keeps() {
    let Reports::Values { values } = reports(Kind::Cover, "cover", &Capabilities::default(), None)
    else {
        panic!("cover reports words")
    };
    assert_eq!(
        values,
        ["open", "closed", "opening", "closing"].map(|w| Scalar::Text(w.into()))
    );
}

#[test]
fn switch_reports_booleans() {
    assert_eq!(
        reports(Kind::Switch, "switch", &Capabilities::default(), None),
        Reports::Values {
            values: vec![Scalar::Bool(true), Scalar::Bool(false)]
        }
    );
}

#[test]
fn alarm_reports_only_announced_armed_states() {
    let caps = Capabilities {
        alarm_modes: [AlarmMode::Away].into(),
        ..Default::default()
    };
    let Reports::Values { values } = reports(Kind::AlarmPanel, "alarm_control_panel", &caps, None)
    else {
        panic!("alarm reports words")
    };
    assert!(values.contains(&Scalar::Text("disarmed".into())));
    assert!(values.contains(&Scalar::Text("armed_away".into())));
    assert!(values.contains(&Scalar::Text("triggered".into())));
    assert!(!values.contains(&Scalar::Text("armed_home".into())));
}

#[test]
fn sensor_with_unit_is_a_number() {
    assert_eq!(
        reports(Kind::Sensor, "sensor", &Capabilities::default(), Some("°C")),
        Reports::Number {
            unit: Some("°C".into()),
            min: None,
            max: None
        }
    );
    assert_eq!(
        reports(Kind::Sensor, "sensor", &Capabilities::default(), None),
        Reports::Text
    );
}

#[test]
fn select_and_climate_report_their_announced_lists() {
    let caps = Capabilities {
        options: vec!["eco".into()],
        hvac_modes: vec!["heat".into()],
        ..Default::default()
    };
    assert_eq!(
        reports(Kind::Select, "select", &caps, None),
        Reports::Values {
            values: vec![Scalar::Text("eco".into())]
        }
    );
    assert_eq!(
        reports(Kind::Climate, "climate", &caps, None),
        Reports::Values {
            values: vec![Scalar::Text("heat".into())]
        }
    );
}

#[test]
fn generic_lock_reports_its_primary_choice() {
    let generic = crate::types::GenericCaps {
        options: [("state".to_owned(), vec!["locked".into(), "unlocked".into()])].into(),
        ..Default::default()
    };
    let caps = Capabilities {
        generic: Some(generic),
        ..Default::default()
    };
    assert_eq!(
        reports(Kind::Generic, "lock", &caps, None),
        Reports::Values {
            values: vec![
                Scalar::Text("locked".into()),
                Scalar::Text("unlocked".into())
            ]
        }
    );
}

#[test]
fn button_reports_nothing() {
    assert_eq!(
        reports(Kind::Button, "button", &Capabilities::default(), None),
        Reports::Nothing
    );
}

#[test]
fn media_player_reports_its_three_states() {
    assert_eq!(
        reports(
            Kind::MediaPlayer,
            "media_player",
            &Capabilities::default(),
            None
        ),
        Reports::Values {
            values: ["playing", "paused", "idle"]
                .iter()
                .map(|w| Scalar::Text((*w).to_owned()))
                .collect()
        }
    );
}

#[test]
fn media_player_actions_follow_features() {
    let caps = Capabilities {
        media_features: ["play", "next", "play_media"]
            .iter()
            .map(|f| (*f).to_owned())
            .collect(),
        ..Default::default()
    };
    let list = actions(Kind::MediaPlayer, "media_player", &caps);
    let commands: Vec<_> = list.iter().map(|a| a.command.clone()).collect();
    assert_eq!(
        commands,
        vec![
            Command::MediaAction {
                action: "play".into()
            },
            Command::MediaAction {
                action: "next".into()
            },
            Command::PlayMedia {
                query: String::new()
            },
        ]
    );
    assert!(list[0].params.is_empty());
    assert_eq!(
        list[2].params,
        vec![Param::Text {
            key: "query".into(),
            secret: false,
            optional: false
        }]
    );
}

#[test]
fn media_player_play_playlist_action_has_defaults_and_no_params() {
    let caps = Capabilities {
        media_features: ["play", "play_playlist"]
            .iter()
            .map(|f| (*f).to_owned())
            .collect(),
        ..Default::default()
    };
    let list = actions(Kind::MediaPlayer, "media_player", &caps);
    assert_eq!(
        list[1].command,
        Command::PlayPlaylist {
            playlist: "favorites".into(),
            shuffle: true,
            source: None,
            fade_in_s: 60,
        }
    );
    assert!(list[1].params.is_empty());
}
