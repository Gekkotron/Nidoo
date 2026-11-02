---
title: Getting started
sidebar_position: 1
description: Install the hub with one command, set up your home in the browser wizard and open the web UI.
---

You need a Linux host with Docker and its Compose plugin. Zigbee2MQTT can run on that host or elsewhere on your network.

## Install

```sh
git clone https://github.com/Gekkotron/Nidoo.git
cd Nidoo
docker compose up -d --build
docker compose logs hub
```

This builds the hub and starts a broker and a database beside it. The log shows `Open http://<ip>:8081 to set up your home`: open that address in a browser. The details, and what to do if you already run a broker or a database, are in [Install on Linux](install-on-linux.md).

Already run a broker or database? See [Bring your own broker and database](install-on-linux.md#bring-your-own-broker-and-database) before you run the command above.

## The setup wizard

The wizard has six steps. Each one saves when you press Next, so closing the tab loses nothing, and Back is always allowed.

1. **Welcome**: pick the language (English or French) and your home's city and country. The hub looks up the latitude, longitude and time zone, which you can correct. Sun-based automations need this even when weather is off.
2. **MQTT broker**: host, port, user and password of the broker. They are pre-filled for the broker that came with Nidoo. **Test connection** must succeed before Next. The client id and the Zigbee2MQTT base topic are under Advanced.
3. **Database**: the TimescaleDB connection, pre-filled for the bundled one. The test checks the connection and the `timescaledb` extension and prepares the tables. This step cannot be skipped, because the hub keeps its settings and history there.
4. **Devices**: what the hub has found through MQTT discovery, with a live count. You can name devices and set their rooms here. With nothing found, check that Zigbee2MQTT publishes to this broker and under the base topic shown; see [Zigbee2MQTT](zigbee2mqtt.md).
5. **Services**: optional features as cards, each Set up, Not set up or Skipped: Weather, Solar forecast, Energy, Cameras, Notifications, Climate and Alarm. Do only the ones you want.
6. **Done**: a summary and **Open my house**.

Everything can be changed later in Settings. **Run setup again** there reopens the wizard and never wipes anything.

## Next steps

- [Zigbee2MQTT](zigbee2mqtt.md): point Zigbee2MQTT's discovery at the hub.
- [The web UI](web-ui.md): the screens the hub serves.
- [Talking to the house (MCP)](assistants-mcp.md): connect an AI assistant.
- [Cameras (Frigate)](cameras.md), [Push notifications](push-notifications.md) and [History](history.md): the optional parts.
- [Security](security.md): read this before the hub is reachable from anywhere but your LAN.
- Running from source instead of Docker: [Development](../contributors/development.md).
