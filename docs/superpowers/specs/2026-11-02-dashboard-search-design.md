# Dashboard search — design

Date: 2026-11-02 · Status: draft for approval · TODOS item: "Dashboard-wide search bar"

## Why

Today the only search in the web UI is on the Zigbee2MQTT diagnostics screen
(`screens/devices.rs`). To reach a device that is not on the current dashboard the user has to
know its room, open the room and scroll. The house has more than a hundred entities (Garage alone
holds 52), so "where is that plug" is a daily friction.

## What it is

A magnifying glass in the top bar opens a **Search page**. The user types a few words; the
matching devices appear **as the same live cards a dashboard shows** (light slider, thermostat
dial, vacuum, shutter, sensor sparkline…), grouped under their room. A card behaves exactly as it
does on a dashboard: it can be switched or dragged in place, and tapping it opens its sheet.
Matching rooms appear as small tiles that open the room.

The user picked this layout (option C) over a pop-up over the dashboard and over a drop-down under
the field (mockups in `.superpowers/brainstorm/`, screen `search-entry.html`).

## Goals

- Find any shown device by name, room, device, or kind, in French or English, from anywhere in
  the web UI.
- Control the result in place with the real card — no second, poorer control surface.
- Results follow the keystrokes with no visible lag.

## Non-goals

- Flows, moods (ambiances), groups' definitions, settings pages, alerts: not searched.
- Typo tolerance / fuzzy ranking: words must be found as typed (accents and case aside).
- Remembering searches between visits, search history, suggestions.
- A hub endpoint: the browser already holds the whole house.
- The Android app.

## Anti-goals (failure even if it "works")

- Cards on the Search page that look or act differently from the dashboard's.
- Typing that stutters because every keystroke rebuilds dozens of cards.
- A result list that loses its place when the user opens a card's sheet and comes back.

## Decided with the user

- **Entry point**: a magnifying glass in the top bar (user: "a little loupe in the dashboard").
- **Results page**: option C, its own page made of real cards.
- **Matching**: the word matching of the MCP `search` tool (PR #141): lowercase, accents folded,
  French/English stop words dropped, every remaining word must be found in the entity's name,
  id, room (id or name), device (name, model, vendor), kind, device class, or the French/English
  words for those.
- **Rooms** in the results as small tiles that open the room.
- **Empty field** shows the Favorites.
- **Esc / back** returns to the page the user came from; the search is not remembered between
  visits.
- Flows, moods and settings are not searched.

## Design

### 1. Shared matcher

Move the matcher out of `crates/nidoo/src/mcp/search.rs` into `nidoo-api` (new module
`nidoo_api::search`), which both the hub and the web crate already depend on and which already
depends on `nidoo-core` (`Kind`, `SwitchType`). Public surface:

- `Query::parse(&str)`, `Query::is_empty()` (no words left after stop words).
- `Query::device_matches(&DeviceView)`.
- `EntityMatcher::new(&Query, &[AreaView], &[DeviceView])` and `matches(&EntityView)`.
- New: `Query::area_matches(&AreaView)` — every word found in the folded area id or name.
  Room names are user-typed (not translated), so id + name is enough; the only localised area
  name is the virtual Unassigned, which never gets a room tile.

The MCP tool keeps its behaviour and imports from the new place; its tests move with the code.
No behaviour change for the MCP `search` tool.

### 2. The magnifying glass

`shell/header.rs` → `ContentHeader` tools gain a first button, before the theme button: the
`MAGNIFIER` icon already used by `screens/devices.rs`, `aria-label` "Search" / "Rechercher",
navigating to `/search`. On the Search page itself the button is shown as current
(`aria-current="page"`).

Keyboard: pressing `/` anywhere in the UI — except while focus is in an `input`, `textarea`,
`select` or `contenteditable` element, or with a modifier key held — opens `/search`. (A `keydown`
listener installed once in the shell.)

### 3. The Search page

Route `/search` (`app.rs`), screen `screens/search.rs`, crumb "Search" / "Recherche"
(`shell/route.rs`).

- **Field**: a search input at the top of the page, focused on arrival, same look as the
  devices-screen field (`devices__search`), with a clear (×) button once it has text.
- **Query in the URL**: the field mirrors to `?q=…` with a history *replace* (not push), so
  opening a card's sheet and coming back restores the same results, and typing does not fill the
  back stack. Arriving at `/search` without `q` starts empty.
- **Esc** in the field: first press clears a non-empty field; on an empty field it goes back
  (`history.back()` when the previous entry is in the app, otherwise to `/`).

### 4. What is searched

An entity is a candidate when it would render a card: `is_primary()` (which already excludes
`ui.hidden` and folded entities), and for a generic entity `generic_placement == Card` — i.e. what
`card_for` draws. Unlike Home, entities moved to Energy or Security are **included**: search is
for finding *any* device. Diagnostic, config and disabled-by-default entities are not primary and
stay out.

**Flexible groups** follow Home: the `group_<id>` entity is a candidate, and the members of a
group with `hideMembers` set are not (the same rule `groups::home_items` applies). So the results
function takes `house.groups` too. A hidden member the user made a favorite still shows in the
empty-field Favorites, exactly as Home draws it.

A word that matches a room name matches every device in that room (the matcher already searches
the room), so "salon" shows the whole Salon plus its room tile. That is intended.

The candidate list then goes through `without_paired_companions` so that a humidity sensor folded
into its temperature card is not drawn twice — but only when both matched; a search for
"humidité" still shows the humidity sensor on its own.

### 5. Results layout

Top to bottom:

1. A count line: "4 devices · 1 room" / "4 appareils · 1 pièce".
2. **Devices**, grouped by room, each group headed by the room name. Rooms in the sidebar order,
   Unassigned last; inside a room, the same sort Home uses (`ui.order`, then index — as in
   `home_items`). Cards drawn by `card_for`.
3. **Rooms**: one small tile per matching room — name and the room's sidebar count
   (`home_counts`, as the sidebar shows it) — that opens `/rooms/<id>`. The virtual Unassigned
   area never gets a tile.

**Card grid.** Search sections use the same section markup and grid classes as a room
(`section.room`, `grid grid--home`) and draw each card with `card_for`, so the cards are the
dashboard's own (anti-goal). `RoomSection` itself stays as it is — its drag-reorder wrappers and
edit flow are not needed — and the empty-field Favorites reuse `RoomSection(FAVORITES)`, which is
already read-only.

**Back to the same place.** Search sections carry the same `room-<id>` anchors as Home and use
`use_back_anchor`, so returning from a card's sheet restores both the query (`?q=`) and the
scroll position at that room.

- **Empty field**: the Favorites section as Home draws it, **read-only** (Home's Favorites have no
  edit either), with a hint line above ("Type a name, a room or a kind of device"). No favorites →
  only the hint.
- **No match**: "Nothing matches “…”" / "Rien ne correspond à « … »".
- **Cap**: at most 60 cards are drawn, taken from the flattened list in the order above (so the
  last rooms can lose all their cards); a line then says "Showing 60 of 143 — type more to narrow
  it down". The count line at the top counts the uncapped total. (A one-letter query should not
  draw the whole house.)

### 6. Speed

Matching runs in the browser on the `House` signals (`entities`, `areas`, `devices`), so it is
synchronous and needs no network. The result list is a `Memo` over the query and the house, so
live state echoes refresh the cards without re-running the search layout. The field updates the
query after a short debounce (≈150 ms) so a fast typist does not rebuild the cards on every
letter.

### 7. Text

New keys in `crates/nidoo-web/src/i18n` (a new `search.rs` or an extension of `home.rs`), French
and English: Search, Rechercher, the placeholder, the hint, the count line (singular/plural), the
no-match line, the cap line, the clear button label.

### 8. Tests

- `nidoo-api`: the moved matcher tests, plus `area_matches` (name, id, accents, every word).
- `nidoo-web` (native unit test of a pure function, e.g. `search_results(query, entities, areas,
  devices) -> SearchResults`): candidate filter (hidden, non-primary, generic non-card out;
  energy/security in), room grouping and order, companion folding, cap.
- `wasm-pack` browser test: the glass navigates to `/search`; typing filters; `?q=` restores.

### 9. Docs

`docs/users/web-ui.md`: a short "Search" section (the glass, `/`, what it finds). No new
screenshot required.

## Left to the builder

- Exact module/file split inside `nidoo-web` and the name of the pure results function.
- The CSS for the count line, room headings and room tiles, reusing existing tokens (Night and
  Ivoire both).
- Whether the debounce lives in the field or in a derived signal.
- How `/` is ignored inside existing text fields beyond the list above (e.g. the flow canvas
  editor), as long as typing a `/` in any field never leaves the page.

## Calls made without the user (check these)

- The glass sits in the **shared top bar, so it shows on every page**, not only dashboards.
- **`/` keyboard shortcut** to open the search.
- The query lives in the **URL (`?q=`)**, so back from a card's sheet restores the results.
- **Energy and Security devices are included** (Home leaves them out).
- **60-card cap** with a "type more" line.
- Rooms listed **after** the devices.
- **Grouped lamps follow Home**: a group that hides its members shows only the group card.
- Search sections **reuse the room's grid classes and `card_for`** rather than extracting a
  component from `RoomSection`; search sections are not editable.
- The matcher moves to **`nidoo-api`**; no new hub endpoint.
