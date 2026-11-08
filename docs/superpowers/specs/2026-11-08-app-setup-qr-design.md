# Phone setup QR code in the hub — design

Date: 2026-11-08 · Status: draft, awaiting owner approval

## Intent

The Android app sets itself up from a QR code: the user taps **Scan setup code** and the phone
gets the broker, the remote gateway and Frigate with no typing. Today that code only comes from
`tools/setup-qr.py` in the app repo, run by hand from a JSON file holding secrets. The hub
already knows almost everything the code needs, so the hub should draw it.

The owner wants it shown **at the end of onboarding**, to send people on to the mobile app, and
again in **Settings**, for a second phone or to add remote access later. The one thing the hub
does not know is the remote gateway, so the hub gets **two new inputs: gateway URL and gateway
key**.

## Goals

- The wizard's Done step shows the setup QR code with a short "get the app on your phone" text.
- A new Settings page, **Mobile app**, shows the same block.
- Both places carry the two optional inputs, gateway URL and gateway key. When both are set,
  the code includes remote access; when not, the phone works at home only.
- A code the hub draws is always one the app accepts (same rules as `OnboardingPayload.parse`).

## Non-goals

- No change to the Android app or to the `v:1` format.
- No download link to the app. It ships through Firebase App Distribution for now, which is
  invite-based and specific to the owner's project, so the text says "install the Nidoo app"
  without a link (*builder's call / mine*: a link setting can come later, with a store listing).
- No login on the hub's web page (it has none today; out of scope here).
- The hub does not run, test or watch the gateway (the gateway watchdog stays as it is).

## Anti-goals

- A secret (broker password, gateway key, Frigate password) in a log line, an error message,
  a settings read, or a tracked file.
- A code the app rejects ("Not a Nidoo code" / invalid field) because the hub drew something the
  app's parser refuses.
- The owner's host names, addresses or Tailscale names anywhere in tracked text (examples use
  `house.example.ts.net`, `192.0.2.10`).

## The payload (`v:1`, as `OnboardingPayload.parse` in Nidoo-app reads it)

```json
{
  "v": 1,
  "local":   { "host": "...", "port": 1883, "tls": false, "ws": false, "user": "...", "pass": "..." },
  "gateway": { "url": "https://...", "key": "<64 hex>", "broker_host": "..." },
  "push":    { "token_topic": "<push prefix>/tokens/{installId}" },
  "frigate": { "url": "http://...", "user": "...", "pass": "..." }
}
```

- **`local`**: from the MQTT config the hub actually runs with (the assembled `Config.mqtt`),
  not from `hub.json` alone. In bundled mode `hub.json` `mqtt` can be `None`, and the host
  (`localhost`, `routes.rs` `BUNDLED_HOST`), user and password then come from `bundled.json`
  (`bundled::load`). Port, user, password; `tls` and `ws` false (the hub speaks plain MQTT).
  `user`/`pass` left out when empty.
  **Phone-reachable host** (*mine*): the hub may reach its broker through an address a phone
  cannot use: `localhost`, `127.0.0.0/8`, `::1`, or a name with no dot that is not an IP (a
  Docker service name). The **hub** then uses the host from the request's `Host` header instead,
  with any port removed. That is the address the browser opened the hub on. The bundled compose
  runs Mosquitto with `network_mode: host`, so the broker listens on 1883 at that same address.
  Otherwise the broker host is used as it is. The swap happens on the hub, in one pure function
  that is unit-tested, so `local.host` and `gateway.broker_host` always agree.
- **`prefix`**: left out. The app's default is `homeassistant`, where Zigbee2MQTT publishes, and
  that is what the working hand-made code uses today.
- **`gateway`**: only when both URL and key are stored. `broker_host` is the same
  phone-reachable host as `local.host`: the broker's LAN address, which a gateway on the LAN
  can reach. *Mine.*
- **`push`**: `token_topic` built from the hub's push prefix (`<prefix>/tokens/{installId}`),
  so a hub with a non-default push prefix still receives the phone's token. *Mine*.
- **`frigate`**: only when Cameras is set up (`frigate.url` non-empty). The user and password
  come from `FrigateConfig.user` / `.password` in the `frigate` settings row, which stores them
  as plain text, so there is nothing to decrypt. User/pass are left out when empty. A password with no user is left out entirely
  (the app refuses `pass` without `user`).
- **`history`**: left out (the app finds the history API at `http://<broker host>:8090`).

The JSON is compact (no spaces) to keep the QR small.

## Storage and API

- A new settings row **`mobile`** in the `settings` table:
  `{ "gateway_url": Option<String>, "gateway_key": Option<String> }`. It is handled like
  `system` (`sections.rs` `SystemSection`, `Hub::set_system_notify`): it is added to
  `Sections` and `SECTION_KEYS` so it is stored, loaded and cached with the others. It is also
  **left out of the generic restart routes**: `known()` in `setup/settings.rs` excludes it the
  way it excludes `"system"`. It gets its own handlers instead, which save the row and update
  the hub's in-memory copy without a restart.
- Mode: every route below follows the existing **Run-mode** rule (409 "finish MQTT and
  database setup first" otherwise; `settings.rs` `ctx.mode != Mode::Run`). The Done step comes
  after MQTT and the database are set, so it runs in Run mode with setup not yet finished, and
  the routes work there.
- `GET /api/v1/settings/mobile` returns `{ "gateway_url": "...", "gateway_key": {"set": bool} }`
  — the key is never read back, the same `SecretView` rule as every other secret.
- `PUT /api/v1/settings/mobile` takes `{ "gateway_url": string|null, "gateway_key": Secret }`
  (`Secret`: absent or `""` keeps, `null` clears, a string sets). Checks, with fixed messages that
  never carry the key:
  - URL: `https://` with a host and an optional port in 1..65535 (the app's `isGatewayUrl`).
    Empty string or `null` clears it.
  - Key: exactly 64 hex characters, stored lower-case.
  - Both or neither: a URL without a stored key (or a key without a URL) is saved but the code
    leaves the gateway out, and the read says so (`"gateway_ready": bool`), so the page can say
    "add the key to include remote access".
  - Saving **does not restart the hub** (nothing running reads it), like the `system` section.
- `GET /api/v1/setup/mobile-code` returns `{ "payload": "<compact v:1 JSON>" }` built at request
  time from the live config and the request's `Host` header. It is the only read that carries
  secrets, and it is never logged.
- MCP: `get_settings` shows the `mobile` section (key as `{"set": bool}`). `set_settings`
  takes it through the same no-restart path as the HTTP handler, the way it handles `system`.
  No MCP tool returns the payload.

## The web block (one component, used twice)

- Title "Connect your phone", a line: "Install the Nidoo app, open it, tap *Scan setup code* and
  point the camera here."
- The QR code, drawn in the browser as SVG from the payload (a pure-Rust QR crate compiled to
  wasm, e.g. `qrcode` with `default-features = false` plus its SVG renderer (no `image`
  crate), error correction M; *builder picks the crate*. There is no wasm size cap, but keep
  the crate small).
  Dark modules on a white quiet zone in both themes, so phones scan it in Night and Ivoire.
- **Hidden by default**: the code is blurred behind a **Show code** button, because it holds the
  broker password and the gateway key. Once shown, a **Hide** button blurs it again. *Mine.*
- A status line under it: "Home network only" or "Home and away (remote access)".
- **Remote access** (collapsed when empty, open when set): two inputs, **Gateway URL**
  (placeholder `https://house.example.ts.net`) and **Gateway key** (password field, placeholder
  "64 hex characters", shows "Saved" when one is stored), a **Save** button, inline errors from
  the API. After a save the code is fetched again.
- Done step: the block goes under the summary and above **Open my house**. It is optional;
  finishing never waits on it.
- Settings: a new page `SettingsPage::MobileApp`, slug `mobile-app`, in the same group as Push
  notifications (*mine*), phone icon, hint "Set up the Nidoo app on a phone".
- English and French text in the i18n tables, like every other screen.

## Docs

- `docs/users/`: a short "Connect the mobile app" page (where the code is, what remote access
  needs, that the code holds passwords).
- `docs/reference/configuration.md`: the `mobile` section.
- README feature list: one line.

## Tests (at least)

- Payload builder (hub, pure function): broker only; with gateway; gateway URL without key →
  no gateway; Frigate with/without login; pass without user dropped; push token topic from the
  prefix; the output parses as JSON with the exact field names the app reads.
- `PUT /settings/mobile`: http URL refused, bad port refused, 63/65 chars and non-hex keys
  refused, key stored lower-case, `""` keeps, `null` clears, the read never contains the key,
  no restart.
- Web: Done step and Settings page render the block; the code stays blurred until Show code;
  a save refetches; the local-host replacement rule.

## Left to the builder

Crate choice and SVG styling, exact copy wording, where the section lives in `sections.rs`
versus its own file, test placement, and the screenshot refresh for the docs.
