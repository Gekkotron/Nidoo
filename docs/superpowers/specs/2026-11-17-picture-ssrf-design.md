# Picture URL fetch: refuse the hub's own host (short design)

Security audit 2026-11-17, item 4 (P1): a camera or image entity's `url_topic` / `url_template` becomes a URL the hub fetches with no host check, and `GET /api/v1/entities/{id}/picture` returns the body.

## Why

Any broker login (the app login in the setup QR included) can publish a discovery config with a `url_topic`, then a URL. The hub runs with `network_mode: host` and has no API login, so the LAN already has full control. What the fetch adds is reach: the host's own `localhost` services, link-local and cloud-metadata addresses, and read-back of any reply body, JSON included.

## Goals, non-goals

- Goal: the picture fetch can no longer reach the hub host's loopback, link-local (169.254.169.254 included), unspecified, multicast or broadcast addresses, and returns only image bodies.
- Goal: LAN cameras keep working (private RFC 1918 / ULA addresses stay allowed), and so does a Frigate on the same host as the hub.
- Non-goal: an allowlist setting (option 2) or dropping URL pictures (option 3).
- Anti-goal: a check that a DNS answer can dodge (check one IP, connect to another).

## Approaches

1. **Block the hub's own host and metadata, pin the IP, image bodies only.** Picked: it keeps real LAN cameras working and closes what the LAN does not already have.
2. Allowlist (Frigate origin plus a new "Picture hosts" setting): tighter, but breaks URL cameras until configured and needs a screen and docs.
3. Drop URL pictures (bytes only): simplest, but undoes the HA `url_topic` support the entity-types design chose.

## Design

- `fetch_picture` takes the URL and the Frigate origin (`hub.media().map(|m| m.origin())`).
- Only `http` and `https`; anything else is refused.
- A URL on the Frigate origin (scheme, host and port, as `media::origin_of` normalises them) skips the address check: the user configured it, and it may be `localhost`.
- Otherwise the host is resolved once (an IP literal as is, a name with `tokio::net::lookup_host`). If any resolved address is blocked, the fetch is refused; else the client is pinned to that address with `ClientBuilder::resolve`, so a second DNS answer cannot move the connection.
- Blocked: IPv4 `0.0.0.0/8`, `127.0.0.0/8`, `169.254.0.0/16`, `224.0.0.0/4`, `255.255.255.255`; IPv6 `::`, `::1`, `fe80::/10`, `ff00::/8`, and an IPv4-mapped / compatible address judged as its IPv4.
- The client ignores proxy environment variables (`no_proxy()`), so the pin is the address actually dialled.
- Kept: no redirects, 10 s timeout, 5 MiB cap.
- The reply must say `Content-Type: image/*`; anything else, a missing header included, fails with 502 and the body is dropped. (Builder's call: before, a missing header defaulted to `image/jpeg`.)
- A refused URL logs a warning with the entity id and the reason (never the body) and answers 502, as a fetch failure does today.
- Docs: `docs/users/security.md` and `docs/users/cameras.md` gain one line each on what the picture fetch refuses.

## Tests (TDD)

- Unit: the address classifier over each blocked range and a few allowed ones (`192.168.1.20`, `10.0.0.5`, `fd00::1`, a public address), and the IPv4-mapped case.
- Unit: the URL gate refuses `ftp://`, `http://127.0.0.1/`, `http://[::1]/`, `http://169.254.169.254/`, `http://localhost/` and allows the Frigate origin on `127.0.0.1`.
- Integration (`tests/http.rs`): an image entity whose URL points at a local test server on `127.0.0.1` answers 502; the same entity with that server as the Frigate origin answers 200 with the image; a non-image reply on the Frigate origin answers 502.

## Left to the builder

Where the gate lives (a small module next to `pictures.rs` is the natural place) and the exact error strings.
