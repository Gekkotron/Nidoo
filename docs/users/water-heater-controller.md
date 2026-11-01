---
title: WaterHeaterController
sidebar_position: 3.5
description: Bring the WaterHeaterController, an Arduino Mega dimmer for an electric water heater, into the hub over MQTT.
---

The [WaterHeaterController](https://github.com/Gekkotron/WaterHeaterController) drives an electric water heater through three triac channels, 0 to 100 % each, and measures the tank and cold-water inlet temperatures. It runs on an Arduino Mega 2560 with a W5100 Ethernet chip (or the STM32F103 board of the same project) and talks MQTT over Ethernet; it is not a Zigbee device.

## Topics

| Topic | Direction | Content |
|---|---|---|
| `water-heater/data` | from the board, every 4 s | JSON readings: `triac_0_power` to `triac_2_power`, `adc_result_mv` (tank probe), `temp_0`, `temp_1`, `measuredFrequency`, `frequencyError` |
| `water-heater/control` | to the board | JSON `{"power0": n, "power1": n, "power2": n}`; any subset of the keys |

## Discovery

The firmware publishes no discovery, so publish it once, retained, for its existing topics:

```sh
mosquitto_pub -h <broker> -u <user> -P <password> -r -t oikos/device/water_heater/config -m '{"dev":{"ids":["water_heater_controller"],"name":"Water heater","mf":"Gekkotron","mdl":"WaterHeaterController"},"stat_t":"water-heater/data","cmps":{"heater":{"p":"water_heater","name":null,"uniq_id":"water_heater","unit_of_meas":"°C","modes":[],"curr_temp_t":"water-heater/data","curr_temp_tpl":"{{ (80 + (0.35 - value_json.adc_result_mv / 1000) * 86.67) | round(1) }}","mode_stat_t":"water-heater/data","mode_stat_tpl":"{{ \"heat\" if (value_json.triac_0_power + value_json.triac_1_power + value_json.triac_2_power) > 0 else \"off\" }}","json_attr_t":"water-heater/data","json_attr_tpl":"{{ {\"inlet_temperature\": value_json.temp_0 | round(1), \"power\": ((value_json.triac_0_power + value_json.triac_1_power + value_json.triac_2_power) / 3) | round | int} | tojson }}"},"tank_temp":{"p":"sensor","name":"Tank temperature","uniq_id":"water_heater_tank_temp","dev_cla":"temperature","stat_cla":"measurement","unit_of_meas":"°C","val_tpl":"{{ (80 + (0.35 - value_json.adc_result_mv / 1000) * 86.67) | round(1) }}"},"power":{"p":"number","name":"Power","uniq_id":"water_heater_power","cmd_t":"water-heater/control","cmd_tpl":"{\"power0\": {{ value | int }}, \"power1\": {{ value | int }}, \"power2\": {{ value | int }}}","val_tpl":"{{ ((value_json.triac_0_power + value_json.triac_1_power + value_json.triac_2_power) / 3) | round | int }}","min":0,"max":100,"step":1,"unit_of_meas":"%"},"power_0":{"p":"number","name":"Power 1","uniq_id":"water_heater_power_0","cmd_t":"water-heater/control","cmd_tpl":"{\"power0\": {{ value | int }}}","val_tpl":"{{ value_json.triac_0_power }}","min":0,"max":100,"step":1,"unit_of_meas":"%","ent_cat":"config"},"power_1":{"p":"number","name":"Power 2","uniq_id":"water_heater_power_1","cmd_t":"water-heater/control","cmd_tpl":"{\"power1\": {{ value | int }}}","val_tpl":"{{ value_json.triac_1_power }}","min":0,"max":100,"step":1,"unit_of_meas":"%","ent_cat":"config"},"power_2":{"p":"number","name":"Power 3","uniq_id":"water_heater_power_2","cmd_t":"water-heater/control","cmd_tpl":"{\"power2\": {{ value | int }}}","val_tpl":"{{ value_json.triac_2_power }}","min":0,"max":100,"step":1,"unit_of_meas":"%","ent_cat":"config"},"temp_0":{"p":"sensor","name":"Cold water inlet temperature","uniq_id":"water_heater_temp_0","dev_cla":"temperature","stat_cla":"measurement","unit_of_meas":"°C","val_tpl":"{{ value_json.temp_0 | round(1) }}"},"temp_1":{"p":"sensor","name":"Triac temperature","uniq_id":"water_heater_temp_1","dev_cla":"temperature","stat_cla":"measurement","unit_of_meas":"°C","val_tpl":"{{ value_json.temp_1 | round(1) }}","ent_cat":"diagnostic"},"frequency":{"p":"sensor","name":"Mains frequency","uniq_id":"water_heater_frequency","dev_cla":"frequency","stat_cla":"measurement","unit_of_meas":"Hz","ent_cat":"diagnostic","val_tpl":"{{ value_json.measuredFrequency | round(1) }}"},"frequency_error":{"p":"binary_sensor","name":"Mains frequency error","uniq_id":"water_heater_frequency_error","dev_cla":"problem","ent_cat":"diagnostic","pl_on":"1","pl_off":"0","val_tpl":"{{ value_json.frequencyError }}"}}}'
```

It creates one device, **Water heater**, with:

- **Water heater**: the `water_heater` card, with the tank temperature, `heat` or `off` from the triacs, and the inlet temperature and average power as attributes.
- **Power**: sets the three channels in one message (`power0`, `power1` and `power2` together) and reads back their average. Drive this one from flows.
- **Power 1** to **Power 3**: one channel each, as configuration entities.
- **Tank temperature**, **Cold water inlet temperature**, and the diagnostic **Triac temperature**, **Mains frequency** and **Mains frequency error**.

## Things to know

- The board answers a command only with its next `water-heater/data` message, up to about 4 s later plus its loop time. The hub's default `commands.confirm_timeout_ms` of 5000 can expire first and report a command as failed although the board applied it; set it to 8000 in the [advanced settings](../reference/configuration.md#commands) (`PUT /api/v1/settings/advanced` or the `set_settings` MCP tool; the hub restarts).
- The board turns every channel off when it has received no power command for 15 minutes. A flow that drives it should send a value regularly, for example on every grid power reading, not only when the value changes.
- Solar surplus heating is a flow: see [Compute a value](flows.md#compute-a-value) in Flows.
