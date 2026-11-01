---
title: Zigbee2MQTT
sidebar_position: 3
description: Point Zigbee2MQTT's Home Assistant discovery at the hub and publish the devices it does not cover.
---

The hub discovers entities under the `oikos` prefix. In Zigbee2MQTT's `configuration.yaml`:

```yaml
homeassistant:
  enabled: true
  discovery_topic: oikos
```

Restart Zigbee2MQTT, then set the Oikos app's discovery prefix to `oikos` as well.

Devices that Zigbee2MQTT does not cover reach the hub through a bridge publishing the same discovery: see [Adding a device](../contributors/adding-a-device.md).

## WaterHeaterController

The [WaterHeaterController](https://github.com/Gekkotron/WaterHeaterController) firmware publishes no discovery, so publish it once, retained, for its existing `water-heater/*` topics:

```sh
mosquitto_pub -h <broker> -u <user> -P <password> -r -t oikos/device/water_heater/config -m '{"dev":{"ids":["water_heater_controller"],"name":"Water heater","mf":"Gekkotron","mdl":"WaterHeaterController"},"stat_t":"water-heater/data","cmps":{"heater":{"p":"water_heater","name":null,"uniq_id":"water_heater","unit_of_meas":"°C","modes":[],"curr_temp_t":"water-heater/data","curr_temp_tpl":"{{ (80 + (0.35 - value_json.adc_result_mv / 1000) * 86.67) | round(1) }}","mode_stat_t":"water-heater/data","mode_stat_tpl":"{{ \"heat\" if (value_json.triac_0_power + value_json.triac_1_power + value_json.triac_2_power) > 0 else \"off\" }}","json_attr_t":"water-heater/data","json_attr_tpl":"{{ {\"inlet_temperature\": value_json.temp_0 | round(1), \"power\": ((value_json.triac_0_power + value_json.triac_1_power + value_json.triac_2_power) / 3) | round | int} | tojson }}"},"tank_temp":{"p":"sensor","name":"Tank temperature","uniq_id":"water_heater_tank_temp","dev_cla":"temperature","stat_cla":"measurement","unit_of_meas":"°C","val_tpl":"{{ (80 + (0.35 - value_json.adc_result_mv / 1000) * 86.67) | round(1) }}"},"power":{"p":"number","name":"Power","uniq_id":"water_heater_power","cmd_t":"water-heater/control","cmd_tpl":"{\"power0\": {{ value | int }}, \"power1\": {{ value | int }}, \"power2\": {{ value | int }}}","val_tpl":"{{ ((value_json.triac_0_power + value_json.triac_1_power + value_json.triac_2_power) / 3) | round | int }}","min":0,"max":100,"step":1,"unit_of_meas":"%"},"power_0":{"p":"number","name":"Power 1","uniq_id":"water_heater_power_0","cmd_t":"water-heater/control","cmd_tpl":"{\"power0\": {{ value | int }}}","val_tpl":"{{ value_json.triac_0_power }}","min":0,"max":100,"step":1,"unit_of_meas":"%","ent_cat":"config"},"power_1":{"p":"number","name":"Power 2","uniq_id":"water_heater_power_1","cmd_t":"water-heater/control","cmd_tpl":"{\"power1\": {{ value | int }}}","val_tpl":"{{ value_json.triac_1_power }}","min":0,"max":100,"step":1,"unit_of_meas":"%","ent_cat":"config"},"power_2":{"p":"number","name":"Power 3","uniq_id":"water_heater_power_2","cmd_t":"water-heater/control","cmd_tpl":"{\"power2\": {{ value | int }}}","val_tpl":"{{ value_json.triac_2_power }}","min":0,"max":100,"step":1,"unit_of_meas":"%","ent_cat":"config"},"temp_0":{"p":"sensor","name":"Cold water inlet temperature","uniq_id":"water_heater_temp_0","dev_cla":"temperature","stat_cla":"measurement","unit_of_meas":"°C","val_tpl":"{{ value_json.temp_0 | round(1) }}"},"temp_1":{"p":"sensor","name":"Triac temperature","uniq_id":"water_heater_temp_1","dev_cla":"temperature","stat_cla":"measurement","unit_of_meas":"°C","val_tpl":"{{ value_json.temp_1 | round(1) }}","ent_cat":"diagnostic"},"frequency":{"p":"sensor","name":"Mains frequency","uniq_id":"water_heater_frequency","dev_cla":"frequency","stat_cla":"measurement","unit_of_meas":"Hz","ent_cat":"diagnostic","val_tpl":"{{ value_json.measuredFrequency | round(1) }}"},"frequency_error":{"p":"binary_sensor","name":"Mains frequency error","uniq_id":"water_heater_frequency_error","dev_cla":"problem","ent_cat":"diagnostic","pl_on":"1","pl_off":"0","val_tpl":"{{ value_json.frequencyError }}"}}}'
```

`Power` sets the three triac channels in one message (`power0`, `power1` and `power2` together) and reads back their average; drive it from flows. `Power 1` to `Power 3` stay available as configuration entities for setting one channel alone.
