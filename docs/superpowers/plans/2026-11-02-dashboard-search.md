# Dashboard Search Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A magnifying glass in the web UI's top bar opens a `/search` page that finds any shown device by words and draws the matches as the dashboard's own live cards, grouped by room, plus matching room tiles.

**Architecture:** The MCP word matcher moves from the hub crate into `nidoo-api` so the browser can use it. A pure function in `nidoo-web` turns (query, house signals) into ordered room sections, room tiles and counts; a new screen renders it with `card_for` in the room grid, and the shared `ContentHeader` gains the glass and a `/` shortcut.

**Tech Stack:** Rust, Leptos (CSR/WASM), leptos_router, wasm-bindgen-test (`wasm-pack test --headless --chrome`).

**Spec:** `docs/superpowers/specs/2026-11-02-dashboard-search-design.md`

## Global Constraints

- All user-visible text in French and English through `crate::i18n` (the `bare_text` test rejects literals in views).
- Matching = the MCP `search` rules unchanged: lowercase, accents folded, stop words dropped, every word must be found.
- Card cap: **60** cards drawn; the count line counts the uncapped total.
- Debounce ≈ **150 ms** between keystrokes and the search.
- Query lives in the URL as `?q=…`, written with **replace**, never push.
- No new hub endpoint; no Android changes; flows, moods and settings are not searched.
- Keep tracked text generic: no real room, host or family names in code, tests or docs (use "Kitchen", "Lamp" etc.).
- Long cargo runs: `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0` per invocation.
- Commits as `-c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com`, message ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. A query of stop words only ("le", "the") — should look like an empty field (Favorites + hint), not draw the whole house. Test in Task 2.
2. Typing `/` inside any text field (the search field itself, a flow name, a rename box) — must type the character, never navigate. Test in Task 4.
3. An entity whose `area` is not in `house.areas` (room just deleted) — still listed, in a last group with the Unassigned title, never dropped or panicking. Test in Task 2.
4. Accented query in the URL (`?q=lumi%C3%A8re`) — arriving or coming Back restores "lumière" in the field and the same results. Test in Task 3.
5. A hidden-member lamp of a `hideMembers` group that is also a favorite — absent from results, present in the empty-field Favorites (as Home). Test in Task 2.

---

### Task 1: Shared word matcher in `nidoo-api`

**Files:**
- Create: `crates/nidoo-api/src/search.rs` (moved from `crates/nidoo/src/mcp/search.rs`, tests included)
- Modify: `crates/nidoo-api/src/lib.rs` (`pub mod search;`)
- Delete: `crates/nidoo/src/mcp/search.rs`; Modify: `crates/nidoo/src/mcp/mod.rs` (drop `mod search;`), `crates/nidoo/src/mcp/tools.rs:30` (import from `nidoo_api::search`)

**Interfaces:**
- Produces (all `pub`): `nidoo_api::search::Query { parse(&str) -> Query, is_empty(&self) -> bool, device_matches(&self, &DeviceView) -> bool, area_matches(&self, &AreaView) -> bool }`, `EntityMatcher<'a> { new(&'a Query, &'a [AreaView], &'a [DeviceView]) -> Self, matches(&self, &EntityView) -> bool }`.

- [ ] **Step 1: Move the file with `git mv`, make the items `pub`, fix the MCP import; add failing tests**

```rust
#[test]
fn area_matches_name_or_id_every_word() {
    let a = area("kitchen_1", "Cuisine d'été");
    assert!(Query::parse("cuisine ete").area_matches(&a));
    assert!(Query::parse("KITCHEN_1").area_matches(&a));
    assert!(!Query::parse("cuisine garage").area_matches(&a));
}
#[test]
fn is_empty_after_stop_words() {
    assert!(Query::parse("  le de the ").is_empty());
    assert!(!Query::parse("le salon").is_empty());
}
```

- [ ] **Step 2: Run, expect FAIL (no `area_matches` / `is_empty`)** — `cargo test -p nidoo-api search`
- [ ] **Step 3: Implement `area_matches` (fold `id` + `\n` + `name`, then `all_in`) and `is_empty` (`words.is_empty()`)**
- [ ] **Step 4: Run** `cargo test -p nidoo-api search && cargo test -p nidoo mcp` — PASS, MCP tests unchanged; `cargo check -p nidoo-web --target wasm32-unknown-unknown` — OK.
- [ ] **Step 5: Commit** `refactor(api): the MCP word matcher moves to nidoo-api for the web`

### Task 2: Search results (pure function)

**Files:**
- Create: `crates/nidoo-web/src/search.rs` (+ `pub mod search;` in `src/lib.rs`), native `#[cfg(test)] mod tests` inside.
- Modify: `crates/nidoo-web/src/groups.rs` — expose `pub(crate) fn hidden_member_ids(groups: &BTreeMap<String, GroupView>) -> HashSet<&str>` (extracted from `home_items`, which then calls it).

**Interfaces:**
- Consumes: Task 1 `Query`, `EntityMatcher`; `EntityView::is_primary`, `screens::home::without_paired_companions`, `rooms::home_counts`, `rooms::generic_placement`.
- Produces:
```rust
pub const SEARCH_CAP: usize = 60;
pub struct SearchSection { pub area: String, pub ids: Vec<String> }   // area id; UNASSIGNED_AREA for unknown areas
pub struct RoomHit { pub id: String, pub name: String, pub count: usize } // count = home_counts value
pub struct SearchResults {
    pub sections: Vec<SearchSection>, // capped, in display order
    pub rooms: Vec<RoomHit>,          // never UNASSIGNED_AREA
    pub total: usize,                 // uncapped number of cards
    pub shown: usize,                 // min(total, SEARCH_CAP)
}
/// None when the query has no words (caller shows Favorites + hint).
pub fn search_results<'a>(query: &str, entities: impl Iterator<Item = &'a EntityView>,
    areas: &[AreaView], devices: &[DeviceView], groups: &BTreeMap<String, GroupView>) -> Option<SearchResults>;
```

Rules (spec §4–5): candidate = `is_primary()` and (non-generic or `generic_placement == Card`) — energy/security entities **included**; drop `hidden_member_ids`; keep `EntityMatcher::matches`; then `without_paired_companions` on the matched set; group by area in `house.areas` order (the hub's order, as `rooms::chips` uses — not `sidebar_rooms`, which drops rooms with no Home card) with `UNASSIGNED_AREA` and unknown areas last; inside a group sort by `(ui.order.unwrap_or(i32::MAX), name.to_lowercase())` as `groups::shown` does; cap the flattened list at 60.

- [ ] **Step 1: Write failing tests** (fixtures like `tests/room_back.rs`'s `light()` / `area()`):
  - `stop_words_only_is_none` — `search_results("le", …)` is `None`.
  - `matches_name_room_and_french_kind` — "kitchen lampe" finds the Kitchen light, not the Garage one.
  - `energy_and_security_entities_are_included` — an entity with `ui.energy_role` set and a door sensor are in results.
  - `hidden_and_non_primary_are_out` — `ui.hidden`, diagnostic category, generic non-card are absent.
  - `hide_members_group_shows_group_only` — group with `hide_members` and member "Lamp A" (also `ui.favorite`): query "lamp" lists `group_x`, not the member.
  - `paired_humidity_folded_only_when_both_match` — "kitchen" → temperature only; "humidite" → humidity sensor present.
  - `sections_in_sidebar_order_unknown_area_last` — areas [B, A], entity in removed area "gone" → section order B, A, then `UNASSIGNED_AREA` holding the "gone" entity.
  - `cap_cuts_flattened_list` — 70 matches over two rooms → `total == 70`, `shown == 60`, sum of section ids == 60, last room shortened first.
  - `room_hits_with_sidebar_count_never_unassigned` — "kitchen" → `RoomHit { id: "kitchen", count: home_counts value }`; a query matching "unassigned" yields no `RoomHit` for it.
- [ ] **Step 2: Run, expect FAIL** — `cargo test -p nidoo-web --lib search`
- [ ] **Step 3: Implement `search_results` and `hidden_member_ids`**
- [ ] **Step 4: Run** `cargo test -p nidoo-web --lib` — PASS (existing `groups`/`rooms` tests too).
- [ ] **Step 5: Commit** `feat(web): search results — matches grouped by room, room hits, 60-card cap`

### Task 3: The Search page

**Files:**
- Create: `crates/nidoo-web/src/screens/search.rs` (+ `pub mod search;` in `screens/mod.rs`); `crates/nidoo-web/src/i18n/search.rs` (+ `mod search; pub use search::*;` in `i18n/mod.rs`); `crates/nidoo-web/tests/search_page.rs`
- Modify: `crates/nidoo-web/src/app.rs:~212` (`<Route path=path!("/search") view=SearchScreen />`); `crates/nidoo-web/src/shell/route.rs` (crumb `["search", ..]` → `Key::Search`; `section_for` → `Section::Overview`); `crates/nidoo-web/src/i18n/mod.rs` (`Key::Search` "Search"/"Recherche"); CSS file holding `.devices__search` (new `.search__*` rules reusing tokens, Night + Ivoire).

**Interfaces:**
- Consumes: Task 2 `search_results`, `SearchResults`, `SEARCH_CAP`; `RoomSection(id: FAVORITES, anchored: true)` for the empty state; `card_for`; `use_back_anchor`; `provide_card_place(CardPlace::Room)`; `icons::MAGNIFIER`.
- Produces: `pub fn SearchScreen() -> impl IntoView`; `SearchText` enum + `search_text(Lang, SearchText) -> &'static str`; `pub fn search_count(lang, devices: usize, rooms: usize) -> String`.

Copy (EN / FR), exact:
- Placeholder: "Search a device, a room…" / "Chercher un appareil, une pièce…"
- Hint: "Type a name, a room or a kind of device" / "Tapez un nom, une pièce ou un type d'appareil"
- Count: "{n} device(s) · {m} room(s)" / "{n} appareil(s) · {m} pièce(s)" — singular at 1, the room part omitted at 0.
- No match: "Nothing matches “{q}”" / "Rien ne correspond à « {q} »"
- Cap: "Showing {shown} of {total} — type more to narrow it down" / "{shown} sur {total} affichés — précisez la recherche"
- Clear button label: "Clear" / "Effacer"; Rooms heading: "Rooms" / "Pièces"

Behaviour (spec §3, §5): field autofocused, value initialised from `?q=`; input → 150 ms debounce → `navigate("/search?q=<encoded>", replace: true)` (empty → `/search`); results `Memo` from the URL query over `house.entities/areas/devices/groups`; each `SearchSection` renders a `<section class="room" id="room-<encoded area>">` with an `h2` room title (`area_name`) and `div.grid.grid--home` of `card_for` per id (one `Memo` per card, as `RoomSection` does), click → `use_back_anchor` mark; room tiles link to `/rooms/<id>`; Esc in the field: non-empty → clear, empty → Back target (the `BackLink` stack, else `/`).

- [ ] **Step 1: Write failing browser tests** in `tests/search_page.rs` (harness like `tests/room_back.rs`: `House` with two areas and lights, `Router` with `/search`, `/entity/:id`, `TrackBackStack`):
  - `empty_shows_hint_and_favorites` — at `/search`, hint text present, the favorite's card present.
  - `typing_filters_after_debounce` — type "kitchen" → after 200 ms only the Kitchen card; URL is `/search?q=kitchen`; history length unchanged (replace).
  - `url_query_restores_accented_field` — open `/search?q=lumi%C3%A8re` → input value "lumière", the light named "Lumière" shown.
  - `back_from_sheet_restores_results` — search "kitchen", click the card's sheet link, click Back → `/search?q=kitchen#room-kitchen`, card present.
  - `no_match_and_cap_lines` — "zzz" → no-match line; 70 lights + "lamp" → cap line "Showing 60 of 70".
  - `escape_clears_then_leaves` — Esc with text → empty field; Esc again → previous page.
- [ ] **Step 2: Run, expect FAIL** — `cd crates/nidoo-web && wasm-pack test --headless --chrome -- --test search_page`
- [ ] **Step 3: Implement the i18n keys, route, crumb, `SearchScreen`, CSS**
- [ ] **Step 4: Run** the browser test above, `cargo test -p nidoo-web` (incl. `bare_text`, `route` crumb test for `/search` → ["My home", "Search"]) — PASS.
- [ ] **Step 5: Commit** `feat(web): the Search page — live cards by room, room tiles, query in the URL`

### Task 4: The glass in the top bar and the `/` shortcut

**Files:**
- Modify: `crates/nidoo-web/src/shell/header.rs` (button first in `content-header__tools`); `crates/nidoo-web/src/shell/mod.rs` (one window `keydown` listener); header CSS (`.content-header__search`, same size as `.content-header__theme`).
- Test: `crates/nidoo-web/tests/shell.rs` (extend).

**Interfaces:**
- Consumes: `Key::Search`, `icons::MAGNIFIER`, `/search` route from Task 3.
- Produces: `pub(crate) fn opens_search(key: &str, target_is_text: bool, modifier: bool) -> bool` (pure, in `shell/mod.rs`).

The glass is an `<a href="/search" class="content-header__search">` with `aria-label` = `Key::Search`, `aria-current="page"` when the path is `/search`. The listener calls `opens_search(ev.key(), <target is input/textarea/select/contenteditable>, ctrl||meta||alt)` and, when true, `prevent_default()` + navigate to `/search`.

- [ ] **Step 1: Write failing tests**
  - native `opens_search_only_outside_text_fields`: `("/", false, false)` → true; `("/", true, false)` → false; `("/", false, true)` → false; `("a", false, false)` → false.
  - browser `glass_opens_search_page` — click the glass → path `/search`, input focused.
  - browser `slash_in_a_text_field_types` — focus a rename `<input>` on a room page, dispatch `/` keydown → path unchanged.
  - browser `slash_on_page_opens_search` — keydown `/` on `document.body` → path `/search`.
- [ ] **Step 2: Run, expect FAIL** — `cargo test -p nidoo-web --lib opens_search` and `wasm-pack test --headless --chrome -- --test shell`
- [ ] **Step 3: Implement** the button, `opens_search`, the listener (removed `on_cleanup`).
- [ ] **Step 4: Run** both commands — PASS; then `cargo clippy -p nidoo-web --target wasm32-unknown-unknown -- -D warnings` and `cargo fmt --check` — clean.
- [ ] **Step 5: Commit** `feat(web): magnifying glass in the top bar, / opens the search`

### Task 5: Docs and backlog

**Files:**
- Modify: `docs/users/web-ui.md` (short "Search" section: the glass, `/`, what it finds and what it does not — flows, moods, settings); `TODOS.md` (tick "Dashboard-wide search bar" and "System health screen + uptime history").

- [ ] **Step 1: Write the section and tick both items** (generic wording, EN).
- [ ] **Step 2: Verify** `cd website && npm run build` succeeds (broken links fail the build).
- [ ] **Step 3: Commit** `docs: the search page in the web UI guide; backlog ticks`
