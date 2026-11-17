# Picture URL fetch guard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The picture route's URL fetch can no longer reach the hub host's own loopback, link-local, unspecified, multicast or broadcast addresses, and returns only `image/*` bodies.

**Architecture:** A new `picture_fetch` module owns the address classifier, the URL gate (scheme, Frigate-origin exception, resolve-once, pin) and the fetch itself; `http/api.rs` calls it with the Frigate origin from `hub.media()`.

**Tech Stack:** Rust, reqwest (`ClientBuilder::resolve`, `no_proxy`), tokio `lookup_host`, `url`, axum test server.

**Spec:** `docs/superpowers/specs/2026-11-17-picture-ssrf-design.md`

## Global Constraints

- Only `http` and `https`.
- Blocked: IPv4 `0.0.0.0/8`, `127.0.0.0/8`, `169.254.0.0/16`, `224.0.0.0/4`, `255.255.255.255`; IPv6 `::`, `::1`, `fe80::/10`, `ff00::/8`; IPv4-mapped / compatible IPv6 judged as its IPv4.
- A URL on the Frigate origin (`media::origin_of` equality) skips the address check.
- Kept: no redirects, 10 s timeout, 5 MiB cap (`pictures::MAX_PICTURE_BYTES`).
- Reply must carry `Content-Type: image/*` (case-insensitive); missing or other → error, body dropped.
- Refusals and failures answer 502 `could not fetch picture URL`; the log carries the entity id and reason, never the body.
- Commit as Gekkotron; `CARGO_INCREMENTAL=0`.

## Review Focus

- A name that resolves to both a LAN and a loopback address (e.g. `localhost` with `::1` and `127.0.0.1`, or a rebinding DNS answer) → refused, because any blocked address refuses. Test: `localhost` refused in Task 1.
- `http://[::ffff:127.0.0.1]/` → refused. Test in Task 1.
- `Content-Type: IMAGE/PNG; charset=x` → accepted (case-insensitive prefix). Test in Task 1 (`is_image`).
- A URL with no host (`http:///x`, `file:///etc/passwd`) → refused, no panic. Test in Task 1.
- A Frigate origin given with the default port written out (`http://h:80`) still matches `http://h/…` → `origin_of` already normalises; covered by Task 1's Frigate case using `origin_of`.

---

### Task 1: `picture_fetch` module (classifier, gate, fetch)

**Files:**
- Create: `crates/nidoo/src/picture_fetch.rs`
- Modify: `crates/nidoo/src/lib.rs` (add `pub mod picture_fetch;`)

**Interfaces:**
- Produces:
  - `pub fn blocked(ip: std::net::IpAddr) -> bool`
  - `pub fn is_image(content_type: &str) -> bool`
  - `pub async fn target(url: &str, frigate_origin: Option<&str>) -> Result<Target, String>` where `pub struct Target { pub url: url::Url, pub pin: Option<std::net::SocketAddr> }` (`pin` is `None` on the Frigate origin).
  - `pub async fn fetch(url: &str, frigate_origin: Option<&str>) -> Result<(String, bytes::Bytes), String>`

- [ ] **Step 1: Write the failing unit tests** (`#[cfg(test)] mod tests` in the module)
  - `blocked_ranges`: `blocked` is true for `0.0.0.0`, `0.1.2.3`, `127.0.0.1`, `127.255.0.1`, `169.254.169.254`, `224.0.0.1`, `239.1.1.1`, `255.255.255.255`, `::`, `::1`, `fe80::1`, `ff02::1`, `::ffff:127.0.0.1`, `::ffff:169.254.169.254`; false for `192.168.1.20`, `10.0.0.5`, `172.17.0.2`, `8.8.8.8`, `fd00::1`, `2001:db8::1`, `::ffff:192.168.1.20`.
  - `is_image_cases`: true for `image/jpeg`, `IMAGE/PNG; charset=x`, ` image/webp`; false for `application/json`, `text/html`, `""`, `imag`.
  - `target_refuses` (tokio test): `Err` for `ftp://192.168.1.20/a.jpg`, `file:///etc/passwd`, `http:///x`, `http://127.0.0.1/a.jpg`, `http://[::1]/a.jpg`, `http://[::ffff:127.0.0.1]/a.jpg`, `http://169.254.169.254/latest/meta-data`, `http://localhost:5000/a.jpg`, `nope`.
  - `target_allows_lan`: `target("http://192.168.1.20:8080/a.jpg", None)` is `Ok` with `pin == Some("192.168.1.20:8080")`.
  - `target_allows_the_frigate_origin_on_loopback`: `target("http://127.0.0.1:5000/api/x.jpg", Some("http://127.0.0.1:5000"))` is `Ok` with `pin == None`; with `Some("http://127.0.0.1:5001")` it is `Err`.

- [ ] **Step 2: Run, expect compile failure**

Run: `CARGO_INCREMENTAL=0 cargo test -p nidoo --lib picture_fetch`
Expected: FAIL (module empty / functions missing).

- [ ] **Step 3: Implement**

`blocked`: map `V6` through `to_ipv4_mapped()` / `to_ipv4()` (compatible, but not `::`/`::1`) first; then `is_unspecified || is_loopback || is_link_local || is_multicast || is_broadcast` plus `octets()[0] == 0` for v4; for v6 `is_unspecified || is_loopback || is_multicast || (segments()[0] & 0xffc0) == 0xfe80`.
`target`: parse with `url::Url`; scheme must be http/https; host must exist; if `media::origin_of(url) == frigate_origin` return `pin: None`; else resolve `(host_str, port_or_known_default)` with `tokio::net::lookup_host` (IP literals resolve to themselves; strip IPv6 brackets via `url.host()`), refuse on empty answer or any `blocked` address, pin the first.
`fetch`: build `reqwest::Client` with `timeout(10 s)`, `redirect(Policy::none())`, `no_proxy()`, and `.resolve(host, pin)` when pinned; `error_for_status`; require `is_image` on the `Content-Type` header (missing → `Err("reply is not an image")`); stream with the 5 MiB cap as today.

- [ ] **Step 4: Run, expect pass**

Run: `CARGO_INCREMENTAL=0 cargo test -p nidoo --lib picture_fetch`
Expected: all pass.

- [ ] **Step 5: Commit** (`fix(pictures): the URL fetch refuses the hub's own host and non-image replies` once Task 2 lands; commit here as WIP-free code only if Task 2 follows in the same session — one commit for both is fine.)

### Task 2: route wiring, integration tests, docs

**Files:**
- Modify: `crates/nidoo/src/http/api.rs` (`entity_picture`, delete `fetch_picture`)
- Test: `crates/nidoo/tests/http.rs` (next to `oversized_picture_is_dropped`)
- Modify: `docs/users/security.md`, `docs/users/cameras.md`

**Interfaces:**
- Consumes: `picture_fetch::fetch(url, frigate_origin)`.

- [ ] **Step 1: Write the failing integration tests**

Helper: `async fn picture_server() -> SocketAddr` serving on `127.0.0.1:0` `/pic.jpg` → `image/jpeg` body `[0xFF,0xD8,9]` and `/data.json` → `application/json` `{"secret":1}`. Image entity: ingest `nidoo/image/door/config` with `{"unique_id":"door_image","name":"Door","url_topic":"img/url"}`, then `ingest_message("img/url", url, false)`.
  - `picture_url_on_loopback_is_refused`: default config, URL `http://<addr>/pic.jpg` → 502.
  - `picture_url_on_the_frigate_origin_is_served`: config with `frigate.url = Some("http://<addr>")` → 200, `content-type: image/jpeg`, body `[0xFF,0xD8,9]`.
  - `non_image_picture_reply_is_refused`: same config, URL `/data.json` → 502 and the body does not contain `secret`.

- [ ] **Step 2: Run, expect the loopback and non-image tests to fail**

Run: `CARGO_INCREMENTAL=0 cargo test -p nidoo --test http picture`
Expected: `picture_url_on_loopback_is_refused` and `non_image_picture_reply_is_refused` FAIL (200 today).

- [ ] **Step 3: Wire the route**: `entity_picture` passes `hub.media().as_deref().map(|m| m.origin())` to `picture_fetch::fetch`; remove `fetch_picture`.

- [ ] **Step 4: Run, expect pass**: same command, all pass.

- [ ] **Step 5: Docs**: `security.md` "What else to know": one bullet saying the picture fetch of a camera or image entity's URL refuses loopback, link-local (cloud metadata), unspecified, multicast and broadcast addresses (the Frigate address excepted), connects to the address it checked, follows no redirect and returns only `image/*` replies. `cameras.md`: one line pointing at it.

- [ ] **Step 6: Verify and commit**

Run: `cargo fmt --all --check && CARGO_INCREMENTAL=0 cargo clippy --workspace --all-targets -- -D warnings && CARGO_INCREMENTAL=0 cargo test -p nidoo`
Expected: clean, all pass (DB tests skip locally).
Commit spec, plan, code and docs as Gekkotron.
