---
title: Talking to the house (MCP)
sidebar_position: 7
description: Connect an AI assistant to the hub over MCP and ask it, in plain words, to inspect or change the house.
---

The hub runs an MCP server, so an AI assistant can inspect and change the house instead of a rules editor. The server is served at `http://<host>:8081/mcp` (streamable HTTP), on the same port as the REST API. Like that API, it is LAN-trusted with no auth in v1; see [Security](security.md).

The assistant gets the house's tools and one resource, `nidoo://snapshot`, which is the whole house as one JSON document. The full tool list is in [MCP tools](../reference/mcp-tools.md).

## Connect a client

Claude Code:

```sh
claude mcp add --transport http nidoo http://<host>:8081/mcp
```

Codex CLI, in `~/.codex/config.toml`:

```toml
[mcp_servers.nidoo]
url = "http://<host>:8081/mcp"
```

Claude Desktop only speaks stdio, so it launches `nidoo mcp-stdio`, a proxy that relays every request to the running hub. In its config JSON, with the path of a built binary (`cargo build --release -p nidoo`):

```json
{
  "mcpServers": {
    "nidoo": {"command": "<path>/nidoo", "args": ["mcp-stdio", "--server", "http://<host>:8081"]}
  }
}
```

The proxy exits with `cannot reach the Nidoo MCP server at …` when the hub is down; its logs go to stderr, stdout carries the protocol.

The web UI's Settings screen builds the same snippets from the address you opened it at (see [The web UI](web-ui.md#settings)).

## When a call fails

A failed tool call answers in-band (`isError: true` with the reason) so the assistant can retry with better arguments.

## Example requests

You can ask in plain words, for example:

- "Which lights are on in the living room?"
- "Turn off every light on the ground floor."
- "Send my phone a notification saying the laundry is done."
- "Say dinner is ready in the kitchen." (the `speak` tool, on the [sound satellites](sound.md#speak-in-a-room))
- "Make a flow that turns the hall light on when the hall motion sensor sees movement, and off five minutes after the last movement."
- "Show me the energy the house used today."
- "Hold the bedroom at 19 degrees for the next two hours."
- "How is the hub doing? Has it restarted recently?" (the `system_status` tool: uptime, restarts, the server's figures and the services; see [System health](system-health.md))

Flows made this way appear on the [Flows](flows.md) page. Energy questions need the database connected (it is, after setup), and climate requests need the climate engine on (see [Climate](climate.md)); without them the tools answer with an error. An assistant can also read and change the service settings (weather, solar forecast, cameras, notifications, climate and the advanced ones) with `get_settings` and `set_settings`; see [MCP tools](../reference/mcp-tools.md).
