//! The climate switch, its retained documents and the read view, driven without a broker.

use std::path::PathBuf;
use std::sync::Arc;

use nidoo::config::Config;
use nidoo::hub::{Hub, HubError};
use nidoo_api::{ConnectionStatus, WsEvent};
use nidoo_climate::doc::{Block, Day, HhMm, ModeRules, Rules, Setpoint, ZoneSettings, ZoneState};
use nidoo_core::command::Publish;
use tokio::sync::mpsc;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../nidoo-core/tests/fixtures/discovery/"
        ))
        .join(name),
    )
    .expect("fixture")
}

fn hub(enabled: bool) -> (Arc<Hub>, mpsc::Receiver<Publish>) {
    let mut cfg = Config::default();
    cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
    cfg.climate.enabled = enabled;
    let (tx, rx) = mpsc::channel(64);
    let hub = Hub::new(&cfg, tx);
    hub.set_connection(ConnectionStatus::Online);
    (hub, rx)
}

/// The recorded Daikin `n` (0..=3), placed in `area` by override; returns its entity id.
async fn feed_daikin(hub: &Hub, n: u8, area: &str) -> String {
    let name = format!("climate__daikin_192_0_2_15{n}__climate.json");
    let topic = format!("nidoo/climate/daikin_192_0_2_15{n}/climate/config");
    hub.ingest(&topic, &fixture(&name));
    place(hub, &format!("daikin_192_0_2_15{n}"), area).await
}

/// Puts the entity whose id starts with `prefix` in `area`.
async fn place(hub: &Hub, prefix: &str, area: &str) -> String {
    let id = hub
        .snapshot()
        .entities
        .into_iter()
        .find(|e| e.id.starts_with(prefix))
        .unwrap_or_else(|| panic!("no entity {prefix}"))
        .id;
    let mut patch = serde_json::Map::new();
    patch.insert("area".to_owned(), serde_json::Value::from(area));
    hub.set_override(&id, &patch).await.expect("area override");
    id
}

fn drain(rx: &mut mpsc::Receiver<Publish>) -> Vec<Publish> {
    let mut out = Vec::new();
    while let Ok(p) = rx.try_recv() {
        out.push(p);
    }
    out
}

fn active() -> ZoneSettings {
    ZoneSettings {
        state: ZoneState::Active,
        ..ZoneSettings::default()
    }
}

#[tokio::test]
async fn disabled_has_no_filters_and_refuses() {
    let (hub, mut rx) = hub(false);
    assert!(!hub.extra_filters().iter().any(|f| f.contains("climate")));
    assert!(!hub.climate_enabled());
    assert!(matches!(hub.climate_view(), Err(HubError::Publish(m)) if m == "climate disabled"));
    assert!(matches!(
        hub.set_climate_zone("living", active()),
        Err(HubError::Publish(m)) if m == "climate disabled"
    ));
    // A climate-looking topic is just an unknown topic while the engine is off.
    assert!(
        hub.ingest_message("nidoo/climate/house", b"{}", true)
            .is_empty()
    );
    assert!(drain(&mut rx).is_empty());
    assert!(matches!(hub.climate_view(), Err(HubError::Publish(m)) if m == "climate disabled"));
}

#[tokio::test]
async fn enabled_subscribes_house_and_zones_only() {
    let (hub, _rx) = hub(true);
    let f = hub.extra_filters();
    assert!(
        f.contains(&"nidoo/climate/house".to_owned())
            && f.contains(&"nidoo/climate/zone/+/+".to_owned())
            && f.contains(&"nidoo/climate/zone/+/hold/set".to_owned())
    );
    assert!(!f.iter().any(|t| t.ends_with('#')));
}

#[tokio::test]
async fn zone_settings_doc_is_not_a_device() {
    let (hub, _rx) = hub(true);
    let changes = hub.ingest_message(
        "nidoo/climate/zone/living/settings",
        br#"{"state":"active"}"#,
        true,
    );
    assert!(changes.is_empty());
    assert!(
        hub.snapshot()
            .entities
            .iter()
            .all(|e| !e.id.contains("living"))
    );
    assert!(hub.snapshot().entities.is_empty());
}

#[tokio::test]
async fn climate_group_discovery_still_reaches_registry() {
    let (hub, _rx) = hub(true);
    let text = String::from_utf8(fixture("climate__daikin_192_0_2_150__climate.json"))
        .unwrap()
        .replace("daikin_192_0_2_150", "group_g1");
    hub.ingest_message("nidoo/climate/group/g1/config", text.as_bytes(), true);
    assert!(
        hub.snapshot()
            .entities
            .iter()
            .any(|e| e.id.starts_with("group_g1")),
        "a group's discovery config under the climate prefix is still a device"
    );
}

#[tokio::test]
async fn store_keeps_invalid_with_reason() {
    let (hub, _rx) = hub(true);
    let mut events = hub.subscribe();
    hub.ingest_message(
        "nidoo/climate/zone/attic/settings",
        br#"{"state":"active"}"#,
        true,
    );
    assert!(matches!(
        events.recv().await.unwrap(),
        WsEvent::ClimateChanged { zone: Some(z) } if z == "attic"
    ));
    hub.ingest_message(
        "nidoo/climate/zone/attic/settings",
        br#"{"state":"bogus"}"#,
        true,
    );
    let view = hub.climate_view().unwrap();
    let zone = view.zones.iter().find(|z| z.zone == "attic").unwrap();
    assert_eq!(zone.settings.state, ZoneState::Active);
    assert!(zone.settings_invalid.is_some());

    hub.ingest_message("nidoo/climate/house", br#"{"frost":40.0}"#, true);
    let view = hub.climate_view().unwrap();
    assert!(view.house_invalid.is_some());
}

#[tokio::test]
async fn view_lists_area_zones_and_skips_groups() {
    let (hub, _rx) = hub(true);
    let a = feed_daikin(&hub, 0, "living").await;
    let b = feed_daikin(&hub, 1, "living").await;
    let text = String::from_utf8(fixture("climate__daikin_192_0_2_152__climate.json"))
        .unwrap()
        .replace("daikin_192_0_2_152", "group_g1");
    hub.ingest("nidoo/climate/group/g1/config", text.as_bytes());
    place(&hub, "group_g1", "living").await;

    let view = hub.climate_view().unwrap();
    assert!(view.enabled && !view.history);
    assert_eq!(view.zones.len(), 1, "{:?}", view.zones);
    let zone = &view.zones[0];
    assert_eq!(zone.zone, "living");
    assert!(!zone.orphan);
    let mut want = vec![a, b];
    want.sort();
    assert_eq!(zone.entities, want);

    // `settings.entities` wins over the area's devices.
    let settings = ZoneSettings {
        entities: Some(vec![want[0].clone()]),
        ..active()
    };
    hub.ingest_message(
        "nidoo/climate/zone/living/settings",
        serde_json::to_string(&settings).unwrap().as_bytes(),
        true,
    );
    let view = hub.climate_view().unwrap();
    assert_eq!(view.zones[0].entities, vec![want[0].clone()]);
}

#[tokio::test]
async fn zone_without_area_is_listed_orphan() {
    let (hub, _rx) = hub(true);
    hub.ingest_message(
        "nidoo/climate/zone/attic/settings",
        br#"{"state":"active"}"#,
        true,
    );
    let view = hub.climate_view().unwrap();
    assert_eq!(view.zones.len(), 1);
    assert!(view.zones[0].orphan);
    assert_eq!(view.zones[0].zone, "attic");
}

#[tokio::test]
async fn set_zone_publishes_retained_and_applies() {
    let (hub, mut rx) = hub(true);
    feed_daikin(&hub, 0, "living").await;
    drain(&mut rx);
    let mut events = hub.subscribe();
    hub.set_climate_zone("living", active()).unwrap();
    let published = drain(&mut rx);
    assert_eq!(published.len(), 1);
    assert_eq!(published[0].topic, "nidoo/climate/zone/living/settings");
    assert!(published[0].retain);
    let view = hub.climate_view().unwrap();
    assert_eq!(view.zones[0].settings.state, ZoneState::Active);
    assert!(matches!(
        events.recv().await.unwrap(),
        WsEvent::ClimateChanged { zone: Some(z) } if z == "living"
    ));
}

#[tokio::test]
async fn set_house_publishes_retained_and_applies() {
    let (hub, mut rx) = hub(true);
    let house = nidoo_climate::doc::HouseSettings {
        heat_below: 17.0,
        ..Default::default()
    };
    hub.set_climate_house(house).unwrap();
    let published = drain(&mut rx);
    assert_eq!(published[0].topic, "nidoo/climate/house");
    assert!(published[0].retain);
    assert_eq!(hub.climate_view().unwrap().house.heat_below, 17.0);
    let bad = nidoo_climate::doc::HouseSettings {
        frost: 40.0,
        ..Default::default()
    };
    assert!(matches!(
        hub.set_climate_house(bad),
        Err(HubError::Rejected(_))
    ));
}

#[tokio::test]
async fn set_zone_rejects_invalid_rules() {
    let (hub, mut rx) = hub(true);
    feed_daikin(&hub, 0, "living").await;
    drain(&mut rx);
    let block = |from: u16, to: u16| Block {
        days: vec![Day::Mon],
        from: HhMm(from),
        to: HhMm(to),
        setpoint: Setpoint::Temp(20.0),
        evidence: None,
    };
    let overlapping = ZoneSettings {
        rules: Rules {
            heat: Some(ModeRules {
                base: Setpoint::Temp(18.0),
                blocks: vec![block(60, 300), block(120, 400)],
            }),
            ..Rules::default()
        },
        ..active()
    };
    assert!(matches!(
        hub.set_climate_zone("living", overlapping),
        Err(HubError::Rejected(_))
    ));
    assert!(drain(&mut rx).is_empty());
}

#[tokio::test]
async fn set_zone_rejects_bad_ids_and_unknown_areas() {
    let (hub, mut rx) = hub(true);
    feed_daikin(&hub, 0, "living").await;
    drain(&mut rx);
    for id in ["", "a/b", "a+b", "a#", "#"] {
        assert!(
            matches!(
                hub.set_climate_zone(id, active()),
                Err(HubError::Rejected(_))
            ),
            "{id:?}"
        );
    }
    assert!(matches!(
        hub.set_climate_zone("nowhere", active()),
        Err(HubError::UnknownArea(_))
    ));
    assert!(drain(&mut rx).is_empty());
}

#[tokio::test]
async fn setting_new_rules_approves_them() {
    let (hub, mut rx) = hub(true);
    feed_daikin(&hub, 0, "living").await;
    hub.set_climate_zone("living", ZoneSettings::default())
        .unwrap();
    let rules = Rules {
        heat: Some(ModeRules {
            base: Setpoint::Temp(18.0),
            blocks: Vec::new(),
        }),
        ..Rules::default()
    };
    let view_zone = |hub: &Hub| hub.climate_view().unwrap().zones[0].settings.clone();
    assert_eq!(view_zone(&hub).state, ZoneState::Pending);
    let before = nidoo::clock::now();
    let edited = ZoneSettings {
        rules: rules.clone(),
        ..view_zone(&hub)
    };
    hub.set_climate_zone("living", edited).unwrap();
    let saved = view_zone(&hub);
    assert_eq!(
        saved.state,
        ZoneState::Active,
        "a pending zone is now active"
    );
    let approved = saved.approved_at.expect("approved");
    assert!(approved >= before);
    // The same rules again are no new approval.
    let same = ZoneSettings {
        approved_at: Some(1),
        ..saved
    };
    hub.set_climate_zone("living", same).unwrap();
    assert_eq!(view_zone(&hub).approved_at, Some(1));
    // An off zone stays off, its new rules approved.
    let off = ZoneSettings {
        state: ZoneState::Off,
        rules: Rules {
            heat: Some(ModeRules {
                base: Setpoint::Temp(19.0),
                blocks: Vec::new(),
            }),
            ..Rules::default()
        },
        approved_at: None,
        ..view_zone(&hub)
    };
    hub.set_climate_zone("living", off).unwrap();
    let saved = view_zone(&hub);
    assert_eq!(saved.state, ZoneState::Off);
    assert!(saved.approved_at.is_some());
    // Clearing the rules approves nothing.
    let cleared = ZoneSettings {
        state: ZoneState::Pending,
        rules: Rules::default(),
        approved_at: None,
        ..saved
    };
    hub.set_climate_zone("living", cleared).unwrap();
    let saved = view_zone(&hub);
    assert_eq!(saved.state, ZoneState::Pending);
    assert_eq!(saved.approved_at, None);
    drain(&mut rx);
}

#[tokio::test]
async fn an_area_id_that_cannot_be_a_topic_level_is_not_a_zone() {
    let (hub, _rx) = hub(true);
    feed_daikin(&hub, 0, "up/stairs").await;
    feed_daikin(&hub, 1, "a+b").await;
    feed_daikin(&hub, 2, "living").await;
    let zones: Vec<String> = hub
        .climate_view()
        .unwrap()
        .zones
        .into_iter()
        .map(|z| z.zone)
        .collect();
    assert_eq!(zones, vec!["living".to_owned()]);
}

// ----- the evaluation loop -----

mod engine {
    use std::collections::BTreeMap;
    use std::time::Duration;

    use chrono::TimeZone;
    use nidoo_climate::doc::{
        Decision, DeviceTarget, HouseSettings, Mode, Override, OverrideOrigin, Source,
    };
    use nidoo_climate::view::PresenceView;

    use super::*;

    /// What the simulated Daikins hold: mode, target and current temperature per unit.
    struct Sim {
        echo: bool,
        /// The broker link drops as a command reaches a daikin, before any echo.
        drop_link: bool,
        units: BTreeMap<u8, (String, f64, f64)>,
        /// Fan mode per unit, reported only for the units that have one.
        fans: BTreeMap<u8, String>,
    }

    impl Sim {
        fn new() -> Self {
            Self {
                echo: true,
                drop_link: false,
                units: BTreeMap::new(),
                fans: BTreeMap::new(),
            }
        }

        /// Sets the state of unit `n` and reports it on its state topic.
        fn set(&mut self, hub: &Hub, n: u8, mode: &str, target: f64, current: f64) {
            self.units.insert(n, (mode.to_owned(), target, current));
            self.report(hub, n);
        }

        fn report(&self, hub: &Hub, n: u8) {
            let (mode, target, current) = &self.units[&n];
            let mut payload = serde_json::json!({
                "mode": mode, "target_temperature": target, "current_temperature": current,
            });
            if let Some(fan) = self.fans.get(&n) {
                payload["fan_mode"] = serde_json::Value::from(fan.clone());
            }
            hub.ingest(
                &format!("daikin/192.0.2.15{n}/ha/state"),
                payload.to_string().as_bytes(),
            );
        }

        /// What a Daikin does with a command: applies it and reports its state.
        fn on_publish(&mut self, hub: &Hub, p: &Publish) {
            // The broker hands the push history back to the hub.
            if p.topic == "nidoo/push/history" {
                hub.ingest_message(&p.topic, p.payload.as_bytes(), false);
                return;
            }
            let Some(rest) = p.topic.strip_prefix("daikin/192.0.2.15") else {
                return;
            };
            if self.drop_link {
                hub.set_connection(ConnectionStatus::Offline {
                    reason: "test".into(),
                });
                return;
            }
            if !self.echo {
                return;
            }
            let n: u8 = rest[..1].parse().expect("unit");
            let unit = self.units.get_mut(&n).expect("known unit");
            if rest.ends_with("/mode/set") {
                unit.0 = p.payload.clone();
            } else if rest.ends_with("/temperature/set") {
                unit.1 = p.payload.parse().expect("temperature");
            } else if rest.ends_with("/fan_mode/set") {
                self.fans.insert(n, p.payload.clone());
            }
            self.report(hub, n);
        }
    }

    struct Rig {
        hub: Arc<Hub>,
        rx: mpsc::Receiver<Publish>,
        sim: Sim,
        /// The first daikin's entity id.
        id: String,
        /// "Now" of the test, real so the registry's report times look fresh.
        t0: u64,
    }

    impl Rig {
        async fn new(tz: Option<&str>) -> Rig {
            let mut cfg = Config::default();
            cfg.topics.discovery_prefixes = vec!["nidoo".to_owned()];
            cfg.climate.enabled = true;
            cfg.flows.timezone = tz.map(str::to_owned);
            let (tx, rx) = mpsc::channel(64);
            let hub = Hub::new(&cfg, tx);
            hub.set_connection(ConnectionStatus::Online);
            let id = feed_daikin(&hub, 0, "living").await;
            let mut rig = Rig {
                hub,
                rx,
                sim: Sim::new(),
                id,
                t0: nidoo::clock::now(),
            };
            rig.sim.set(&rig.hub, 0, "heat", 22.0, 20.0);
            drain(&mut rig.rx);
            rig
        }

        /// One evaluation pass at `t0 + at`; the daikins answer the commands. Returns what
        /// was published.
        async fn pass(&mut self, at: u64) -> Vec<Publish> {
            let hub = Arc::clone(&self.hub);
            let now = self.t0 + at;
            let mut task = tokio::spawn(async move { hub.evaluate_climate(now).await });
            let mut out = Vec::new();
            loop {
                tokio::select! {
                    published = self.rx.recv() => {
                        let published = published.expect("publisher open");
                        self.sim.on_publish(&self.hub, &published);
                        out.push(published);
                    }
                    done = &mut task => {
                        done.expect("pass");
                        break;
                    }
                }
            }
            for published in drain(&mut self.rx) {
                self.sim.on_publish(&self.hub, &published);
                out.push(published);
            }
            out
        }

        fn settings(&mut self, settings: ZoneSettings) {
            self.hub.set_climate_zone("living", settings).unwrap();
            drain(&mut self.rx);
        }

        fn zone(&self) -> nidoo_climate::view::ZoneView {
            self.hub
                .climate_view()
                .unwrap()
                .zones
                .into_iter()
                .find(|z| z.zone == "living")
                .expect("living zone")
        }
    }

    fn heat_rules(base: f64) -> ZoneSettings {
        ZoneSettings {
            rules: Rules {
                heat: Some(ModeRules {
                    base: Setpoint::Temp(base),
                    blocks: Vec::new(),
                }),
                ..Rules::default()
            },
            ..active()
        }
    }

    /// The commands the daikins received, as (topic suffix, payload).
    fn commands(out: &[Publish]) -> Vec<(String, String)> {
        out.iter()
            .filter_map(|p| {
                let rest = p.topic.strip_prefix("daikin/192.0.2.15")?;
                Some((rest[1..].to_owned(), p.payload.clone()))
            })
            .collect()
    }

    fn decisions_published(out: &[Publish]) -> usize {
        out.iter()
            .filter(|p| p.topic == "nidoo/climate/zone/living/decision")
            .count()
    }

    fn temperature_commands(out: &[Publish]) -> Vec<f64> {
        commands(out)
            .into_iter()
            .filter(|(topic, _)| topic == "/ha/temperature/set")
            .map(|(_, payload)| payload.parse().unwrap())
            .collect()
    }

    /// Makes the push service accept alerts, as it does once the broker has settled.
    async fn load_push(hub: &Hub) {
        hub.on_connected();
        hub.on_suback();
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    fn alert(hub: &Hub, id: &str) -> Option<nidoo_api::AlertView> {
        hub.alerts().into_iter().find(|a| a.id == id)
    }

    #[tokio::test(start_paused = true)]
    async fn pending_zone_commands_nothing() {
        let mut rig = Rig::new(None).await;
        let out = rig.pass(0).await;
        assert!(commands(&out).is_empty(), "{out:?}");
        let decision = rig.zone().decision.expect("decision published");
        assert_eq!(decision.source, Source::Idle);
        assert!(!decision.applied);
    }

    #[tokio::test(start_paused = true)]
    async fn active_zone_commands_schedule_target() {
        let mut rig = Rig::new(None).await;
        rig.settings(heat_rules(17.0));
        let out = rig.pass(0).await;
        assert_eq!(
            commands(&out),
            vec![("/ha/temperature/set".to_owned(), "17".to_owned())],
            "{out:?}"
        );
        let decision = rig.zone().decision.unwrap();
        assert_eq!(decision.source, Source::Schedule);
        assert!(decision.applied);
    }

    #[tokio::test(start_paused = true)]
    async fn a_device_in_the_wrong_mode_gets_the_mode_first() {
        let mut rig = Rig::new(None).await;
        rig.sim.set(&rig.hub, 0, "off", 22.0, 20.0);
        rig.settings(heat_rules(17.0));
        let out = rig.pass(0).await;
        assert_eq!(
            commands(&out),
            vec![
                ("/ha/mode/set".to_owned(), "heat".to_owned()),
                ("/ha/temperature/set".to_owned(), "17".to_owned()),
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn frost_commands_pending_zone_then_restores() {
        let mut rig = Rig::new(None).await;
        load_push(&rig.hub).await;
        // A heater switched off in a freezing room.
        rig.sim.set(&rig.hub, 0, "off", 10.0, 5.0);
        let out = rig.pass(0).await;
        assert_eq!(
            commands(&out),
            vec![("/ha/mode/set".to_owned(), "heat".to_owned())],
            "{out:?}"
        );
        let frost = alert(&rig.hub, "climate-safety-living-frost").expect("alert raised");
        assert_eq!(serde_json::to_value(frost.severity).unwrap(), "critical");
        assert!(!frost.resolved);
        assert!(
            rig.zone().decision.unwrap().applied,
            "a sent safety command"
        );

        // Still frosty: nothing more to say.
        let out = rig.pass(61).await;
        assert!(commands(&out).is_empty(), "{out:?}");

        // The room warms up: what was found on the device is put back.
        rig.sim.set(&rig.hub, 0, "heat", 10.0, 18.0);
        let out = rig.pass(122).await;
        assert_eq!(
            commands(&out),
            vec![("/ha/mode/set".to_owned(), "off".to_owned())],
            "{out:?}"
        );
        // Restored once.
        let out = rig.pass(183).await;
        assert!(commands(&out).is_empty(), "{out:?}");
        assert!(!rig.zone().decision.unwrap().applied);

        // The alert resolves only after its cause has been gone for ten minutes.
        let hub = Arc::clone(&rig.hub);
        let frost_alert = move || alert(&hub, "climate-safety-living-frost").unwrap();
        assert!(!frost_alert().resolved);
        rig.pass(122 + 599).await;
        assert!(!frost_alert().resolved, "not before ten minutes");
        rig.pass(122 + 601).await;
        assert!(frost_alert().resolved);
    }

    #[tokio::test(start_paused = true)]
    async fn no_command_when_already_at_target() {
        let mut rig = Rig::new(None).await;
        rig.sim.set(&rig.hub, 0, "heat", 17.0, 20.0);
        rig.settings(heat_rules(17.0));
        let out = rig.pass(0).await;
        assert!(commands(&out).is_empty(), "{out:?}");
        assert!(
            rig.zone().decision.unwrap().applied,
            "nothing needed sending"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limited_to_one_per_minute() {
        let mut rig = Rig::new(None).await;
        rig.settings(heat_rules(17.0));
        assert_eq!(temperature_commands(&rig.pass(0).await), vec![17.0]);
        rig.settings(heat_rules(19.0));
        let out = rig.pass(30).await;
        assert!(commands(&out).is_empty(), "{out:?}");
        assert!(!rig.zone().decision.unwrap().applied, "wanted but not sent");
        assert_eq!(temperature_commands(&rig.pass(61).await), vec![19.0]);
    }

    #[tokio::test(start_paused = true)]
    async fn hand_change_creates_device_override() {
        let mut rig = Rig::new(None).await;
        rig.settings(heat_rules(17.0));
        rig.pass(0).await;
        // Ten minutes later someone turns it up.
        rig.sim.set(&rig.hub, 0, "heat", 22.0, 20.0);
        rig.hub.observe_climate(&rig.id, rig.t0 + 600);
        let overrides = rig.zone().overrides;
        assert_eq!(
            overrides,
            vec![Override {
                entity: Some(rig.id.clone()),
                setpoint: Some(Setpoint::Temp(22.0)),
                mode: Some(Mode::Heat),
                until: rig.t0 + 600 + 240 * 60,
                origin: OverrideOrigin::Hand,
            }]
        );
        // The device is held: the engine does not fight it.
        let out = rig.pass(661).await;
        assert!(commands(&out).is_empty(), "{out:?}");
        let decision = rig.zone().decision.unwrap();
        assert_eq!(decision.source, Source::Schedule);
        assert!(decision.entities[&rig.id].held);
        // More reports of the same state are not new hand changes: the hold does not slide.
        rig.hub.observe_climate(&rig.id, rig.t0 + 1200);
        assert_eq!(rig.zone().overrides, overrides);
    }

    #[tokio::test(start_paused = true)]
    async fn a_hand_change_while_off_is_not_held_once_back_on() {
        let mut rig = Rig::new(None).await;
        rig.settings(heat_rules(17.0));
        rig.pass(0).await;
        rig.settings(ZoneSettings {
            state: ZoneState::Off,
            ..heat_rules(17.0)
        });
        rig.pass(61).await;
        // Turned up by hand while the zone is off.
        rig.sim.set(&rig.hub, 0, "heat", 22.0, 20.0);
        rig.hub.observe_climate(&rig.id, rig.t0 + 600);
        // Back on: the device's state is not taken for a change against the engine.
        rig.settings(heat_rules(17.0));
        rig.hub.observe_climate(&rig.id, rig.t0 + 700);
        let out = rig.pass(761).await;
        assert!(rig.zone().overrides.is_empty());
        assert_eq!(temperature_commands(&out), vec![17.0], "{out:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn echo_within_30s_is_not_hand_change() {
        let mut rig = Rig::new(None).await;
        rig.settings(heat_rules(17.0));
        rig.pass(0).await;
        // The device still reports an in-between value right after the command.
        rig.sim.set(&rig.hub, 0, "heat", 20.0, 20.0);
        rig.hub.observe_climate(&rig.id, rig.t0 + 10);
        assert!(rig.zone().overrides.is_empty());
        // The same report well after the window is someone's doing.
        rig.hub.observe_climate(&rig.id, rig.t0 + 40);
        assert_eq!(rig.zone().overrides.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn pending_zone_overrides_nothing() {
        let mut rig = Rig::new(None).await;
        rig.sim.set(&rig.hub, 0, "heat", 24.0, 20.0);
        rig.hub.observe_climate(&rig.id, rig.t0 + 600);
        assert!(rig.zone().overrides.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn drift_after_reconnect_is_reasserted() {
        let mut rig = Rig::new(None).await;
        rig.settings(heat_rules(17.0));
        rig.pass(0).await;
        let offline = |hub: &Hub, state: &[u8]| {
            hub.ingest("daikin/bridge/availability", state);
        };
        offline(&rig.hub, b"offline");
        rig.hub.observe_climate(&rig.id, rig.t0 + 300);
        offline(&rig.hub, b"online");
        rig.hub.observe_climate(&rig.id, rig.t0 + 400);
        // It comes back with its own default.
        rig.sim.set(&rig.hub, 0, "heat", 24.0, 20.0);
        rig.hub.observe_climate(&rig.id, rig.t0 + 430);
        assert!(
            rig.zone().overrides.is_empty(),
            "drift is not a hand change"
        );
        let out = rig.pass(461).await;
        assert_eq!(temperature_commands(&out), vec![17.0], "{out:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn restart_seeds_ledger_from_retained_decision() {
        let mut rig = Rig::new(None).await;
        rig.sim.set(&rig.hub, 0, "heat", 22.0, 20.0);
        let settings = heat_rules(20.0);
        rig.hub.ingest_message(
            "nidoo/climate/zone/living/settings",
            serde_json::to_string(&settings).unwrap().as_bytes(),
            true,
        );
        // The decision the engine retained before the restart: 20 °C, applied.
        let decision = Decision {
            eco_restore: BTreeMap::new(),
            adjust: None,
            zone: "living".into(),
            mode: Some(Mode::Heat),
            setpoint: Some(Setpoint::Temp(20.0)),
            source: Source::Schedule,
            cause: None,
            reason: "Schedule base: 20.0 °C".into(),
            reading: Some(19.0),
            reading_age_s: Some(30),
            outdoor_mean: None,
            home: true,
            holiday: false,
            applied: true,
            at: rig.t0 - 3600,
            entities: BTreeMap::from([(
                rig.id.clone(),
                DeviceTarget {
                    setpoint: Some(20.0),
                    mode: "heat".into(),
                    held: false,
                },
            )]),
        };
        rig.hub.ingest_message(
            "nidoo/climate/zone/living/decision",
            serde_json::to_string(&decision).unwrap().as_bytes(),
            true,
        );
        // It changed while the hub was down: a hand change, not drift (nothing came back
        // from unavailable).
        let out = rig.pass(0).await;
        let overrides = rig.zone().overrides;
        assert_eq!(overrides.len(), 1, "{overrides:?}");
        assert_eq!(overrides[0].entity.as_deref(), Some(rig.id.as_str()));
        assert_eq!(overrides[0].setpoint, Some(Setpoint::Temp(22.0)));
        assert!(commands(&out).is_empty(), "{out:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn an_older_retained_decision_never_overwrites_a_newer_one() {
        let mut rig = Rig::new(None).await;
        let second = feed_daikin(&rig.hub, 1, "other").await;
        rig.sim.set(&rig.hub, 1, "heat", 22.0, 20.0);
        let both = |first: f64, at: u64| {
            let mut entities = BTreeMap::new();
            for id in [&rig.id, &second] {
                entities.insert(
                    id.clone(),
                    DeviceTarget {
                        setpoint: Some(first),
                        mode: "heat".into(),
                        held: false,
                    },
                );
            }
            (entities, at)
        };
        // The same entity listed by two zones' retained decisions (it moved): the newer wins.
        let publish = |zone: &str, first: f64, at: u64| {
            let (entities, at) = both(first, at);
            let decision = Decision {
                eco_restore: BTreeMap::new(),
                adjust: None,
                zone: zone.into(),
                mode: Some(Mode::Heat),
                setpoint: None,
                source: Source::Schedule,
                cause: None,
                reason: String::new(),
                reading: None,
                reading_age_s: None,
                outdoor_mean: None,
                home: true,
                holiday: false,
                applied: true,
                at,
                entities,
            };
            rig.hub.ingest_message(
                &format!("nidoo/climate/zone/{zone}/decision"),
                serde_json::to_string(&decision).unwrap().as_bytes(),
                true,
            );
        };
        publish("living", 22.0, rig.t0 - 100);
        publish("other", 18.0, rig.t0 - 7200);
        for zone in ["living", "other"] {
            rig.hub.set_climate_zone(zone, heat_rules(22.0)).unwrap();
        }
        drain(&mut rig.rx);
        rig.pass(0).await;
        // Both devices are at 22, which is what the newest decision says.
        for zone in rig.hub.climate_view().unwrap().zones {
            assert!(
                zone.overrides.is_empty(),
                "{}: {:?}",
                zone.zone,
                zone.overrides
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn three_failures_pause_and_alert() {
        let mut rig = Rig::new(None).await;
        load_push(&rig.hub).await;
        rig.settings(heat_rules(17.0));
        rig.sim.echo = false;
        let alert_id = format!("climate-command-{}", rig.id);
        for at in [0, 61, 122] {
            let out = rig.pass(at).await;
            assert_eq!(temperature_commands(&out), vec![17.0], "at {at}: {out:?}");
        }
        let raised = alert(&rig.hub, &alert_id)
            .unwrap_or_else(|| panic!("alert after 3 failures: {:?}", rig.hub.alerts()));
        assert!(!raised.resolved);
        // Paused for 15 minutes.
        let out = rig.pass(183).await;
        assert!(commands(&out).is_empty(), "{out:?}");
        // Tried again once the pause is over; the device answers this time.
        rig.sim.echo = true;
        let out = rig.pass(122 + 15 * 60).await;
        assert_eq!(temperature_commands(&out), vec![17.0], "{out:?}");
        assert!(alert(&rig.hub, &alert_id).unwrap().resolved);
    }

    #[tokio::test(start_paused = true)]
    async fn failed_command_is_not_recorded() {
        let mut rig = Rig::new(None).await;
        load_push(&rig.hub).await;
        // Already at target: acknowledged (the Matches path), nothing sent.
        rig.sim.set(&rig.hub, 0, "heat", 17.0, 20.0);
        rig.settings(heat_rules(17.0));
        assert!(commands(&rig.pass(0).await).is_empty());
        // The device stops answering: three refused commands are counted (the alert) and none
        // is written to the ledger.
        rig.settings(heat_rules(19.0));
        rig.sim.echo = false;
        for at in [61, 122, 183] {
            assert_eq!(temperature_commands(&rig.pass(at).await), vec![19.0]);
        }
        let alert_id = format!("climate-command-{}", rig.id);
        assert!(
            alert(&rig.hub, &alert_id).is_some(),
            "failures were counted"
        );
        // The device still reports what it had: that matches the ledger, so no hand change.
        rig.hub.observe_climate(&rig.id, rig.t0 + 400);
        assert!(rig.zone().overrides.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_is_sent_or_counted_without_the_broker() {
        let mut rig = Rig::new(None).await;
        load_push(&rig.hub).await;
        rig.settings(heat_rules(17.0));
        rig.hub.set_connection(ConnectionStatus::Offline {
            reason: "test".into(),
        });
        for at in [0, 61, 122, 183] {
            let out = rig.pass(at).await;
            assert!(commands(&out).is_empty(), "{out:?}");
        }
        assert!(alert(&rig.hub, &format!("climate-command-{}", rig.id)).is_none());
        // Back online: sent at once, not paused.
        rig.hub.set_connection(ConnectionStatus::Online);
        assert_eq!(temperature_commands(&rig.pass(184).await), vec![17.0]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_link_lost_during_the_confirm_wait_is_not_counted() {
        let mut rig = Rig::new(None).await;
        load_push(&rig.hub).await;
        rig.settings(heat_rules(17.0));
        rig.sim.drop_link = true;
        for at in [0, 61, 122] {
            rig.hub.set_connection(ConnectionStatus::Online);
            assert_eq!(
                temperature_commands(&rig.pass(at).await),
                vec![17.0],
                "at {at}"
            );
        }
        assert!(alert(&rig.hub, &format!("climate-command-{}", rig.id)).is_none());
        // Not paused: sent again as soon as the link is back.
        rig.sim.drop_link = false;
        rig.hub.set_connection(ConnectionStatus::Online);
        assert_eq!(temperature_commands(&rig.pass(183).await), vec![17.0]);
    }

    #[tokio::test(start_paused = true)]
    async fn hand_change_on_a_device_already_at_target() {
        let mut rig = Rig::new(None).await;
        rig.sim.set(&rig.hub, 0, "heat", 17.0, 20.0);
        rig.settings(heat_rules(17.0));
        assert!(commands(&rig.pass(0).await).is_empty());
        rig.sim.set(&rig.hub, 0, "heat", 22.0, 20.0);
        rig.hub.observe_climate(&rig.id, rig.t0 + 600);
        let overrides = rig.zone().overrides;
        assert_eq!(overrides.len(), 1, "{overrides:?}");
        assert_eq!(overrides[0].setpoint, Some(Setpoint::Temp(22.0)));
        let out = rig.pass(602).await;
        assert!(commands(&out).is_empty(), "never reverted: {out:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn restart_with_an_unapplied_decision_still_seeds_the_ledger() {
        let mut rig = Rig::new(None).await;
        rig.sim.set(&rig.hub, 0, "heat", 22.0, 20.0);
        rig.hub.ingest_message(
            "nidoo/climate/zone/living/settings",
            serde_json::to_string(&heat_rules(20.0)).unwrap().as_bytes(),
            true,
        );
        let decision = Decision {
            eco_restore: BTreeMap::new(),
            adjust: None,
            zone: "living".into(),
            mode: Some(Mode::Heat),
            setpoint: Some(Setpoint::Temp(20.0)),
            source: Source::Schedule,
            cause: None,
            reason: "Schedule base: 20.0 °C".into(),
            reading: None,
            reading_age_s: None,
            outdoor_mean: None,
            home: true,
            holiday: false,
            applied: false,
            at: rig.t0 - 3600,
            entities: BTreeMap::from([(
                rig.id.clone(),
                DeviceTarget {
                    setpoint: Some(20.0),
                    mode: "heat".into(),
                    held: false,
                },
            )]),
        };
        rig.hub.ingest_message(
            "nidoo/climate/zone/living/decision",
            serde_json::to_string(&decision).unwrap().as_bytes(),
            true,
        );
        let out = rig.pass(0).await;
        assert_eq!(rig.zone().overrides.len(), 1);
        assert!(commands(&out).is_empty(), "{out:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn after_the_ledger_forgets_the_next_pass_acknowledges_again() {
        let mut rig = Rig::new(None).await;
        rig.sim.set(&rig.hub, 0, "heat", 17.0, 20.0);
        rig.settings(heat_rules(17.0));
        rig.pass(0).await;
        // The device leaves the house (the ledger forgets it) and comes back.
        rig.hub
            .ingest("nidoo/climate/daikin_192_0_2_150/climate/config", b"");
        rig.pass(61).await;
        feed_daikin(&rig.hub, 0, "living").await;
        rig.sim.set(&rig.hub, 0, "heat", 17.0, 20.0);
        assert!(commands(&rig.pass(122).await).is_empty());
        rig.sim.set(&rig.hub, 0, "heat", 24.0, 20.0);
        rig.hub.observe_climate(&rig.id, rig.t0 + 700);
        assert_eq!(rig.zone().overrides.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn off_mode_ignores_the_reported_setpoint() {
        let mut rig = Rig::new(None).await;
        let off = ZoneSettings {
            rules: Rules {
                heat: Some(ModeRules {
                    base: Setpoint::Off,
                    blocks: Vec::new(),
                }),
                ..Rules::default()
            },
            ..active()
        };
        rig.settings(off);
        let out = rig.pass(0).await;
        assert_eq!(
            commands(&out),
            vec![("/ha/mode/set".to_owned(), "off".to_owned())]
        );
        // The unit reports a target again, along with a new room temperature.
        rig.sim.set(&rig.hub, 0, "off", 22.0, 19.0);
        rig.hub.observe_climate(&rig.id, rig.t0 + 600);
        assert!(
            rig.zone().overrides.is_empty(),
            "{:?}",
            rig.zone().overrides
        );
    }

    #[tokio::test(start_paused = true)]
    async fn rapid_presses_keep_the_last_value() {
        let mut rig = Rig::new(None).await;
        rig.settings(heat_rules(17.0));
        rig.pass(0).await;
        rig.sim.set(&rig.hub, 0, "heat", 22.0, 20.0);
        rig.hub.observe_climate(&rig.id, rig.t0 + 600);
        rig.sim.set(&rig.hub, 0, "heat", 23.0, 20.0);
        rig.hub.observe_climate(&rig.id, rig.t0 + 605);
        let overrides = rig.zone().overrides;
        assert_eq!(overrides.len(), 1, "{overrides:?}");
        assert_eq!(overrides[0].setpoint, Some(Setpoint::Temp(23.0)));
    }

    #[tokio::test(start_paused = true)]
    async fn frost_hovering_at_the_limit_does_not_restore_until_back_inside() {
        let mut rig = Rig::new(None).await;
        // A heater switched off in a cold room: frost turns it on, clearing turns it off.
        rig.sim.set(&rig.hub, 0, "off", 10.0, 6.9);
        let out = rig.pass(0).await;
        assert_eq!(
            commands(&out),
            vec![("/ha/mode/set".to_owned(), "heat".to_owned())]
        );
        for (at, reading) in [(61, 7.0), (122, 6.9), (183, 7.5), (244, 8.9)] {
            rig.sim.set(&rig.hub, 0, "heat", 10.0, reading);
            let out = rig.pass(at).await;
            assert!(commands(&out).is_empty(), "{reading}: {out:?}");
            assert_eq!(rig.zone().decision.unwrap().source, Source::Safety);
        }
        // Frost holds until the room reaches the frost target, frost + 2.
        rig.sim.set(&rig.hub, 0, "heat", 10.0, 9.0);
        let out = rig.pass(305).await;
        assert_eq!(
            commands(&out),
            vec![("/ha/mode/set".to_owned(), "off".to_owned())],
            "{out:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_hand_change_without_its_event_is_caught_by_the_next_pass() {
        let mut rig = Rig::new(None).await;
        rig.settings(heat_rules(17.0));
        assert_eq!(temperature_commands(&rig.pass(0).await), vec![17.0]);
        // Someone turns it up; the event is lost (no observe), only the registry knows.
        rig.sim.set(&rig.hub, 0, "heat", 22.0, 20.0);
        let out = rig.pass(600).await;
        assert!(commands(&out).is_empty(), "commanded back: {out:?}");
        let overrides = rig.zone().overrides;
        assert_eq!(overrides.len(), 1, "{overrides:?}");
        assert_eq!(overrides[0].entity.as_deref(), Some(rig.id.as_str()));
        assert_eq!(overrides[0].setpoint, Some(Setpoint::Temp(22.0)));
        assert_eq!(overrides[0].origin, OverrideOrigin::Hand);
        assert!(rig.zone().decision.unwrap().entities[&rig.id].held);
    }

    #[tokio::test(start_paused = true)]
    async fn an_explicit_list_claims_its_devices() {
        let mut rig = Rig::new(None).await;
        let second = feed_daikin(&rig.hub, 1, "living").await;
        feed_daikin(&rig.hub, 2, "other").await;
        rig.sim.set(&rig.hub, 1, "heat", 22.0, 20.0);
        rig.sim.set(&rig.hub, 2, "heat", 22.0, 20.0);
        // `other` lists the second living device: it leaves living's default list.
        let claimed = ZoneSettings {
            entities: Some(vec![second.clone()]),
            ..heat_rules(17.0)
        };
        rig.hub.set_climate_zone("other", claimed).unwrap();
        rig.settings(heat_rules(17.0));
        let view = rig.hub.climate_view().unwrap();
        let entities = |zone: &str| {
            view.zones
                .iter()
                .find(|z| z.zone == zone)
                .map(|z| z.entities.clone())
                .unwrap()
        };
        assert_eq!(entities("living"), vec![rig.id.clone()]);
        assert_eq!(entities("other"), vec![second.clone()]);
        // A second explicit claim on it is refused.
        let again = ZoneSettings {
            entities: Some(vec![second.clone()]),
            ..heat_rules(17.0)
        };
        assert!(matches!(
            rig.hub.set_climate_zone("living", again),
            Err(HubError::Rejected(m)) if m.contains("other")
        ));
        // Each device is commanded once, by its own zone, and a hand change on the claimed one
        // is held in the zone that claims it.
        rig.pass(0).await;
        let decision = |zone: &str| {
            rig.hub
                .climate_view()
                .unwrap()
                .zones
                .into_iter()
                .find(|z| z.zone == zone)
                .and_then(|z| z.decision)
                .unwrap()
        };
        assert!(!decision("living").entities.contains_key(&second));
        assert!(decision("other").entities.contains_key(&second));
        rig.sim.set(&rig.hub, 1, "heat", 23.0, 20.0);
        rig.hub.observe_climate(&second, rig.t0 + 600);
        let held_in = |zone: &str| {
            rig.hub
                .climate_view()
                .unwrap()
                .zones
                .into_iter()
                .find(|z| z.zone == zone)
                .unwrap()
                .overrides
                .len()
        };
        assert_eq!(held_in("other"), 1);
        assert_eq!(held_in("living"), 0);
    }

    /// The push history the hub last published in `out`.
    fn last_history(out: &[Publish]) -> Option<String> {
        out.iter()
            .rev()
            .find(|p| p.topic == "nidoo/push/history")
            .map(|p| p.payload.clone())
    }

    #[tokio::test(start_paused = true)]
    async fn a_raised_safety_alert_survives_a_restart() {
        let id = "climate-safety-living-frost";
        let house = HouseSettings {
            stale_after_min: 24 * 60,
            ..HouseSettings::default()
        };
        let mut before = Rig::new(None).await;
        load_push(&before.hub).await;
        before.hub.set_climate_house(house.clone()).unwrap();
        drain(&mut before.rx);
        before.sim.set(&before.hub, 0, "off", 10.0, 5.0);
        let history = last_history(&before.pass(0).await).expect("alert pushed");
        assert!(!alert(&before.hub, id).unwrap().resolved);
        // The hub restarts and finds the alert in the retained history.
        let mut rig = Rig::new(None).await;
        rig.hub
            .ingest_message("nidoo/push/history", history.as_bytes(), true);
        load_push(&rig.hub).await;
        rig.hub.set_climate_house(house).unwrap();
        drain(&mut rig.rx);
        rig.sim.set(&rig.hub, 0, "off", 10.0, 5.0);
        let out = rig.pass(0).await;
        assert_eq!(rig.zone().decision.unwrap().source, Source::Safety);
        assert_eq!(last_history(&out), None, "pushed again: {out:?}");
        // Warm again: the alert raised before the restart still resolves.
        rig.sim.set(&rig.hub, 0, "heat", 10.0, 18.0);
        rig.pass(61).await;
        rig.pass(61 + 601).await;
        assert!(alert(&rig.hub, id).unwrap().resolved);
    }

    #[tokio::test(start_paused = true)]
    async fn a_resolved_frost_alert_is_not_raised_again_for_six_hours() {
        let mut rig = Rig::new(None).await;
        load_push(&rig.hub).await;
        // Long enough that the far passes still see a fresh reading.
        let house = HouseSettings {
            stale_after_min: 24 * 60,
            ..HouseSettings::default()
        };
        rig.hub.set_climate_house(house).unwrap();
        drain(&mut rig.rx);
        let id = "climate-safety-living-frost";
        rig.sim.set(&rig.hub, 0, "off", 10.0, 5.0);
        assert!(!commands(&rig.pass(0).await).is_empty());
        assert!(!alert(&rig.hub, id).unwrap().resolved);
        // Warm again; the alert resolves ten minutes later.
        rig.sim.set(&rig.hub, 0, "heat", 10.0, 18.0);
        rig.pass(61).await;
        rig.pass(61 + 601).await;
        let first = alert(&rig.hub, id).unwrap();
        assert!(first.resolved);
        // Frost again within six hours: safety acts, but no new push.
        rig.sim.set(&rig.hub, 0, "off", 10.0, 5.0);
        let out = rig.pass(3600).await;
        assert_eq!(
            commands(&out),
            vec![("/ha/mode/set".to_owned(), "heat".to_owned())],
            "{out:?}"
        );
        assert_eq!(rig.zone().decision.unwrap().source, Source::Safety);
        let alerts: Vec<_> = rig
            .hub
            .alerts()
            .into_iter()
            .filter(|a| a.id == id)
            .collect();
        assert_eq!(alerts.len(), 1, "{alerts:?}");
        assert!(alerts[0].resolved, "not raised again");
        // Six hours after it resolved, a frost that is still there is told again.
        rig.sim.report(&rig.hub, 0);
        rig.pass(61 + 601 + 6 * 3600).await;
        assert!(!alert(&rig.hub, id).unwrap().resolved);
    }

    #[tokio::test(start_paused = true)]
    async fn restoring_an_off_device_sends_no_temperature() {
        let mut rig = Rig::new(None).await;
        // A heater switched off at 20 °C in a freezing room: frost heats it at the frost
        // target (the unit's minimum, 10 °C), not at the 20 °C it remembers.
        rig.sim.set(&rig.hub, 0, "off", 20.0, 5.0);
        let out = rig.pass(0).await;
        assert_eq!(
            commands(&out),
            vec![
                ("/ha/mode/set".to_owned(), "heat".to_owned()),
                ("/ha/temperature/set".to_owned(), "10".to_owned()),
            ],
            "{out:?}"
        );
        // Warm again: it is put back off, and an off device gets no temperature.
        rig.sim.set(&rig.hub, 0, "heat", 10.0, 18.0);
        let out = rig.pass(61).await;
        assert_eq!(
            commands(&out),
            vec![("/ha/mode/set".to_owned(), "off".to_owned())],
            "{out:?}"
        );
        assert!(commands(&rig.pass(122).await).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_dropped_pass_does_not_leave_a_device_in_flight() {
        let mut rig = Rig::new(None).await;
        rig.settings(heat_rules(17.0));
        rig.pass(0).await;
        rig.settings(heat_rules(19.0));
        rig.sim.echo = false;
        let hub = Arc::clone(&rig.hub);
        let now = rig.t0 + 61;
        let task = tokio::spawn(async move { hub.evaluate_climate(now).await });
        loop {
            let published = rig.rx.recv().await.expect("publisher open");
            if published.topic.ends_with("/ha/temperature/set") {
                break;
            }
        }
        task.abort();
        let _ = task.await;
        // The device is turned up by hand; the abandoned command must not hide it.
        rig.sim.set(&rig.hub, 0, "heat", 24.0, 20.0);
        rig.hub.observe_climate(&rig.id, rig.t0 + 700);
        assert_eq!(rig.zone().overrides.len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_dropped_pass_command_the_device_later_echoes_is_not_a_hand_change() {
        let mut rig = Rig::new(None).await;
        rig.settings(heat_rules(17.0));
        rig.pass(0).await;
        rig.settings(heat_rules(19.0));
        rig.sim.echo = false;
        let hub = Arc::clone(&rig.hub);
        let now = rig.t0 + 61;
        let task = tokio::spawn(async move { hub.evaluate_climate(now).await });
        loop {
            let published = rig.rx.recv().await.expect("publisher open");
            if published.topic.ends_with("/ha/temperature/set") {
                break;
            }
        }
        task.abort();
        let _ = task.await;
        // The device applies the abandoned command after the pass is gone.
        rig.sim.set(&rig.hub, 0, "heat", 19.0, 20.0);
        rig.hub.observe_climate(&rig.id, rig.t0 + 700);
        assert!(rig.zone().overrides.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn decision_published_only_when_outcome_changes() {
        let mut rig = Rig::new(None).await;
        rig.settings(heat_rules(17.0));
        let out = rig.pass(0).await;
        assert_eq!(decisions_published(&out), 1);
        // Same outcome, the reading moved: nothing to publish.
        rig.sim.set(&rig.hub, 0, "heat", 17.0, 19.5);
        let out = rig.pass(61).await;
        assert_eq!(decisions_published(&out), 0, "{out:?}");
        rig.settings(heat_rules(18.0));
        let out = rig.pass(122).await;
        assert_eq!(decisions_published(&out), 1, "{out:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn zone_drops_removed_entity() {
        let mut rig = Rig::new(None).await;
        let second = feed_daikin(&rig.hub, 1, "living").await;
        rig.sim.set(&rig.hub, 1, "heat", 22.0, 20.0);
        drain(&mut rig.rx);
        rig.settings(heat_rules(17.0));
        rig.pass(0).await;
        rig.sim.set(&rig.hub, 0, "heat", 24.0, 20.0);
        rig.hub.observe_climate(&rig.id, rig.t0 + 600);
        assert_eq!(rig.zone().overrides.len(), 1);

        // The held device disappears from the house; the other keeps following the rules.
        rig.hub
            .ingest("nidoo/climate/daikin_192_0_2_150/climate/config", b"");
        assert!(rig.hub.snapshot().entities.iter().all(|e| e.id != rig.id));
        rig.sim.report(&rig.hub, 1);
        rig.settings(heat_rules(18.0));
        let out = rig.pass(700).await;
        assert!(rig.zone().overrides.is_empty(), "its override is dropped");
        assert!(
            out.iter().all(|p| !p.topic.contains("150")),
            "never commanded: {out:?}"
        );
        assert_eq!(temperature_commands(&out), vec![18.0]);
        assert!(rig.zone().decision.unwrap().entities.contains_key(&second));
    }

    #[tokio::test(start_paused = true)]
    async fn hold_until_across_dst() {
        let mut rig = Rig::new(Some("Europe/Paris")).await;
        // Sunday 2026-03-29: 02:30 does not exist in Paris, the clocks jump to 03:00.
        let local = |h, m| {
            chrono_tz::Europe::Paris
                .with_ymd_and_hms(2026, 3, 29, h, m, 0)
                .earliest()
        };
        let settings = ZoneSettings {
            rules: Rules {
                heat: Some(ModeRules {
                    base: Setpoint::Temp(17.0),
                    blocks: vec![Block {
                        days: vec![Day::Sun],
                        from: HhMm(150),
                        to: HhMm(1380),
                        setpoint: Setpoint::Temp(18.0),
                        evidence: None,
                    }],
                }),
                ..Rules::default()
            },
            ..active()
        };
        rig.settings(settings);
        let start = local(0, 50).unwrap().timestamp() as u64;
        rig.t0 = start;
        rig.pass(0).await;
        rig.sim.set(&rig.hub, 0, "heat", 22.0, 20.0);
        rig.hub.observe_climate(&rig.id, start + 600);
        let overrides = rig.zone().overrides;
        assert_eq!(overrides.len(), 1);
        // The next change is "02:30", which is 03:00 once the clocks have moved.
        let three = chrono::Utc
            .with_ymd_and_hms(2026, 3, 29, 1, 0, 0)
            .unwrap()
            .timestamp() as u64;
        assert_eq!(overrides[0].until, three);
    }

    #[tokio::test(start_paused = true)]
    async fn away_after_grace() {
        let mut rig = Rig::new(None).await;
        rig.hub.ingest(
            "nidoo/switch/presence/home/config",
            br#"{"name":"Home","unique_id":"presence_home","state_topic":"t/home","command_topic":"t/home/set"}"#,
        );
        let switch = rig
            .hub
            .snapshot()
            .entities
            .into_iter()
            .find(|e| e.id.contains("presence_home"))
            .expect("presence switch")
            .id;
        rig.hub.ingest("t/home", b"ON");
        rig.hub
            .set_climate_house(HouseSettings {
                presence: vec![switch],
                ..HouseSettings::default()
            })
            .unwrap();
        rig.settings(heat_rules(17.0));
        assert_eq!(temperature_commands(&rig.pass(0).await), vec![17.0]);

        // Nobody home for ten minutes: still the schedule.
        rig.hub.ingest("t/home", b"OFF");
        let out = rig.pass(600).await;
        assert!(commands(&out).is_empty(), "{out:?}");
        assert_eq!(rig.zone().decision.unwrap().source, Source::Schedule);
        // Sixteen minutes: away.
        let out = rig.pass(960).await;
        assert_eq!(temperature_commands(&out), vec![16.0], "{out:?}");
        assert_eq!(rig.zone().decision.unwrap().source, Source::Away);
    }

    fn eco_for(rig: &Rig, secs: u64) -> nidoo_climate::doc::RoomEco {
        nidoo_climate::doc::RoomEco {
            until: rig.t0 + secs,
            set_at: rig.t0,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn room_eco_lowers_then_ends() {
        let mut rig = Rig::new(None).await;
        rig.settings(heat_rules(19.0));
        assert_eq!(temperature_commands(&rig.pass(0).await), vec![19.0]);
        let eco = eco_for(&rig, 600);
        rig.hub
            .set_climate_eco("living", eco.clone())
            .await
            .unwrap();
        drain(&mut rig.rx);
        assert_eq!(rig.zone().eco, Some(eco));
        assert_eq!(temperature_commands(&rig.pass(60).await), vec![16.0]);
        assert_eq!(rig.zone().decision.unwrap().source, Source::Eco);

        // Past its end: back on the schedule, and the document is gone.
        let out = rig.pass(700).await;
        assert_eq!(temperature_commands(&out), vec![19.0]);
        assert_eq!(rig.zone().decision.unwrap().source, Source::Schedule);
        assert_eq!(rig.zone().eco, None, "expired_room_eco_is_deleted");
        assert!(
            out.iter()
                .any(|p| p.topic == "nidoo/climate/zone/living/eco"
                    && p.payload.is_empty()
                    && p.retain),
            "{out:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn room_eco_refused_on_off_zone_and_in_the_past() {
        let mut rig = Rig::new(None).await;
        rig.settings(heat_rules(19.0));
        let past = nidoo_climate::doc::RoomEco {
            until: nidoo::clock::now().saturating_sub(10),
            set_at: 0,
        };
        assert!(matches!(
            rig.hub.set_climate_eco("living", past).await,
            Err(HubError::Rejected(_))
        ));
        rig.settings(ZoneSettings {
            state: ZoneState::Off,
            ..heat_rules(19.0)
        });
        let eco = eco_for(&rig, 3600);
        assert!(matches!(
            rig.hub.set_climate_eco("living", eco.clone()).await,
            Err(HubError::Rejected(_))
        ));
        assert!(matches!(
            rig.hub.set_climate_eco("nowhere", eco).await,
            Err(HubError::UnknownArea(_))
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn clear_room_eco_returns_to_schedule() {
        let mut rig = Rig::new(None).await;
        rig.settings(heat_rules(19.0));
        rig.hub
            .set_climate_eco("living", eco_for(&rig, 3600))
            .await
            .unwrap();
        assert_eq!(temperature_commands(&rig.pass(0).await), vec![16.0]);
        rig.hub.clear_climate_eco("living").await.unwrap();
        assert_eq!(rig.zone().eco, None);
        assert_eq!(temperature_commands(&rig.pass(60).await), vec![19.0]);
    }

    /// A test alarm panel in the house presence list; returns its state topic.
    fn alarm_presence(rig: &Rig) -> &'static str {
        rig.hub.ingest(
            "nidoo/alarm_control_panel/test/panel/config",
            br#"{"name":"Panel","unique_id":"panel_test","state_topic":"t/panel","command_topic":"t/panel/set"}"#,
        );
        let panel = rig
            .hub
            .snapshot()
            .entities
            .into_iter()
            .find(|e| e.id.contains("panel_test"))
            .expect("alarm panel")
            .id;
        rig.hub
            .set_climate_house(HouseSettings {
                presence: vec![panel],
                ..HouseSettings::default()
            })
            .unwrap();
        "t/panel"
    }

    #[tokio::test(start_paused = true)]
    async fn armed_away_puts_the_house_in_eco() {
        let mut rig = Rig::new(None).await;
        let topic = alarm_presence(&rig);
        rig.hub.ingest(topic, b"disarmed");
        rig.settings(heat_rules(19.0));
        assert_eq!(temperature_commands(&rig.pass(0).await), vec![19.0]);
        // Armed away is a definite answer: no grace, eco on the next pass.
        rig.hub.ingest(topic, b"armed_away");
        assert_eq!(temperature_commands(&rig.pass(60).await), vec![16.0]);
        assert_eq!(rig.zone().decision.unwrap().source, Source::Away);
        rig.hub.ingest(topic, b"disarmed");
        assert_eq!(temperature_commands(&rig.pass(120).await), vec![19.0]);
    }

    #[tokio::test(start_paused = true)]
    async fn armed_home_and_night_are_home() {
        let mut rig = Rig::new(None).await;
        let topic = alarm_presence(&rig);
        rig.settings(heat_rules(19.0));
        for (i, state) in ["armed_home", "armed_night"].into_iter().enumerate() {
            rig.hub.ingest(topic, state.as_bytes());
            rig.pass(1000 * (i as u64 + 1)).await;
            assert_eq!(
                rig.zone().decision.unwrap().source,
                Source::Schedule,
                "{state}"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn alarm_transient_states_keep_last_answer() {
        let mut rig = Rig::new(None).await;
        let topic = alarm_presence(&rig);
        rig.hub.ingest(topic, b"disarmed");
        rig.settings(heat_rules(19.0));
        rig.pass(0).await;
        rig.hub.ingest(topic, b"armed_away");
        rig.pass(960).await;
        assert_eq!(rig.zone().decision.unwrap().source, Source::Away);
        for (i, state) in ["triggered", "pending"].into_iter().enumerate() {
            rig.hub.ingest(topic, state.as_bytes());
            rig.pass(1000 + 60 * i as u64).await;
            assert_eq!(rig.zone().decision.unwrap().source, Source::Away, "{state}");
        }
        rig.hub.ingest(topic, b"disarmed");
        rig.pass(1200).await;
        assert_eq!(rig.zone().decision.unwrap().source, Source::Schedule);
    }

    /// The hub's own alarm panel as the registry sees it; returns its state topic.
    fn native_alarm(rig: &Rig) -> &'static str {
        rig.hub.ingest(
            "nidoo/alarm_control_panel/alarm/house/config",
            br#"{"name":"Alarm","unique_id":"nidoo_alarm_house","state_topic":"t/alarm","command_topic":"t/alarm/set"}"#,
        );
        assert!(
            rig.hub
                .snapshot()
                .entities
                .iter()
                .any(|e| e.id == "nidoo_alarm_house"),
            "native alarm panel"
        );
        "t/alarm"
    }

    /// A presence switch in the house presence list; returns its state topic.
    fn switch_presence(rig: &Rig) -> &'static str {
        rig.hub.ingest(
            "nidoo/switch/presence/home/config",
            br#"{"name":"Home","unique_id":"presence_home","state_topic":"t/home","command_topic":"t/home/set"}"#,
        );
        let switch = rig
            .hub
            .snapshot()
            .entities
            .into_iter()
            .find(|e| e.id.contains("presence_home"))
            .expect("presence switch")
            .id;
        rig.hub
            .set_climate_house(HouseSettings {
                presence: vec![switch],
                ..HouseSettings::default()
            })
            .unwrap();
        "t/home"
    }

    #[tokio::test(start_paused = true)]
    async fn native_alarm_armed_away_is_eco_without_a_presence_list() {
        let mut rig = Rig::new(None).await;
        let topic = native_alarm(&rig);
        rig.hub.ingest(topic, b"disarmed");
        rig.settings(heat_rules(19.0));
        assert_eq!(temperature_commands(&rig.pass(0).await), vec![19.0]);
        rig.hub.ingest(topic, b"armed_away");
        assert_eq!(temperature_commands(&rig.pass(60).await), vec![16.0]);
        assert_eq!(rig.zone().decision.unwrap().source, Source::Away);
        rig.hub.ingest(topic, b"triggered");
        rig.pass(120).await;
        assert_eq!(rig.zone().decision.unwrap().source, Source::Away);
        rig.hub.ingest(topic, b"disarmed");
        assert_eq!(temperature_commands(&rig.pass(180).await), vec![19.0]);
    }

    #[tokio::test(start_paused = true)]
    async fn armed_away_wins_over_a_tracker_at_home() {
        let mut rig = Rig::new(None).await;
        let alarm = native_alarm(&rig);
        let home = switch_presence(&rig);
        rig.hub.ingest(home, b"ON");
        rig.hub.ingest(alarm, b"disarmed");
        rig.settings(heat_rules(19.0));
        assert_eq!(temperature_commands(&rig.pass(0).await), vec![19.0]);
        rig.hub.ingest(alarm, b"armed_away");
        assert_eq!(temperature_commands(&rig.pass(60).await), vec![16.0]);
        assert_eq!(rig.zone().decision.unwrap().source, Source::Away);
    }

    #[tokio::test(start_paused = true)]
    async fn listed_alarm_armed_away_wins_over_a_tracker_at_home() {
        let mut rig = Rig::new(None).await;
        let home = switch_presence(&rig);
        let alarm = alarm_presence(&rig);
        let presence = rig
            .hub
            .snapshot()
            .entities
            .into_iter()
            .filter(|e| e.id.contains("presence_home") || e.id.contains("panel_test"))
            .map(|e| e.id)
            .collect::<Vec<_>>();
        assert_eq!(presence.len(), 2);
        rig.hub
            .set_climate_house(HouseSettings {
                presence,
                ..HouseSettings::default()
            })
            .unwrap();
        rig.hub.ingest(home, b"ON");
        rig.hub.ingest(alarm, b"disarmed");
        rig.settings(heat_rules(19.0));
        rig.pass(0).await;
        rig.hub.ingest(alarm, b"armed_away");
        rig.pass(960).await;
        assert_eq!(rig.zone().decision.unwrap().source, Source::Away);
    }

    #[tokio::test(start_paused = true)]
    async fn disarmed_native_alarm_leaves_trackers_in_charge() {
        let mut rig = Rig::new(None).await;
        let alarm = native_alarm(&rig);
        let home = switch_presence(&rig);
        rig.hub.ingest(home, b"ON");
        rig.hub.ingest(alarm, b"disarmed");
        rig.settings(heat_rules(19.0));
        rig.pass(0).await;
        rig.hub.ingest(home, b"OFF");
        assert_eq!(temperature_commands(&rig.pass(960).await), vec![16.0]);
        assert_eq!(rig.zone().decision.unwrap().source, Source::Away);
    }

    fn fan_commands(out: &[Publish]) -> Vec<String> {
        commands(out)
            .into_iter()
            .filter(|(topic, _)| topic == "/ha/fan_mode/set")
            .map(|(_, payload)| payload)
            .collect()
    }

    /// Unit 0's fan modes other than `auto`, as the recorded device lists them.
    fn other_fans(rig: &Rig) -> Vec<String> {
        rig.hub
            .snapshot()
            .entities
            .into_iter()
            .find(|e| e.id == rig.id)
            .expect("unit 0")
            .capabilities
            .fan_modes
            .into_iter()
            .filter(|m| m != "auto")
            .collect()
    }

    /// Heat rules at 19 °C, eco setting unit 0's fan to its first non-`auto` mode (returned),
    /// the fan at `auto`, and one pass on the schedule.
    async fn fan_rig() -> (Rig, String) {
        let mut rig = Rig::new(None).await;
        let fan = other_fans(&rig)[0].clone();
        rig.sim.fans.insert(0, "auto".to_owned());
        rig.sim.report(&rig.hub, 0);
        rig.hub
            .set_climate_house(HouseSettings {
                eco: nidoo_climate::doc::EcoSettings {
                    actions: vec![nidoo_climate::doc::EcoAction::FanMode(fan.clone())],
                    ..Default::default()
                },
                ..HouseSettings::default()
            })
            .unwrap();
        rig.settings(heat_rules(19.0));
        let out = rig.pass(0).await;
        assert_eq!(temperature_commands(&out), vec![19.0]);
        assert!(fan_commands(&out).is_empty(), "{out:?}");
        (rig, fan)
    }

    #[tokio::test(start_paused = true)]
    async fn eco_sets_the_fan_and_restores_it() {
        let (mut rig, fan) = fan_rig().await;
        rig.hub
            .set_climate_eco("living", eco_for(&rig, 600))
            .await
            .unwrap();
        let out = rig.pass(60).await;
        assert_eq!(temperature_commands(&out), vec![16.0]);
        assert_eq!(fan_commands(&out), vec![fan.clone()]);
        let restore = rig.zone().decision.unwrap().eco_restore;
        assert_eq!(restore[&rig.id].was, "auto");
        assert!(fan_commands(&rig.pass(120).await).is_empty(), "sent once");
        let out = rig.pass(700).await;
        assert_eq!(temperature_commands(&out), vec![19.0]);
        assert_eq!(fan_commands(&out), vec!["auto".to_owned()]);
        assert!(rig.zone().decision.unwrap().eco_restore.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_fan_changed_during_eco_is_not_restored() {
        let (mut rig, fan) = fan_rig().await;
        let other = other_fans(&rig)
            .into_iter()
            .find(|m| *m != fan)
            .expect("two modes");
        rig.hub
            .set_climate_eco("living", eco_for(&rig, 600))
            .await
            .unwrap();
        assert_eq!(fan_commands(&rig.pass(60).await), vec![fan]);
        rig.sim.fans.insert(0, other);
        rig.sim.report(&rig.hub, 0);
        assert!(fan_commands(&rig.pass(120).await).is_empty());
        // Later eco passes leave the user's choice alone too.
        assert!(
            fan_commands(&rig.pass(180).await).is_empty(),
            "not re-applied"
        );
        assert!(
            fan_commands(&rig.pass(240).await).is_empty(),
            "not re-applied"
        );
        assert!(fan_commands(&rig.pass(700).await).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn failed_fan_command_is_retried() {
        let (mut rig, fan) = fan_rig().await;
        rig.hub
            .set_climate_eco("living", eco_for(&rig, 600))
            .await
            .unwrap();
        rig.sim.echo = false;
        assert_eq!(fan_commands(&rig.pass(60).await), vec![fan.clone()]);
        assert!(
            matches!(
                rig.zone().decision.unwrap().eco_restore[&rig.id].state,
                nidoo_climate::doc::FanState::Sent { .. }
            ),
            "a command that did not land is only sent, not set"
        );
        rig.sim.echo = true;
        assert_eq!(fan_commands(&rig.pass(180).await), vec![fan]);
        assert!(!rig.zone().decision.unwrap().eco_restore.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn fan_restored_after_restart() {
        let (mut rig, fan) = fan_rig().await;
        rig.hub
            .set_climate_eco("living", eco_for(&rig, 600))
            .await
            .unwrap();
        assert_eq!(fan_commands(&rig.pass(60).await), vec![fan.clone()]);
        let view = rig.hub.climate_view().unwrap();
        let zone = rig.zone();

        // A new hub gets the retained documents back from the broker.
        let mut again = Rig::new(None).await;
        again.sim.fans.insert(0, fan);
        again.sim.set(&again.hub, 0, "heat", 16.0, 20.0);
        let retained = [
            (
                "nidoo/climate/house",
                serde_json::to_vec(&view.house).unwrap(),
            ),
            (
                "nidoo/climate/zone/living/settings",
                serde_json::to_vec(&zone.settings).unwrap(),
            ),
            (
                "nidoo/climate/zone/living/decision",
                serde_json::to_vec(&zone.decision.unwrap()).unwrap(),
            ),
            (
                "nidoo/climate/zone/living/eco",
                serde_json::to_vec(&zone.eco.unwrap()).unwrap(),
            ),
        ];
        for (topic, payload) in retained {
            again.hub.ingest_message(topic, &payload, true);
        }
        again.t0 = rig.t0;
        assert!(fan_commands(&again.pass(120).await).is_empty());
        again.hub.clear_climate_eco("living").await.unwrap();
        assert_eq!(
            fan_commands(&again.pass(180).await),
            vec!["auto".to_owned()]
        );
    }

    /// The fan commands sent to unit `n`.
    fn fan_commands_to(out: &[Publish], n: u8) -> Vec<String> {
        let topic = format!("daikin/192.0.2.15{n}/ha/fan_mode/set");
        out.iter()
            .filter(|p| p.topic == topic)
            .map(|p| p.payload.clone())
            .collect()
    }

    #[tokio::test(start_paused = true)]
    async fn a_late_fan_echo_is_still_restored() {
        let (mut rig, fan) = fan_rig().await;
        rig.hub
            .set_climate_eco("living", eco_for(&rig, 600))
            .await
            .unwrap();
        rig.sim.echo = false;
        assert_eq!(fan_commands(&rig.pass(60).await), vec![fan.clone()]);
        // The unit applies it after the hub stopped waiting.
        rig.sim.echo = true;
        rig.sim.fans.insert(0, fan);
        rig.sim.report(&rig.hub, 0);
        assert!(fan_commands(&rig.pass(120).await).is_empty());
        assert_eq!(fan_commands(&rig.pass(700).await), vec!["auto".to_owned()]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_command_after_the_eco_fan_changes_keeps_the_original() {
        let (mut rig, fan) = fan_rig().await;
        let other = other_fans(&rig)
            .into_iter()
            .find(|m| *m != fan)
            .expect("two modes");
        rig.hub
            .set_climate_eco("living", eco_for(&rig, 600))
            .await
            .unwrap();
        assert_eq!(fan_commands(&rig.pass(60).await), vec![fan]);
        let mut house = rig.hub.climate_view().unwrap().house;
        house.eco.actions = vec![nidoo_climate::doc::EcoAction::FanMode(other.clone())];
        rig.hub.set_climate_house(house).unwrap();
        drain(&mut rig.rx);
        rig.sim.echo = false;
        assert_eq!(fan_commands(&rig.pass(120).await), vec![other.clone()]);
        rig.sim.echo = true;
        assert_eq!(fan_commands(&rig.pass(180).await), vec![other]);
        assert_eq!(fan_commands(&rig.pass(700).await), vec!["auto".to_owned()]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_unit_leaving_the_zone_gets_its_fan_back() {
        let (mut rig, fan) = fan_rig().await;
        feed_daikin(&rig.hub, 1, "living").await;
        rig.sim.fans.insert(1, "auto".to_owned());
        rig.sim.set(&rig.hub, 1, "heat", 19.0, 20.0);
        drain(&mut rig.rx);
        rig.hub
            .set_climate_eco("living", eco_for(&rig, 600))
            .await
            .unwrap();
        let out = rig.pass(60).await;
        assert_eq!(fan_commands_to(&out, 0), vec![fan.clone()]);
        let id = rig.id.clone();
        place(&rig.hub, &id, "bedroom").await;
        drain(&mut rig.rx);
        let out = rig.pass(120).await;
        assert_eq!(fan_commands_to(&out, 0), vec!["auto".to_owned()]);
        assert!(
            !rig.zone().decision.unwrap().eco_restore.contains_key(&id),
            "forgotten once put back"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_zone_set_to_pending_restores_the_fan() {
        let (mut rig, fan) = fan_rig().await;
        rig.hub
            .set_climate_eco("living", eco_for(&rig, 600))
            .await
            .unwrap();
        assert_eq!(fan_commands(&rig.pass(60).await), vec![fan]);
        rig.settings(ZoneSettings {
            state: ZoneState::Pending,
            ..heat_rules(19.0)
        });
        assert_eq!(fan_commands(&rig.pass(120).await), vec!["auto".to_owned()]);
    }

    #[tokio::test(start_paused = true)]
    async fn switching_the_zone_off_restores_the_fan_once() {
        let (mut rig, fan) = fan_rig().await;
        rig.hub
            .set_climate_eco("living", eco_for(&rig, 600))
            .await
            .unwrap();
        assert_eq!(fan_commands(&rig.pass(60).await), vec![fan]);
        rig.settings(ZoneSettings {
            state: ZoneState::Off,
            ..heat_rules(19.0)
        });
        assert_eq!(fan_commands(&rig.pass(120).await), vec!["auto".to_owned()]);
        assert!(fan_commands(&rig.pass(180).await).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn the_loop_waits_for_the_broker_to_settle_then_commands() {
        let mut rig = Rig::new(None).await;
        rig.settings(heat_rules(17.0));
        let task = rig.hub.start_climate().expect("enabled");
        tokio::time::sleep(Duration::from_secs(5)).await;
        assert!(drain(&mut rig.rx).is_empty(), "still settling");
        let first = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let published = rig.rx.recv().await.expect("publisher open");
                rig.sim.on_publish(&rig.hub, &published);
                if published.topic.ends_with("/ha/temperature/set") {
                    return published;
                }
            }
        })
        .await
        .expect("the loop commands the device");
        assert_eq!(first.payload, "17");
        task.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn approving_a_proposal_commands_the_device() {
        let mut rig = Rig::new(None).await;
        let proposal = nidoo_climate::doc::Proposal {
            id: "p-1".to_owned(),
            zone: "living".to_owned(),
            kind: nidoo_climate::doc::ProposalKind::Initial,
            created: rig.t0,
            window: nidoo_climate::doc::Window {
                from: 0,
                to: 1,
                days: 28,
            },
            rules: heat_rules(17.0).rules,
            changes: Vec::new(),
            fingerprint: "fp".to_owned(),
        };
        rig.hub.ingest_message(
            "nidoo/climate/zone/living/proposal",
            serde_json::to_string(&proposal).unwrap().as_bytes(),
            true,
        );
        let hub = Arc::clone(&rig.hub);
        let mut task =
            tokio::spawn(async move { hub.approve_climate_proposal("living", None).await });
        let mut out = Vec::new();
        loop {
            tokio::select! {
                published = rig.rx.recv() => {
                    let published = published.expect("publisher open");
                    rig.sim.on_publish(&rig.hub, &published);
                    out.push(published);
                }
                done = &mut task => {
                    done.expect("task").expect("approved");
                    break;
                }
            }
        }
        out.extend(drain(&mut rig.rx));
        assert_eq!(temperature_commands(&out), vec![17.0], "{out:?}");
        assert_eq!(rig.zone().settings.state, ZoneState::Active);
    }

    #[tokio::test]
    async fn no_loop_while_disabled() {
        let (hub, _rx) = hub(false);
        assert!(hub.start_climate().is_none());
        hub.evaluate_climate(1).await;
        hub.observe_climate("climate.none", 1);
    }

    #[tokio::test(start_paused = true)]
    async fn view_reading_is_live_not_the_retained_decision() {
        let mut rig = Rig::new(None).await;
        rig.sim.set(&rig.hub, 0, "heat", 22.0, 19.0);
        let decision = Decision {
            eco_restore: BTreeMap::new(),
            adjust: None,
            zone: "living".into(),
            mode: Some(Mode::Heat),
            setpoint: None,
            source: Source::Idle,
            cause: None,
            reason: String::new(),
            reading: Some(99.0),
            reading_age_s: Some(9999),
            outdoor_mean: None,
            home: true,
            holiday: false,
            applied: false,
            at: rig.t0,
            entities: BTreeMap::new(),
        };
        rig.hub.ingest_message(
            "nidoo/climate/zone/living/decision",
            serde_json::to_string(&decision).unwrap().as_bytes(),
            true,
        );
        let zone = rig.zone();
        assert_eq!(zone.reading, Some(19.0));
        assert!(zone.reading_age_s.unwrap() < 60, "{:?}", zone.reading_age_s);
    }

    /// Feeds the Z2M temperature sensor `fixture` (`<ieee>__<object>`), puts it in `area` and
    /// reports `value` on `state_topic` under `key`; returns its entity id.
    async fn feed_thermometer(
        hub: &Hub,
        fixture_name: &str,
        area: &str,
        state_topic: &str,
        key: &str,
        value: f64,
    ) -> String {
        let (ieee, object) = fixture_name.split_once("__").unwrap();
        hub.ingest(
            &format!("nidoo/sensor/{ieee}/{object}/config"),
            &fixture(&format!("sensor__{fixture_name}.json")),
        );
        let id = place(hub, &format!("{ieee}_{object}"), area).await;
        hub.ingest(
            state_topic,
            serde_json::json!({ key: value }).to_string().as_bytes(),
        );
        id
    }

    #[tokio::test(start_paused = true)]
    async fn a_room_thermometer_reads_the_room_before_the_devices() {
        let mut rig = Rig::new(None).await;
        rig.sim.set(&rig.hub, 0, "heat", 22.0, 24.0);
        // A device's chip temperature is never the room.
        feed_thermometer(
            &rig.hub,
            "0x00158d000449ba5c__device_temperature",
            "living",
            "zigbee2mqtt/rooms/stair/motion",
            "device_temperature",
            30.0,
        )
        .await;
        let zone = rig.zone();
        assert_eq!((zone.room_sensor, zone.reading), (None, Some(24.0)));

        let thermometer = feed_thermometer(
            &rig.hub,
            "0x5c0272fffea35cfb__temperature",
            "living",
            "zigbee2mqtt/rooms/bedroom2/thermometer",
            "temperature",
            19.5,
        )
        .await;
        let zone = rig.zone();
        assert_eq!(zone.room_sensor.as_deref(), Some(thermometer.as_str()));
        assert_eq!(zone.reading, Some(19.5));

        // The outdoor sensor is never a room thermometer.
        rig.hub
            .set_climate_house(nidoo_climate::doc::HouseSettings {
                outdoor_entity: Some(thermometer.clone()),
                ..Default::default()
            })
            .unwrap();
        let zone = rig.zone();
        assert_eq!((zone.room_sensor, zone.reading), (None, Some(24.0)));

        // A sensor set on the zone is used as is.
        rig.settings(ZoneSettings {
            sensor: Some(thermometer.clone()),
            ..ZoneSettings::default()
        });
        let zone = rig.zone();
        assert_eq!(zone.room_sensor.as_deref(), Some(thermometer.as_str()));
        assert_eq!(zone.reading, Some(19.5));
    }

    const OCCUPANCY_STATE: &str = "zigbee2mqtt/rooms/living/motion_south";

    /// Reports the occupancy sensor `on` or not.
    fn occupancy(hub: &Hub, on: bool) {
        hub.ingest(
            OCCUPANCY_STATE,
            serde_json::json!({ "occupancy": on })
                .to_string()
                .as_bytes(),
        );
    }

    /// The recorded occupancy sensor, placed in `area` and reporting `on`; returns its id.
    async fn feed_occupancy(hub: &Hub, area: &str, on: bool) -> String {
        hub.ingest(
            "nidoo/binary_sensor/0x00158d00044c4d4c/occupancy/config",
            &fixture("binary_sensor__0x00158d00044c4d4c__occupancy.json"),
        );
        let id = place(hub, "0x00158d00044c4d4c_occupancy", area).await;
        occupancy(hub, on);
        id
    }

    /// An active heating zone with a 19 °C block all day over a 15 °C base, presence on with one
    /// occupancy sensor that reports nobody. Returns the rig and the sensor's id.
    async fn presence_rig() -> (Rig, String) {
        let mut rig = Rig::new(None).await;
        rig.sim.set(&rig.hub, 0, "heat", 19.0, 20.0);
        let sensor = feed_occupancy(&rig.hub, "living", false).await;
        let mut settings = heat_rules(15.0);
        settings.rules.heat.as_mut().unwrap().blocks.push(Block {
            days: Day::ALL.to_vec(),
            from: HhMm(0),
            to: HhMm(1440),
            setpoint: Setpoint::Temp(19.0),
            evidence: None,
        });
        settings.presence.enabled = true;
        settings.presence.sensors = vec![sensor.clone()];
        rig.settings(settings);
        (rig, sensor)
    }

    /// The registry's current state of `id`, as a `StateChanged` event carries it.
    fn state_of(hub: &Hub, id: &str) -> nidoo_core::model::EntityState {
        hub.snapshot()
            .entities
            .into_iter()
            .find(|e| e.id == id)
            .expect("entity")
            .state
    }

    /// The sensor reports `on` at `t0 + at`, and the engine sees the change, as its loop does.
    fn detect(rig: &Rig, sensor: &str, on: bool, at: u64) {
        occupancy(&rig.hub, on);
        let state = state_of(&rig.hub, sensor);
        rig.hub.observe_presence(sensor, &state, rig.t0 + at);
    }

    #[tokio::test(start_paused = true)]
    async fn a_pulse_handled_late_still_counts() {
        let (mut rig, sensor) = presence_rig().await;
        rig.pass(0).await;
        // Both events of a short pulse wait behind a slow pass: by the time they are handled
        // the registry already says off again, but each event carries its own state.
        occupancy(&rig.hub, true);
        let on = state_of(&rig.hub, &sensor);
        occupancy(&rig.hub, false);
        let off = state_of(&rig.hub, &sensor);
        rig.hub.observe_presence(&sensor, &on, rig.t0 + 1700);
        rig.hub.observe_presence(&sensor, &off, rig.t0 + 1740);
        rig.pass(1800).await;
        assert_eq!(rig.sim.units[&0].1, 19.0);
        rig.pass(3540).await;
        assert_eq!(rig.sim.units[&0].1, 17.0);
    }

    #[tokio::test(start_paused = true)]
    async fn an_empty_room_is_set_back_after_30_min() {
        let (mut rig, _) = presence_rig().await;
        rig.pass(0).await;
        assert_eq!(rig.sim.units[&0].1, 19.0);
        rig.pass(1799).await;
        assert_eq!(rig.sim.units[&0].1, 19.0);
        rig.pass(1800).await;
        assert_eq!(rig.sim.units[&0].1, 17.0);
        assert_eq!(rig.zone().decision.unwrap().source, Source::Empty);
    }

    #[tokio::test(start_paused = true)]
    async fn a_pulse_between_passes_counts() {
        let (mut rig, sensor) = presence_rig().await;
        rig.pass(0).await;
        detect(&rig, &sensor, true, 1700);
        detect(&rig, &sensor, false, 1740);
        rig.pass(1800).await;
        assert_eq!(rig.sim.units[&0].1, 19.0);
        rig.pass(3539).await;
        assert_eq!(rig.sim.units[&0].1, 19.0);
        rig.pass(3540).await;
        assert_eq!(rig.sim.units[&0].1, 17.0);
    }

    #[tokio::test(start_paused = true)]
    async fn idle_reports_are_not_detections() {
        let (mut rig, sensor) = presence_rig().await;
        rig.pass(0).await;
        for at in [600, 1200] {
            detect(&rig, &sensor, false, at);
        }
        rig.pass(1800).await;
        assert_eq!(rig.sim.units[&0].1, 17.0);
    }

    #[tokio::test(start_paused = true)]
    async fn staying_brings_it_back() {
        let (mut rig, sensor) = presence_rig().await;
        rig.pass(0).await;
        rig.pass(1800).await;
        assert_eq!(rig.sim.units[&0].1, 17.0);
        // Someone comes in and stays; the change triggers a pass, as the loop's debounce does.
        detect(&rig, &sensor, true, 1900);
        rig.pass(1900).await;
        rig.pass(2140).await;
        assert_eq!(rig.sim.units[&0].1, 17.0);
        rig.pass(2200).await;
        assert_eq!(rig.sim.units[&0].1, 19.0);
        assert_eq!(rig.zone().decision.unwrap().source, Source::Schedule);
    }

    #[tokio::test(start_paused = true)]
    async fn empty_decision_is_stable() {
        let (mut rig, _) = presence_rig().await;
        rig.pass(0).await;
        let first = rig.pass(1800).await;
        assert_eq!(decisions_published(&first), 1, "{first:?}");
        let second = rig.pass(1860).await;
        assert_eq!(decisions_published(&second), 0, "{second:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn view_shows_room_presence() {
        let (mut rig, _) = presence_rig().await;
        rig.pass(0).await;
        assert_eq!(
            rig.zone().presence,
            Some(PresenceView { empty_since: None })
        );
        rig.pass(1800).await;
        assert_eq!(
            rig.zone().presence,
            Some(PresenceView {
                empty_since: Some(rig.t0 + 1800)
            })
        );
        let mut settings = rig.zone().settings;
        settings.presence.enabled = false;
        rig.settings(settings);
        assert_eq!(rig.zone().presence, None);
    }

    #[tokio::test(start_paused = true)]
    async fn unavailable_sensor_keeps_the_room_occupied() {
        let (mut rig, _) = presence_rig().await;
        rig.hub
            .ingest("zigbee2mqtt/bridge/state", br#"{"state":"offline"}"#);
        rig.pass(0).await;
        rig.pass(3600).await;
        assert_eq!(rig.sim.units[&0].1, 19.0);
        assert_eq!(rig.zone().decision.unwrap().source, Source::Schedule);
    }

    /// An active heating zone at 19 °C with the correction on and a room thermometer at `room`.
    /// 19.6 is just over the band: one step down (-0.5), not a jump.
    async fn corrected_rig(room: f64) -> Rig {
        let mut rig = Rig::new(None).await;
        rig.sim.set(&rig.hub, 0, "heat", 19.0, 20.0);
        feed_thermometer(
            &rig.hub,
            "0x5c0272fffea35cfb__temperature",
            "living",
            "zigbee2mqtt/rooms/bedroom2/thermometer",
            "temperature",
            room,
        )
        .await;
        // A 19 °C block all day over a 17 °C base: the correction may go below the block.
        let mut settings = heat_rules(17.0);
        settings.rules.heat.as_mut().unwrap().blocks.push(Block {
            days: Day::ALL.to_vec(),
            from: HhMm(0),
            to: HhMm(1440),
            setpoint: Setpoint::Temp(19.0),
            evidence: None,
        });
        settings.feedback.enabled = true;
        rig.settings(settings);
        rig
    }

    #[tokio::test(start_paused = true)]
    async fn correction_steps_down_and_is_not_a_hand_change() {
        let mut rig = corrected_rig(19.6).await;
        let out = rig.pass(0).await;
        assert_eq!(temperature_commands(&out), vec![18.5], "{out:?}");
        // The device echoed 18.5: the next report is not someone's doing.
        rig.hub.observe_climate(&rig.id, rig.t0 + 100);
        assert!(
            rig.zone().overrides.is_empty(),
            "{:?}",
            rig.zone().overrides
        );
        let out = rig.pass(120).await;
        assert!(commands(&out).is_empty(), "{out:?}");
        assert!(rig.zone().overrides.is_empty());
        assert_eq!(rig.sim.units[&0].1, 18.5);
        let decision = rig.zone().decision.unwrap();
        assert_eq!(decision.adjust, Some(-0.5));
    }

    #[tokio::test(start_paused = true)]
    async fn a_quiet_room_thermometer_still_reads_the_room() {
        // A thermometer that only reports changes, silent for 4 h in a steady room.
        let mut rig = corrected_rig(17.4).await;
        rig.pass(4 * 3600).await;
        let decision = rig.zone().decision.unwrap();
        assert_eq!(decision.source, Source::Schedule, "{}", decision.reason);
        assert_eq!(decision.reading, Some(17.4));
        assert_eq!(decision.reading_age_s, Some(4 * 3600));
        assert!(!decision.reason.contains("silent"), "{}", decision.reason);
    }

    #[tokio::test(start_paused = true)]
    async fn correction_note_is_stable_while_the_reading_ages() {
        let mut rig = corrected_rig(19.6).await;
        // The thermometer reported 20 min ago: the note must not carry its age.
        let first = rig.pass(1200).await;
        assert_eq!(decisions_published(&first), 1, "{first:?}");
        let second = rig.pass(1260).await;
        assert_eq!(decisions_published(&second), 0, "{second:?}");
        let decision = rig.zone().decision.unwrap();
        assert!(
            decision
                .reason
                .ends_with("room above 19.5 °C: AC 18.5 °C (-0.5)"),
            "{}",
            decision.reason
        );
    }

    #[tokio::test(start_paused = true)]
    async fn correction_survives_restart_without_hand_change() {
        let mut rig = corrected_rig(19.6).await;
        // The device sits at the corrected 18.5, as the retained decision says.
        rig.sim.set(&rig.hub, 0, "heat", 18.5, 20.0);
        let decision = Decision {
            eco_restore: BTreeMap::new(),
            adjust: Some(-0.5),
            zone: "living".into(),
            mode: Some(Mode::Heat),
            setpoint: Some(Setpoint::Temp(19.0)),
            source: Source::Schedule,
            cause: None,
            reason: "Schedule base: 19.0 °C".into(),
            reading: Some(19.6),
            reading_age_s: Some(30),
            outdoor_mean: None,
            home: true,
            holiday: false,
            applied: true,
            // Stepped 5 min before the restart: the next step still waits interval_min (15).
            at: rig.t0 - 300,
            entities: BTreeMap::from([(
                rig.id.clone(),
                DeviceTarget {
                    setpoint: Some(18.5),
                    mode: "heat".into(),
                    held: false,
                },
            )]),
        };
        rig.hub.ingest_message(
            "nidoo/climate/zone/living/decision",
            serde_json::to_string(&decision).unwrap().as_bytes(),
            true,
        );
        let out = rig.pass(0).await;
        assert!(
            rig.zone().overrides.is_empty(),
            "{:?}",
            rig.zone().overrides
        );
        assert!(commands(&out).is_empty(), "{out:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn warmup_ends_once_the_room_reached_the_band() {
        // The room sits in the band (18.5 and up) on the first tick, then cools: the correction
        // raises 10 min into the block instead of waiting out the 60-min warm-up.
        let mut rig = corrected_rig(18.6).await;
        let out = rig.pass(0).await;
        assert!(temperature_commands(&out).is_empty(), "{out:?}");
        rig.hub.ingest(
            "zigbee2mqtt/rooms/bedroom2/thermometer",
            br#"{"temperature":18.0}"#,
        );
        let out = rig.pass(600).await;
        assert_eq!(temperature_commands(&out), vec![19.5], "{out:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn real_hand_change_still_holds() {
        let mut rig = corrected_rig(19.6).await;
        rig.pass(0).await;
        assert_eq!(rig.sim.units[&0].1, 18.5);
        rig.sim.set(&rig.hub, 0, "heat", 21.0, 20.0);
        rig.hub.observe_climate(&rig.id, rig.t0 + 600);
        let overrides = rig.zone().overrides;
        assert_eq!(overrides.len(), 1, "{overrides:?}");
        assert_eq!(overrides[0].setpoint, Some(Setpoint::Temp(21.0)));
        let out = rig.pass(661).await;
        assert!(commands(&out).is_empty(), "{out:?}");
        assert_eq!(rig.sim.units[&0].1, 21.0);
    }

    /// A rig whose house presence follows the `t/home` switch, at home, heating at 17 °C.
    async fn house_rig() -> Rig {
        let mut rig = Rig::new(None).await;
        rig.hub.ingest(
            "nidoo/switch/presence/home/config",
            br#"{"name":"Home","unique_id":"presence_home","state_topic":"t/home","command_topic":"t/home/set"}"#,
        );
        let switch = rig
            .hub
            .snapshot()
            .entities
            .into_iter()
            .find(|e| e.id.contains("presence_home"))
            .expect("presence switch")
            .id;
        rig.hub.ingest("t/home", b"ON");
        rig.hub
            .set_climate_house(HouseSettings {
                presence: vec![switch],
                ..HouseSettings::default()
            })
            .unwrap();
        rig.settings(heat_rules(17.0));
        rig
    }

    fn boost(rig: &Rig, setpoint: f64) -> Override {
        Override {
            entity: None,
            setpoint: Some(Setpoint::Temp(setpoint)),
            mode: None,
            until: rig.t0 + 7200,
            origin: OverrideOrigin::User,
        }
    }

    fn source(rig: &Rig) -> Source {
        rig.zone().decision.expect("decision").source
    }

    #[tokio::test(start_paused = true)]
    async fn leaving_drops_a_boost_set_before() {
        let mut rig = house_rig().await;
        rig.pass(0).await;
        rig.hub
            .set_climate_override("living", boost(&rig, 22.0))
            .await
            .unwrap();
        rig.pass(60).await;
        assert_eq!(source(&rig), Source::Boost);
        // Nobody home past the grace: away, and the boost is gone for good.
        rig.hub.ingest("t/home", b"OFF");
        let out = rig.pass(1000).await;
        assert_eq!(source(&rig), Source::Away);
        assert!(rig.zone().overrides.iter().all(|o| o.entity.is_some()));
        let retained = out
            .iter()
            .rfind(|p| p.topic == "nidoo/climate/zone/living/override")
            .expect("override doc republished");
        // An empty list clears the retained doc.
        if !retained.payload.is_empty() {
            let list: Vec<Override> = serde_json::from_str(&retained.payload).unwrap();
            assert!(list.iter().all(|o| o.entity.is_some()), "{list:?}");
        }
        rig.hub.ingest("t/home", b"ON");
        rig.pass(1100).await;
        assert_eq!(source(&rig), Source::Schedule);
    }

    #[tokio::test(start_paused = true)]
    async fn boost_set_while_away_runs_and_survives_return() {
        let mut rig = house_rig().await;
        rig.pass(0).await;
        rig.hub.ingest("t/home", b"OFF");
        rig.pass(1000).await;
        assert_eq!(source(&rig), Source::Away);
        rig.hub
            .set_climate_override("living", boost(&rig, 22.0))
            .await
            .unwrap();
        rig.pass(1060).await;
        assert_eq!(source(&rig), Source::Boost);
        rig.hub.ingest("t/home", b"ON");
        rig.pass(1120).await;
        assert_eq!(source(&rig), Source::Boost);
    }

    #[tokio::test(start_paused = true)]
    async fn leaving_keeps_hand_holds() {
        let mut rig = house_rig().await;
        rig.pass(0).await;
        rig.hub
            .set_climate_override("living", boost(&rig, 22.0))
            .await
            .unwrap();
        let hold = Override {
            entity: Some(rig.id.clone()),
            ..boost(&rig, 20.0)
        };
        rig.hub.set_climate_override("living", hold).await.unwrap();
        rig.pass(60).await;
        rig.hub.ingest("t/home", b"OFF");
        rig.pass(1000).await;
        let overrides = rig.zone().overrides;
        assert_eq!(overrides.len(), 1, "{overrides:?}");
        assert_eq!(overrides[0].entity.as_deref(), Some(rig.id.as_str()));
    }

    #[tokio::test(start_paused = true)]
    async fn restart_while_away_keeps_new_boost() {
        let mut rig = house_rig().await;
        rig.pass(0).await;
        rig.hub.ingest("t/home", b"OFF");
        let out = rig.pass(1000).await;
        assert_eq!(source(&rig), Source::Away);
        let decision = out
            .iter()
            .rfind(|p| p.topic == "nidoo/climate/zone/living/decision")
            .expect("decision published")
            .clone();
        // A new hub picks up the retained away decision, then a boost is set while away.
        let mut next = house_rig().await;
        next.t0 = rig.t0;
        next.hub.ingest("t/home", b"OFF");
        next.hub
            .ingest_message(&decision.topic, decision.payload.as_bytes(), true);
        next.hub
            .set_climate_override("living", boost(&next, 22.0))
            .await
            .unwrap();
        next.pass(1100).await;
        assert_eq!(source(&next), Source::Boost);
    }
}

// ----- learning, proposals and approval -----

mod approval {
    use std::time::Duration;

    use nidoo::clock;
    use nidoo::history::History;
    use nidoo_climate::doc::{Proposal, ProposalKind, Rejection, Window};
    use nidoo_climate::view::LearnStatus;

    use super::*;

    const RESPONSES: &str = "nidoo/push/responses";

    fn rules(base: f64) -> Rules {
        Rules {
            heat: Some(ModeRules {
                base: Setpoint::Temp(base),
                blocks: vec![Block {
                    days: vec![Day::Mon, Day::Tue],
                    from: HhMm(420),
                    to: HhMm(600),
                    setpoint: Setpoint::Temp(20.0),
                    evidence: None,
                }],
            }),
            cool: None,
        }
    }

    fn proposal(id: &str, base: f64) -> Proposal {
        Proposal {
            id: id.to_owned(),
            zone: "living".to_owned(),
            kind: ProposalKind::Initial,
            created: 1_000,
            window: Window {
                from: 0,
                to: 1,
                days: 28,
            },
            rules: rules(base),
            changes: vec!["Base: 17.0 °C".to_owned()],
            fingerprint: format!("fp-{id}"),
        }
    }

    /// A hub with a living zone (one Daikin, no state, so nothing is ever commanded), pending,
    /// with `p-1` proposed and a history that is configured but unreachable.
    async fn rig() -> (Arc<Hub>, mpsc::Receiver<Publish>) {
        let (hub, mut rx) = hub(true);
        feed_daikin(&hub, 0, "living").await;
        let (history, _rx) = History::detached(8, chrono_tz::UTC);
        hub.set_history(history);
        hub.set_climate_zone("living", ZoneSettings::default())
            .unwrap();
        seed(&hub, &proposal("p-1", 17.0));
        drain(&mut rx);
        (hub, rx)
    }

    fn seed(hub: &Hub, proposal: &Proposal) {
        hub.ingest_message(
            "nidoo/climate/zone/living/proposal",
            serde_json::to_string(proposal).unwrap().as_bytes(),
            true,
        );
    }

    fn zone(hub: &Hub) -> nidoo_climate::view::ZoneView {
        hub.climate_view()
            .unwrap()
            .zones
            .into_iter()
            .find(|z| z.zone == "living")
            .expect("living zone")
    }

    /// Makes the push service accept alerts, as it does once the broker has settled.
    async fn load_push(hub: &Hub) {
        hub.on_connected();
        hub.on_suback();
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    #[tokio::test]
    async fn history_off_reports_status() {
        let (hub, mut rx) = hub(true);
        feed_daikin(&hub, 0, "living").await;
        hub.set_climate_zone("living", active()).unwrap();
        drain(&mut rx);
        let status = hub.learn_climate_zone("living").await.unwrap();
        assert_eq!(status, LearnStatus::HistoryOff);
        assert!(zone(&hub).proposal.is_none());
        assert!(drain(&mut rx).is_empty());
        assert!(matches!(
            hub.learn_climate_zone("nowhere").await,
            Err(HubError::UnknownArea(_))
        ));
    }

    #[tokio::test]
    async fn learning_is_refused_while_disabled() {
        let (hub, _rx) = hub(false);
        assert!(matches!(
            hub.learn_climate_zone("living").await,
            Err(HubError::Publish(m)) if m == "climate disabled"
        ));
        assert!(matches!(
            hub.approve_climate_proposal("living", None).await,
            Err(HubError::Publish(m)) if m == "climate disabled"
        ));
        assert!(matches!(
            hub.reject_climate_proposal("living").await,
            Err(HubError::Publish(m)) if m == "climate disabled"
        ));
    }

    #[tokio::test]
    async fn view_says_proposed_even_without_history() {
        let (hub, _rx) = hub(true);
        feed_daikin(&hub, 0, "living").await;
        hub.set_climate_zone("living", active()).unwrap();
        assert_eq!(zone(&hub).learn, LearnStatus::HistoryOff);
        seed(&hub, &proposal("p-1", 17.0));
        assert_eq!(zone(&hub).learn, LearnStatus::Proposed);
    }

    #[tokio::test(start_paused = true)]
    async fn approve_activates_zone_and_clears_proposal() {
        let (hub, mut rx) = rig().await;
        load_push(&hub).await;
        let before = clock::now();
        hub.approve_climate_proposal("living", None).await.unwrap();
        let published = drain(&mut rx);

        let view = zone(&hub);
        assert_eq!(view.settings.state, ZoneState::Active);
        assert_eq!(view.settings.rules, rules(17.0));
        assert_eq!(view.settings.approved_proposal.as_deref(), Some("p-1"));
        assert!(view.settings.approved_at.unwrap() >= before);
        assert!(view.proposal.is_none());
        assert_eq!(view.learn, LearnStatus::UpToDate);

        let cleared = published
            .iter()
            .find(|p| p.topic == "nidoo/climate/zone/living/proposal")
            .expect("proposal topic published");
        assert!(cleared.retain && cleared.payload.is_empty());
        assert!(
            published
                .iter()
                .any(|p| p.topic == "nidoo/climate/zone/living/settings" && p.retain)
        );
        // The alert is cancelled in the push history, whether or not this run raised it.
        let history = published
            .iter()
            .rfind(|p| p.topic == "nidoo/push/history")
            .expect("the push history was published");
        let entries: serde_json::Value = serde_json::from_str(&history.payload).unwrap();
        assert!(
            entries
                .as_array()
                .unwrap()
                .iter()
                .any(|e| { e["id"] == "climate-proposal-p-1" && e["cancel"] == true }),
            "{entries}"
        );

        // Nothing is pending any more.
        assert!(matches!(
            hub.approve_climate_proposal("living", None).await,
            Err(HubError::Rejected(m)) if m == "no pending proposal"
        ));
        assert!(matches!(
            hub.reject_climate_proposal("living").await,
            Err(HubError::Rejected(m)) if m == "no pending proposal"
        ));
    }

    #[tokio::test]
    async fn approve_with_edited_rules_validates() {
        let (hub, mut rx) = rig().await;
        let block = |from: u16, to: u16| Block {
            days: vec![Day::Mon],
            from: HhMm(from),
            to: HhMm(to),
            setpoint: Setpoint::Temp(20.0),
            evidence: None,
        };
        let overlapping = Rules {
            heat: Some(ModeRules {
                base: Setpoint::Temp(18.0),
                blocks: vec![block(60, 300), block(120, 400)],
            }),
            cool: None,
        };
        assert!(matches!(
            hub.approve_climate_proposal("living", Some(overlapping))
                .await,
            Err(HubError::Rejected(_))
        ));
        assert!(
            drain(&mut rx).is_empty(),
            "a refused approval publishes nothing"
        );
        let view = zone(&hub);
        assert_eq!(view.settings.state, ZoneState::Pending);
        assert!(view.proposal.is_some());

        // An edited, valid rule set replaces the proposal's.
        hub.approve_climate_proposal("living", Some(rules(15.0)))
            .await
            .unwrap();
        assert_eq!(zone(&hub).settings.rules, rules(15.0));
    }

    #[tokio::test]
    async fn approve_keeps_an_off_zone_off() {
        let (hub, _rx) = rig().await;
        let off = ZoneSettings {
            state: ZoneState::Off,
            ..ZoneSettings::default()
        };
        hub.set_climate_zone("living", off).unwrap();
        hub.approve_climate_proposal("living", None).await.unwrap();
        assert_eq!(zone(&hub).settings.state, ZoneState::Off);
    }

    #[tokio::test(start_paused = true)]
    async fn reject_remembers_fingerprint_for_7_days() {
        let (hub, mut rx) = rig().await;
        let now = clock::now();
        let expired = Rejection {
            fingerprint: "old".to_owned(),
            until: now - 10,
        };
        let live = Rejection {
            fingerprint: "live".to_owned(),
            until: now + 1_000,
        };
        let settings = ZoneSettings {
            rejected: vec![expired, live.clone()],
            ..ZoneSettings::default()
        };
        hub.set_climate_zone("living", settings).unwrap();
        drain(&mut rx);

        hub.reject_climate_proposal("living").await.unwrap();
        let view = zone(&hub);
        assert!(view.proposal.is_none());
        assert_eq!(
            view.settings.state,
            ZoneState::Pending,
            "reject changes no rules"
        );
        assert!(view.settings.rules.is_empty());
        let rejected = &view.settings.rejected;
        assert_eq!(rejected.len(), 2, "{rejected:?}");
        assert_eq!(rejected[0], live, "the expired one is pruned");
        assert_eq!(rejected[1].fingerprint, "fp-p-1");
        let week = 7 * 86_400;
        assert!((now + week..=now + week + 5).contains(&rejected[1].until));
        assert_eq!(view.learn, LearnStatus::Rejected);

        let published = drain(&mut rx);
        assert!(
            published
                .iter()
                .any(|p| p.topic == "nidoo/climate/zone/living/proposal" && p.payload.is_empty())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn push_reply_approves() {
        let (hub, _rx) = rig().await;
        // A retained message is a replay, never a button press.
        let reply = br#"{"id":"climate-proposal-p-1","action":"approve","install_id":"x","ts":1}"#;
        hub.ingest_message(RESPONSES, reply, true);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(zone(&hub).settings.state, ZoneState::Pending);

        let changes = hub.ingest_message(RESPONSES, reply, false);
        assert!(changes.is_empty());
        tokio::time::sleep(Duration::from_millis(50)).await;
        let view = zone(&hub);
        assert_eq!(view.settings.state, ZoneState::Active);
        assert_eq!(view.settings.rules, rules(17.0));
        assert!(view.proposal.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn push_reply_rejects() {
        let (hub, _rx) = rig().await;
        hub.ingest_message(
            RESPONSES,
            br#"{"id":"climate-proposal-p-1","action":"reject"}"#,
            false,
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        let view = zone(&hub);
        assert!(view.proposal.is_none());
        assert_eq!(view.settings.rejected.len(), 1);
        assert_eq!(view.settings.state, ZoneState::Pending);
    }

    #[tokio::test(start_paused = true)]
    async fn stale_reply_is_ignored() {
        let (hub, _rx) = rig().await;
        // p-2 replaces p-1; the old buttons still on a phone point at p-1.
        seed(&hub, &proposal("p-2", 18.0));
        let replies: [&[u8]; 6] = [
            br#"{"id":"climate-proposal-p-1","action":"approve"}"#,
            br#"{"id":"climate-proposal-p-1","action":"reject"}"#,
            br#"{"id":"climate-proposal-p-2","action":"snooze"}"#,
            br#"{"id":"climate-safety-living-frost","action":"approve"}"#,
            br#"{"id":"climate-proposal-","action":"approve"}"#,
            b"not json",
        ];
        for reply in replies {
            hub.ingest_message(RESPONSES, reply, false);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        let view = zone(&hub);
        assert_eq!(view.settings, ZoneSettings::default());
        assert_eq!(view.proposal.as_ref().map(|p| p.id.as_str()), Some("p-2"));

        // The current proposal's own button still works.
        hub.ingest_message(
            RESPONSES,
            br#"{"id":"climate-proposal-p-2","action":"approve"}"#,
            false,
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(zone(&hub).settings.rules, rules(18.0));
    }

    #[tokio::test]
    async fn responses_filter_is_always_subscribed() {
        // The alarm may be switched on later and filters are fixed at connect, so the
        // responses topic is subscribed whether or not climate is on.
        let (on, _rx) = hub(true);
        assert!(on.extra_filters().contains(&RESPONSES.to_owned()));
        let (off, _rx) = hub(false);
        assert!(off.extra_filters().contains(&RESPONSES.to_owned()));
        // Disabled: a response is just a message nobody here cares about.
        assert!(off.ingest_message(RESPONSES, b"{}", false).is_empty());
    }

    #[tokio::test]
    async fn overrides_are_written_through_the_hub() {
        let (hub, mut rx) = rig().await;
        let id = hub.climate_view().unwrap().zones[0].entities[0].clone();
        let now = clock::now();
        let ov = |entity: Option<&str>, until: u64| nidoo_climate::doc::Override {
            entity: entity.map(str::to_owned),
            setpoint: Some(Setpoint::Temp(21.0)),
            mode: None,
            until,
            origin: nidoo_climate::doc::OverrideOrigin::Hand,
        };
        // A pending zone does not follow overrides: refused rather than silently ignored.
        assert!(matches!(
            hub.set_climate_override("living", ov(None, now + 600)).await,
            Err(HubError::Rejected(m)) if m == "zone is not active"
        ));
        assert!(zone(&hub).overrides.is_empty());
        hub.set_climate_zone(
            "living",
            ZoneSettings {
                state: ZoneState::Active,
                ..ZoneSettings::default()
            },
        )
        .unwrap();
        drain(&mut rx);
        hub.set_climate_override("living", ov(None, now + 600))
            .await
            .unwrap();
        let held = zone(&hub).overrides;
        assert_eq!(held.len(), 1);
        assert_eq!(
            held[0].origin,
            nidoo_climate::doc::OverrideOrigin::User,
            "an override set through the hub is the user's"
        );
        // A second whole-zone override replaces the first; a device one sits beside it.
        hub.set_climate_override("living", ov(None, now + 900))
            .await
            .unwrap();
        hub.set_climate_override("living", ov(Some(&id), now + 900))
            .await
            .unwrap();
        assert_eq!(zone(&hub).overrides.len(), 2);
        let published = drain(&mut rx);
        assert!(
            published
                .iter()
                .all(|p| p.topic == "nidoo/climate/zone/living/override" && p.retain)
        );

        for bad in [ov(None, now), ov(Some("climate.elsewhere"), now + 600)] {
            assert!(matches!(
                hub.set_climate_override("living", bad).await,
                Err(HubError::Rejected(_))
            ));
        }
        // The range error names the override's mode.
        let too_hot = nidoo_climate::doc::Override {
            setpoint: Some(Setpoint::Temp(50.0)),
            mode: Some(nidoo_climate::doc::Mode::Cool),
            ..ov(None, now + 600)
        };
        assert!(matches!(
            hub.set_climate_override("living", too_hot).await,
            Err(HubError::Rejected(m)) if m.starts_with("cool override:") && !m.contains("heat")
        ));
        assert!(matches!(
            hub.set_climate_override("nowhere", ov(None, now + 600))
                .await,
            Err(HubError::UnknownArea(_))
        ));

        hub.clear_climate_override("living").await.unwrap();
        assert!(zone(&hub).overrides.is_empty());
        let cleared = drain(&mut rx);
        assert!(
            cleared
                .last()
                .is_some_and(|p| p.payload.is_empty() && p.retain)
        );
    }

    const HOLD: &str = "nidoo/climate/zone/living/hold/set";

    /// The approval rig with its zone active, as a boost needs.
    async fn active_rig() -> (Arc<Hub>, mpsc::Receiver<Publish>) {
        let (hub, mut rx) = rig().await;
        hub.set_climate_zone("living", active()).unwrap();
        drain(&mut rx);
        (hub, rx)
    }

    /// Lets the task a command spawned run.
    async fn settle() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }

    fn entity_count(hub: &Hub) -> usize {
        hub.snapshot().entities.len()
    }

    #[tokio::test]
    async fn hold_command_sets_a_boost() {
        let (hub, _rx) = active_rig().await;
        let before = entity_count(&hub);
        let now = clock::now();
        let changes = hub.ingest_message(HOLD, br#"{"setpoint":22,"minutes":90}"#, false);
        assert!(changes.is_empty());
        settle().await;
        let held = zone(&hub).overrides;
        assert_eq!(held.len(), 1, "{held:?}");
        assert_eq!(held[0].entity, None);
        assert_eq!(held[0].setpoint, Some(Setpoint::Temp(22.0)));
        assert!(
            (now + 5400..=now + 5402).contains(&held[0].until),
            "{}",
            held[0].until - now
        );
        assert_eq!(entity_count(&hub), before);
    }

    #[tokio::test]
    async fn hold_command_empty_clears_boost_keeps_hand_hold() {
        let (hub, _rx) = active_rig().await;
        let id = zone(&hub).entities[0].clone();
        let now = clock::now();
        hub.ingest_message(HOLD, br#"{"setpoint":22,"minutes":60}"#, false);
        settle().await;
        hub.set_climate_override(
            "living",
            nidoo_climate::doc::Override {
                entity: Some(id.clone()),
                setpoint: Some(Setpoint::Temp(20.0)),
                mode: None,
                until: now + 600,
                origin: nidoo_climate::doc::OverrideOrigin::Hand,
            },
        )
        .await
        .unwrap();
        assert_eq!(zone(&hub).overrides.len(), 2);
        assert!(hub.ingest_message(HOLD, b"", false).is_empty());
        settle().await;
        let held = zone(&hub).overrides;
        assert_eq!(held.len(), 1, "{held:?}");
        assert_eq!(held[0].entity.as_deref(), Some(id.as_str()));
    }

    #[tokio::test]
    async fn hold_command_ignores_retained() {
        let (hub, _rx) = active_rig().await;
        let before = entity_count(&hub);
        let changes = hub.ingest_message(HOLD, br#"{"setpoint":22,"minutes":90}"#, true);
        assert!(changes.is_empty());
        settle().await;
        assert!(zone(&hub).overrides.is_empty());
        assert_eq!(entity_count(&hub), before);
    }

    #[tokio::test]
    async fn hold_command_ignores_bad_json() {
        let (hub, _rx) = active_rig().await;
        let before = entity_count(&hub);
        assert!(hub.ingest_message(HOLD, b"{nope", false).is_empty());
        settle().await;
        assert!(zone(&hub).overrides.is_empty());
        assert_eq!(entity_count(&hub), before);
    }

    #[tokio::test]
    async fn hold_command_ignores_pending_zone() {
        let (hub, _rx) = rig().await;
        assert!(
            hub.ingest_message(HOLD, br#"{"setpoint":22,"minutes":90}"#, false)
                .is_empty()
        );
        settle().await;
        assert!(zone(&hub).overrides.is_empty());
    }

    #[tokio::test]
    async fn hold_command_without_duration_is_rejected() {
        let (hub, _rx) = active_rig().await;
        let before = entity_count(&hub);
        assert!(
            hub.ingest_message(HOLD, br#"{"setpoint":22}"#, false)
                .is_empty()
        );
        settle().await;
        assert!(zone(&hub).overrides.is_empty());
        assert_eq!(entity_count(&hub), before);
    }
}
