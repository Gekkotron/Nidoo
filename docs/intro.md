---
slug: /
title: Overview
sidebar_position: 1
description: What Nidoo is, what it replaces, the pieces it is made of and how they fit together.
---

Nidoo is the home-automation hub that replaces Jeedom. It runs on the home server next to Mosquitto, owns the device registry built from Zigbee2MQTT and Home Assistant MQTT discovery, and is the brain behind the Nidoo Android app. It is configured through conversation: an MCP server lets an AI assistant inspect and change the house instead of a rules editor.

Status: v1 hub with its web UI: registry, API, WebSocket, MCP and the browser app.

## The pieces

- **The hub** (`oikos-os`): one binary with the device registry, the HTTP API, the WebSocket, the MCP server, the flow engine, the climate engine, history, push notifications and the watchdogs.
- **The web UI**: the browser app the hub serves on its own port.
- **The Android app** (Nidoo): the phone client for the same house.
- **The MCP server**: lets AI assistants such as Claude Code, Codex and Claude Desktop read and change the house.

## How it fits together

- Zigbee2MQTT talks to the Zigbee devices and publishes them over MQTT with Home Assistant discovery.
- Mosquitto is the MQTT broker between Zigbee2MQTT, the hub and everything else.
- The hub reads the discovery, builds the registry and sends commands back through the broker.
- The web UI, the Android app and AI assistants talk to the hub.

## Where to next

- [Getting started](users/getting-started.md): run the hub from source and open the web UI.
- [Install on Linux](users/install-on-linux.md): the Docker install on a Linux host.
- [Flows](users/flows.md): automations built from triggers, checks, waits and actions.
- [Talking to the house (MCP)](users/assistants-mcp.md): connect an AI assistant.
- [Settings reference](reference/configuration.md): where the settings live and every field.
- [Development](contributors/development.md): build, test and work on the code.

## License

Apache-2.0, Copyright 2026 Gekkotron.
