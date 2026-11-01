# Broker fixtures

Retained MQTT messages recorded from the Gekkotron house broker (Mosquitto, Zigbee2MQTT + Home Assistant discovery) on 2026-10-18. They are test data only: nothing here is read at runtime, and the Z2M secrets (`mqtt.user`, `mqtt.password`, `network_key`, `pan_id`, `ext_pan_id`, `extended_pan_id`) are replaced by `***`.

Re-record with (credentials come from the environment only):

```bash
MQTT_HOST=<broker> MQTT_USER=<user> MQTT_PASS=<pass> bash tools/record-fixtures.sh
```

Layout:

- `bridge_info.json`, `bridge_devices.json`, `bridge_state.json`: `zigbee2mqtt/bridge/{info,devices,state}` (info is scrubbed).
- `discovery/<component>__<node>__<object_id>.json`: `homeassistant/<...>/config`, topic minus prefix and `/config`, `/` replaced by `__`. Tests rebuild the topic under any prefix (e.g. `oikos`).
- `states/<friendly_name with __>.json`: `zigbee2mqtt/<friendly_name>` device states.
- `ui/<unique_id>.json`, `ui/_areas.json`: `oikos/ui/...` overrides.
- `topics.txt`: every recorded topic as `<retain> <topic>`.
