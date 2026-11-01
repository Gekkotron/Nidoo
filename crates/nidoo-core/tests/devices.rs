//! `Device` and `BridgeInfo` read the recorded Zigbee2MQTT bridge topics.

mod common;

use nidoo_core::model::{Availability, BridgeInfo, Device};

#[test]
fn parses_recorded_device_list_without_coordinator() {
    let raw = common::fixture("bridge_devices.json");
    let devices = Device::parse_list(raw.as_bytes()).expect("bridge/devices parses");
    assert_eq!(devices.len(), 66);
    assert!(devices.iter().all(|d| d.device_type != "Coordinator"));
    assert!(
        devices
            .iter()
            .all(|d| d.id == format!("zigbee2mqtt_{}", d.ieee))
    );
    assert!(devices.iter().all(|d| !d.friendly_name.is_empty()));
}

#[test]
fn bridge_info_and_state_apply() {
    let mut bridge = BridgeInfo::default();
    assert_eq!(bridge.state, Availability::Unknown);
    bridge.apply_info(common::fixture("bridge_info.json").as_bytes());
    assert_eq!(bridge.version.as_deref(), Some("2.14.1"));
    assert_eq!(bridge.coordinator.as_deref(), Some("ZStack3x0"));
    assert!(!bridge.permit_join);
    bridge.apply_state(common::fixture("bridge_state.json").as_bytes());
    assert_eq!(bridge.state, Availability::Online);
    bridge.apply_state(br#"{"state":"offline"}"#);
    assert_eq!(bridge.state, Availability::Offline);
}
