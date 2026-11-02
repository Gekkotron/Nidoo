---
title: Watchdogs
sidebar_position: 12
description: Two opt-in watchdogs that raise system alerts when the encryption gateway changes state or the broker denies an MQTT action.
---

Two opt-in watchdogs raise `system` alerts.

They have no Settings page. Turn them on through `PUT /api/v1/settings/advanced` or the `set_settings` MCP tool: read the `advanced` section with `get_settings`, set `watchdog.gateway.enabled` or `watchdog.acl.enabled` to `true`, and send the whole section back. The hub restarts to start them. The fields are listed in the [Settings reference](../reference/configuration.md#advanced).

## Gateway health

With `watchdog.gateway.enabled` on, the hub subscribes to `<prefix>/+` (default `encryption-gateway/health`), where the gateway publishes one retained state per component (`{"status": "ok" | "fail" | "config_error", ...}`). The hub alerts on a change only: `ok` to non-ok ("<component> down", `warning`), non-ok to `ok` ("<component> recovered", `info`), `fail` to `config_error`, and a first message that is not `ok`. A first `ok`, and a retained state replayed unchanged on reconnect, stay silent.

## Broker ACL

Mosquitto acknowledges an ACL-denied PUBLISH and drops it with only a log line. With `watchdog.acl.enabled` on, the hub turns each `Denied PUBLISH/SUBSCRIBE/UNSUBSCRIBE` line (and the older `Client X is not authorised to ...` wording) into a `warning` alert "MQTT ACL denial". A client, topic and action alert once per `cooldown` seconds (default 300); later denials are counted and named in the next alert (`+N more in last 300s`). Two sources:

- `source = "topic"` (default) reads `log_topic` (`$SYS/broker/log/#`; needs `log_dest topic` in `mosquitto.conf`, and a broker that forwards ACL denials there);
- `source = "file"` tails `log_file` from its end, follows rotation and truncation, and needs the hub to read the Mosquitto log; a file source without `log_file` is a config error.

```json
{
  "watchdog": {
    "gateway": { "enabled": true, "prefix": "encryption-gateway/health" },
    "acl": { "enabled": true, "source": "topic", "log_topic": "$SYS/broker/log/#", "log_file": null, "cooldown": 300 }
  }
}
```

That is only the `watchdog` part of the `advanced` document, shown for the shape. Always send the whole section back, because what is left out takes its default.

The broker user needs read on `<prefix>/+` and, for the topic source, on `$SYS/broker/log/#`.

