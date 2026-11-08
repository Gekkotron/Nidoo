# Phone setup QR code Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The hub draws the Android app's `v:1` setup QR code on the wizard's Done step and on a new Settings → Mobile app page. Two inputs set the remote gateway's URL and key.

**Architecture:** A pure payload builder on the hub (`crates/nidoo/src/setup/mobile.rs`) reads the MQTT config from `hub.json` and the `frigate`/`push`/`mobile` sections. A new `mobile` settings row is saved without a restart, the way `system` is. `GET /api/v1/setup/mobile-code` returns the compact JSON. The web page draws the QR code as SVG in wasm with the `qrcode` crate, in one `PhoneSetup` component used by both screens.

**Tech Stack:** Rust, axum, sqlx/Postgres settings table, Leptos (wasm), `qrcode` 0.14, wasm-bindgen-test.

**Spec:** `docs/superpowers/specs/2026-11-08-app-setup-qr-design.md`

## Global Constraints

- The payload must be accepted by Nidoo-app's `core/data/src/main/kotlin/com/gekkotron/nidoo/data/OnboardingPayload.kt`. Field names, exactly: `v`, `local.{host,port,tls,ws,user,pass}`, `gateway.{url,key,broker_host}`, `push.token_topic`, `frigate.{url,user,pass}`.
- `prefix` and `history` are never written.
- The JSON is compact, with no spaces.
- No secret (broker password, gateway key, Frigate password) may appear in a log line, an error message, a settings read, or a tracked file. Error messages are fixed text.
- Tracked text uses only example hosts (`house.example.ts.net`, `192.0.2.x`), never the owner's real ones.
- Gateway URL: `https` scheme, a non-empty host, port absent or in 1..65535. Gateway key: exactly 64 hex characters, stored lower-case.
- Saving `mobile` never restarts the hub. Every route follows the Run-mode rule: 409 `finish MQTT and database setup first` otherwise.
- Every UI string goes through i18n, in both EN and FR. The `bare_text` test enforces this.
- Commits are made as Gekkotron: `git -c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com commit …`. Stage explicit paths only. Run `git branch --show-current` before each commit; it must print `feat/app-setup-qr`.
- Builds use the shared target with `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0`. Check `df -h ~` before a full workspace build.

## Review Focus

1. **Broker host is `localhost` or a Docker name, and the request's `Host` is `192.0.2.5:8081`.** Both `local.host` and `gateway.broker_host` must be `192.0.2.5`, with no port. Test in Task 1: `phone_host_*`.
2. **Gateway URL saved without a key, or a key without a URL.** The code must leave `gateway` out, and the read must say `gateway_ready: false`. Test in Task 1: `payload_needs_both_url_and_key`. Test in Task 2: `mobile_read_reports_not_ready_with_url_only`.
3. **Frigate has a password but no user.** `frigate` must carry the `url` only. Test in Task 1: `frigate_pass_without_user_is_dropped`.
4. **A browser re-sends the read it got (`gateway_key: {"set": true}`), or an empty password field `""`.** The stored key must be kept, not cleared. Test in Task 2: `mobile_put_keeps_key_on_set_view_and_empty`.
5. **A save while the database is down.** The response must be a 503, and nothing changes in memory or in the cache. Test in Task 2: `mobile_save_without_database_is_unavailable`.

---

### Task 1: Payload builder and gateway checks (hub, pure)

**Files:**
- Create: `crates/nidoo-api/src/mobile.rs`
- Modify: `crates/nidoo-api/src/lib.rs`: add `mod mobile;` and re-export its types like the other modules do
- Create: `crates/nidoo/src/setup/mobile.rs` (unit tests in-file under `#[cfg(test)]`)
- Modify: `crates/nidoo/src/setup/mod.rs`: `pub mod mobile;`

**Interfaces:**
- Produces, in `nidoo_api`:
  - `pub struct MobileView { pub gateway_url: Option<String>, pub gateway_key: SecretView, pub gateway_ready: bool }`, deriving `Serialize, Deserialize, JsonSchema, Debug, Clone, PartialEq`
  - `pub struct MobileInput { #[serde(default)] pub gateway_url: Option<String>, #[serde(default)] pub gateway_key: Secret }`, which deserializes only
  - `pub struct MobileCode { pub payload: String }`
- Produces, in `nidoo::setup::mobile`:
  - `#[derive(Clone, Default, PartialEq, Serialize, Deserialize)] #[serde(default)] pub struct MobileSection { pub gateway_url: Option<String>, pub gateway_key: Option<String> }`. Its hand-written `Debug` masks the key as `***`, the same pattern as `Bundled`.
  - `pub fn view(s: &MobileSection) -> MobileView`
  - `pub fn apply_input(stored: &MobileSection, input: MobileInput) -> Result<MobileSection, &'static str>`. `Err` carries fixed copy: `"The gateway address must start with https:// and name a host"` or `"The gateway key must be 64 hex characters"`. An empty or whitespace URL means `None`. The key goes through `secret::apply`.
  - `pub fn phone_host<'a>(broker_host: &'a str, request_host: Option<&'a str>) -> &'a str`
  - `pub fn build_payload(mqtt: &MqttConfig, sections: &Sections, request_host: Option<&str>) -> String`

- [ ] **Step 1: Write the failing tests** in `crates/nidoo/src/setup/mobile.rs`:

```rust
#[test] fn phone_host_keeps_a_lan_address() { assert_eq!(phone_host("192.0.2.7", Some("192.0.2.5:8081")), "192.0.2.7"); assert_eq!(phone_host("broker.lan", None), "broker.lan"); }
#[test] fn phone_host_swaps_loopback_and_docker_names() {
    for h in ["localhost", "127.0.0.1", "127.1.2.3", "::1", "mosquitto"] { assert_eq!(phone_host(h, Some("192.0.2.5:8081")), "192.0.2.5"); }
    assert_eq!(phone_host("localhost", Some("[2001:db8::5]:8081")), "2001:db8::5");
    assert_eq!(phone_host("localhost", None), "localhost");
}
#[test] fn payload_broker_only() {
    // mqtt: host 192.0.2.7, port 1884, user "u", password "p"; sections default (push prefix "nidoo/push")
    let v: Value = serde_json::from_str(&build_payload(&mqtt, &Sections::default(), None)).unwrap();
    assert_eq!(v, json!({"v":1,"local":{"host":"192.0.2.7","port":1884,"tls":false,"ws":false,"user":"u","pass":"p"},
        "push":{"token_topic":"nidoo/push/tokens/{installId}"}}));
}
#[test] fn payload_is_compact() { assert!(!build_payload(&mqtt, &Sections::default(), None).contains(' ')); }
#[test] fn payload_without_login_leaves_user_and_pass_out() // mqtt user/password None → local has no "user"/"pass" keys
#[test] fn payload_with_gateway() // url "https://house.example.ts.net", key 64×"a" → gateway == {"url":…, "key":…, "broker_host":"192.0.2.7"}
#[test] fn payload_needs_both_url_and_key() // url only → no "gateway"; key only → no "gateway"
#[test] fn gateway_broker_host_follows_phone_host() // mqtt host "localhost", request Some("192.0.2.5:8081") → local.host and gateway.broker_host both "192.0.2.5"
#[test] fn frigate_with_login() // frigate url "http://192.0.2.8:8971", user "f", password "fp" → {"url":…, "user":"f", "pass":"fp"}
#[test] fn frigate_pass_without_user_is_dropped() // user None, password Some → frigate == {"url":…}
#[test] fn no_frigate_when_url_empty() // url None or Some("") → no "frigate"
#[test] fn push_topic_follows_prefix() // push.prefix "house/push" → "house/push/tokens/{installId}"
#[test] fn never_writes_prefix_or_history() // the keys "prefix" and "history" are absent at the root
#[test] fn apply_input_rules() {
    // "http://x" → Err; "https://" → Err; "https://h:0" → Err; "https://house.example.ts.net:8443" → Ok
    // key of 63 or 65 chars → Err; non-hex 64 chars → Err; 64 upper-case hex → stored lower-case
    // url Some("  ") → None; key Secret::Keep keeps the stored one; Secret::Clear → None
}
#[test] fn view_never_carries_the_key() // serde_json::to_string(&view(&s)) does not contain the key; gateway_ready true only when both are set
#[test] fn debug_masks_the_key() // format!("{s:?}") does not contain the key
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `CARGO_PROFILE_TEST_DEBUG=0 cargo test -p nidoo --lib setup::mobile`
Expected: compile errors, because the module items don't exist yet.

- [ ] **Step 3: Implement the types and functions listed above**

For `phone_host`, a broker host counts as unreachable when it is `localhost`, it parses as an `IpAddr` that `is_loopback()`, or it has no `.`, does not parse as an IP, and is not `::1`. To read the request host, strip a `:port` suffix; for `[v6]:port`, take what is inside the brackets. Use the request host only when it is `Some` and not empty.

For `build_payload`, build it with `serde_json::json!` and serialize with `serde_json::to_string`; that gives compact output. Field order doesn't matter.

For URL checks, use `url::Url::parse`: scheme `"https"`, `host_str()` non-empty, and `port()` not `Some(0)`.

- [ ] **Step 4: Run the tests and confirm they pass**

Run: `CARGO_PROFILE_TEST_DEBUG=0 cargo test -p nidoo --lib setup::mobile && cargo test -p nidoo-api`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/nidoo-api/src/mobile.rs crates/nidoo-api/src/lib.rs crates/nidoo/src/setup/mobile.rs crates/nidoo/src/setup/mod.rs
git commit -m "feat(setup): build the phone setup code from the hub's settings"
```

---

### Task 2: `mobile` row, no-restart save, routes and MCP

**Files:**
- Modify: `crates/nidoo/src/setup/sections.rs`: `SECTION_KEYS` gains `"mobile"` (length 8); `Sections` gains `pub mobile: MobileSection`; also update `to_rows` and `from_rows`, and the `row_keys_match_section_keys` test.
- Modify: `crates/nidoo/src/setup/store.rs`: add `save_live`.
- Modify: `crates/nidoo/src/system/mod.rs`: rewrite `set_system_notify` on top of `save_live`. Its existing tests must still pass unchanged.
- Modify: `crates/nidoo/src/setup/settings.rs`: `known()` also excludes `"mobile"`. `read_section` and `write_section` handle `"mobile"` before the `known` check.
- Modify: `crates/nidoo/src/setup/routes.rs`: add the route `GET /api/v1/setup/mobile-code`.
- Modify: `crates/nidoo/src/mcp/tools.rs`: the doc comments of `get_settings` and `set_settings` name `mobile` and say that saving it does not restart the hub.
- Modify: `docs/reference/configuration.md`: document the `mobile` section, the `mobile-code` route, and that the code holds secrets.
- Test: `crates/nidoo/tests/setup_http.rs` and `crates/nidoo/tests/settings_db.rs`

**Interfaces:**
- Consumes: everything Task 1 produces.
- Produces:
  - `pub async fn save_live(ctx: &SetupCtx, key: &'static str, change: impl FnOnce(&mut Sections)) -> Result<Sections, &'static str>`. It locks `ctx.sections`, applies `change` to a clone, and saves the row, then the cache. If the cache write fails, it restores the row. Only after both succeed does it swap in the new copy and return it. The `Err` values are `"no database"`, `"the database did not take the write"` and `"the settings cache could not be written"`.
  - HTTP `GET /api/v1/settings/mobile` returns a `MobileView`.
  - HTTP `PUT /api/v1/settings/mobile` takes a `MobileInput` and returns the new `MobileView` with status 200. It returns 422 with the fixed copy from `apply_input`, 409 outside Run mode, and 503 when there is no database or a save fails.
  - HTTP `GET /api/v1/setup/mobile-code` returns `MobileCode`. It returns 409 outside Run mode or when `hub.json` has no `mqtt`. It is built from `ctx.file.mqtt`, `ctx.sections` and the request's `Host` header (`axum::http::HeaderMap`, `header::HOST`).

- [ ] **Step 1: Write the failing tests**

In `setup_http.rs`, these use no database and reuse `serve`/`mqtt_file`:

```rust
#[tokio::test] async fn mobile_code_is_refused_in_setup_mode() // Mode::Setup → 409, body error "finish MQTT and database setup first"
#[tokio::test] async fn mobile_code_carries_the_broker() // Mode::Run, mqtt_file("192.0.2.7") → 200; parse payload; local.host == "192.0.2.7"; v == 1
#[tokio::test] async fn mobile_code_swaps_localhost_for_the_request_host() // mqtt_file("localhost"), request header Host: 192.0.2.5:8081 → local.host == "192.0.2.5"
#[tokio::test] async fn mobile_read_never_shows_the_key() // sections.mobile with key set → GET /api/v1/settings/mobile has gateway_key == {"set":true} and the body does not contain the key
#[tokio::test] async fn mobile_read_reports_not_ready_with_url_only() // url only → gateway_ready == false
#[tokio::test] async fn mobile_save_without_database_is_unavailable() // Mode::Run, db None → PUT → 503; a following GET returns the old values
```

In `settings_db.rs`, these skip without `TEST_DATABASE_URL`, like their neighbours:

```rust
#[tokio::test] async fn mobile_put_round_trips_without_restart() // PUT {"gateway_url":"https://house.example.ts.net","gateway_key":"<64 hex>"} → 200, gateway_ready true; restart counter still 0; load_sections() has the row
#[tokio::test] async fn mobile_put_keeps_key_on_set_view_and_empty() // then PUT with gateway_key {"set":true}, then with "" → key unchanged in load_sections()
#[tokio::test] async fn mobile_put_refuses_bad_input_without_echo() // "http://…" → 422; a 63-char key → 422; the response body never contains the sent key
#[tokio::test] async fn mobile_code_includes_the_gateway() // after the PUT, GET mobile-code → payload.gateway.url == "https://house.example.ts.net"
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `CARGO_PROFILE_TEST_DEBUG=0 cargo test -p nidoo --test setup_http mobile`
Expected: FAIL with 404s and missing types.

- [ ] **Step 3: Implement `save_live` and move `set_system_notify` onto it**

`set_system_notify` keeps its `HubError::Publish(format!("settings unavailable: {why}"))` mapping and its logging.

- [ ] **Step 4: Implement the `mobile` row, the `read_section`/`write_section` branch and the `mobile-code` handler**

`write_section(ctx, "mobile", body)` follows these steps:
1. Check Run mode, then that `ctx.restarting` is not set.
2. Parse the body as `MobileInput`; on failure, return 422 with `"The mobile settings are not valid"`.
3. Call `apply_input` against the stored section; on error, return 422 with its copy.
4. Call `save_live(ctx, "mobile", …)`; on error, return 503 with its text.
5. Return the `view` of the result as JSON.

It never schedules a restart. MCP inherits this branch, because it calls the same functions.

- [ ] **Step 5: Run the tests and confirm they pass**

Run: `CARGO_PROFILE_TEST_DEBUG=0 cargo test -p nidoo --test setup_http && cargo test -p nidoo --lib system:: setup::`, plus `--test settings_db` when `TEST_DATABASE_URL` is set.
Expected: PASS, and the existing `set_system_notify_*` tests are unchanged and green.

- [ ] **Step 6: Commit**

```bash
git add crates/nidoo/src/setup/{sections,store,settings,routes}.rs crates/nidoo/src/system/mod.rs crates/nidoo/src/mcp/tools.rs crates/nidoo/tests/setup_http.rs crates/nidoo/tests/settings_db.rs docs/reference/configuration.md
git commit -m "feat(setup): store the remote gateway and serve the phone setup code"
```

---

### Task 3: `PhoneSetup` block on the Done step (web)

**Files:**
- Modify: `crates/nidoo-web/Cargo.toml`: `qrcode = { version = "0.14", default-features = false, features = ["svg"] }`
- Create: `crates/nidoo-web/src/qr.rs`, with `pub fn qr_svg(text: &str) -> Option<String>`, and register it in `lib.rs`
- Modify: `crates/nidoo-web/src/api/setup.rs`: add `get_mobile`, `put_mobile` and `get_mobile_code`
- Create: `crates/nidoo-web/src/i18n/mobile.rs`, with `MobileText`, `mobile_text(lang, MobileText) -> &'static str` and `use_mobile_text()` following the pattern of `i18n/system.rs`; register it in `i18n/mod.rs`
- Create: `crates/nidoo-web/src/screens/mobile_app.rs`, with the `PhoneSetup` component; register it in `screens/mod.rs`
- Modify: `crates/nidoo-web/src/setup/done.rs`: put `<PhoneSetup />` between `</ul>` and the `loc__problem` line
- Modify: the web stylesheet that holds `.done__*`, adding `.phone__*` classes
- Test: `crates/nidoo-web/tests/setup_done.rs`, plus a `#[cfg(test)]` test for `qr_svg`

**Interfaces:**
- Produces:
  - `pub async fn get_mobile() -> Result<MobileView, ApiError>`
  - `pub async fn put_mobile(input: &serde_json::Value) -> Result<MobileView, SaveError>`. It returns the reply body instead of dropping it the way `put_section` does.
  - `pub async fn get_mobile_code() -> Result<MobileCode, ApiError>`
  - `#[component] pub fn PhoneSetup() -> impl IntoView`. It reads `Lang` from context and fetches its own data on mount.
- Copy, EN / FR, verbatim:
  - Title: "Connect your phone" / « Connectez votre téléphone »
  - Lead: "Install the Nidoo app, open it, tap Scan setup code and point the camera here." / « Installez l'application Nidoo, ouvrez-la, touchez Scanner le code et visez ce code. »
  - Buttons: "Show code" / « Afficher le code », "Hide" / « Masquer »
  - Warning: "This code holds your passwords. Show it only to people you trust." / « Ce code contient vos mots de passe. Ne le montrez qu'à des personnes de confiance. »
  - Status: "Home network only" / « Réseau de la maison uniquement », "At home and away" / « À la maison et à distance »
  - Section: "Remote access" / « Accès à distance »
  - Fields: "Gateway address" / « Adresse de la passerelle », "Gateway key" / « Clé de la passerelle »
  - Key placeholder: "64 hex characters" / « 64 caractères hexadécimaux »
  - Stored: "Saved" / « Enregistrée »
  - Save button: "Save" / « Enregistrer »
  - Hint: "Add the key to include remote access." / « Ajoutez la clé pour inclure l'accès à distance. »

- [ ] **Step 1: Write the failing tests**

```rust
// qr.rs unit test
#[test] fn qr_svg_draws_a_code() { let s = qr_svg(r#"{"v":1}"#).unwrap(); assert!(s.starts_with("<svg") || s.starts_with("<?xml")); assert!(s.contains("#000000") || s.contains("black")); }
```

In `tests/setup_done.rs`, stub `GET /api/v1/settings/mobile` and `GET /api/v1/setup/mobile-code` with the `FetchStub`:

```rust
#[wasm_bindgen_test] async fn done_shows_the_phone_block_blurred() // .phone__code has class phone__code--hidden; the "Show code" button exists; the text contains "Connect your phone"
#[wasm_bindgen_test] async fn show_code_reveals_and_hide_blurs() // click Show → no --hidden class and an <svg> inside; click Hide → --hidden again
#[wasm_bindgen_test] async fn home_only_status_without_gateway() // gateway_ready false → "Home network only"
#[wasm_bindgen_test] async fn saving_the_gateway_refetches_the_code() // fill both inputs, click Save → stub.wait_for("PUT") body has gateway_url + gateway_key; then a second GET of mobile-code
#[wasm_bindgen_test] async fn a_refusal_shows_inline() // PUT answers 422 {"error":"The gateway key must be 64 hex characters"} → that text shows in role=alert
#[wasm_bindgen_test] async fn open_my_house_still_works_without_the_code() // mobile-code answers 503 → the block shows its error, and the Open my house button is enabled
```

- [ ] **Step 2: Run the tests and confirm they fail**

Run: `cargo test -p nidoo-web --lib qr` and `cd crates/nidoo-web && wasm-pack test --headless --chrome -- --test setup_done`. If chromedriver doesn't match, use the interactive runner from memory `reference_wasm_tests_interactive.md`.
Expected: FAIL.

- [ ] **Step 3: Implement `qr_svg`, the API functions, the i18n module and `PhoneSetup`**

- Use `qrcode::QrCode::with_error_correction_level(text, EcLevel::M)` and `render::<svg::Color>()` with dark `#000000`, light `#ffffff` and a quiet zone. Insert the SVG with `inner_html`. The white background stays white in both themes.
- The blur is the CSS class `phone__code--hidden` with `filter: blur(12px)`.
- Remote access is a `<details>` that is `open` when `gateway_url` is set.
- The key input is `type="password"` and shows `MobileText::Saved` as its placeholder when `gateway_key.set`.
- On Save, send `{"gateway_url": url_or_null, "gateway_key": key_text}`. An empty key field sends `""`, which keeps the stored key. Then fetch the view and the code again, and clear the key field.

- [ ] **Step 4: Run the tests and confirm they pass**

Run: the same commands as Step 2, then `cargo test -p nidoo-web --test bare_text`.
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/nidoo-web/Cargo.toml Cargo.lock crates/nidoo-web/src/qr.rs crates/nidoo-web/src/lib.rs crates/nidoo-web/src/api/setup.rs crates/nidoo-web/src/i18n/mobile.rs crates/nidoo-web/src/i18n/mod.rs crates/nidoo-web/src/screens/mobile_app.rs crates/nidoo-web/src/screens/mod.rs crates/nidoo-web/src/setup/done.rs crates/nidoo-web/tests/setup_done.rs <stylesheet>
git commit -m "feat(setup): the Done step shows the code that sets up the phone app"
```

---

### Task 4: Settings → Mobile app page, user docs

**Files:**
- Modify: `crates/nidoo-web/src/screens/settings.rs`: add `SettingsPage::MobileApp`. Grep `SettingsPage::Push` and add the new arm next to each match:
  - `ALL`: grows to 19 entries, with `MobileApp` after `Push`
  - slug: `"mobile-app"`
  - title key
  - hint `T::HintMobileApp`
  - icon: a new `icons::PHONE`, colour `"sky"` (or the nearest existing tint name)
  - group: `T::GroupHouse`
  - status: `"Home network only"` / `"At home and away"` from a `get_mobile()` fetched once, like `system`
  - page dispatch: `<MobileAppPage />`
- Modify: `crates/nidoo-web/src/screens/mobile_app.rs`: add `#[component] pub fn MobileAppPage()`, which is the page header plus `<PhoneSetup />`
- Modify: `crates/nidoo-web/src/i18n/settings.rs`: page title "Mobile app" / « Application mobile », and hint "Set up the Nidoo app on a phone" / « Configurer l'application Nidoo sur un téléphone »
- Modify: `crates/nidoo-web/src/icons.rs`: `PHONE`, an `svg!` of a rounded rect (`x=6 y=2 w=12 h=20 rx=2`) plus a short line at the bottom (`M11 18h2`)
- Create: `docs/users/mobile-app.md`. Use the same front matter as `docs/users/watchdogs.md`. Cover where the code is, scanning it, what remote access needs (the gateway's https address and its 64-character key), and that the code holds passwords
- Modify: `README.md`: add one feature line
- Test: `crates/nidoo-web/tests/settings_updates.rs`, or the file that already tests settings navigation

**Interfaces:**
- Consumes: `PhoneSetup`, `get_mobile`, `MobileText` (from Task 3).

- [ ] **Step 1: Write the failing tests**

```rust
#[wasm_bindgen_test] async fn settings_lists_mobile_app_in_the_house_group() // the list has a row with href "/settings/mobile-app" and text "Mobile app"
#[wasm_bindgen_test] async fn mobile_app_page_shows_the_phone_block() // navigate to /settings/mobile-app → "Connect your phone" and the Show code button
```

Also update any existing test that counts `SettingsPage::ALL` (18 → 19).

- [ ] **Step 2: Run the tests and confirm they fail.** Use the wasm-pack command from Task 3 with `--test settings_updates`.

- [ ] **Step 3: Implement the page, its wiring, the icon, the i18n strings and the docs**

- [ ] **Step 4: Run the full checks**

Run, after checking `df -h ~` (at least 5 GB free):
`cargo fmt --all --check && CARGO_PROFILE_DEV_DEBUG=0 cargo clippy --workspace --all-targets -- -D warnings && CARGO_PROFILE_TEST_DEBUG=0 cargo test --workspace && cargo clippy -p nidoo --no-default-features --all-targets -- -D warnings`, then the wasm tests for `setup_done` and `settings_updates`.
Expected: everything green.

- [ ] **Step 5: Commit**

```bash
git add crates/nidoo-web/src/screens/settings.rs crates/nidoo-web/src/screens/mobile_app.rs crates/nidoo-web/src/i18n/settings.rs crates/nidoo-web/src/icons.rs crates/nidoo-web/tests/settings_updates.rs docs/users/mobile-app.md README.md
git commit -m "feat(settings): a Mobile app page with the phone setup code"
```
