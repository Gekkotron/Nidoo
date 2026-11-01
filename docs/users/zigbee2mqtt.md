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

Restart Zigbee2MQTT, then set the Nidoo app's discovery prefix to `oikos` as well.

Devices that Zigbee2MQTT does not cover reach the hub through a bridge publishing the same discovery: see [Adding a device](../contributors/adding-a-device.md). An MQTT device with no discovery of its own, such as the [WaterHeaterController](water-heater-controller.md), only needs its discovery published once by hand.
