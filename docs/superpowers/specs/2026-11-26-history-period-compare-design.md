# History: pick a period and compare it with the same period of another year — design

Date: 2026-11-26

## Why

Every history chart shows a fixed range that ends now (1h, 6h, 24h, 7d, 30d, 1y). The hub's
history now goes back years: room thermometers since late 2020, the grid/solar clamp since
March 2025, a Linky index since January 2022 (imported 2026-11-26; native recording started
2026-01-15). None of it can be reached from the app.

The user wants to look at any period and, above all, **compare a period with the same period of
an earlier year** ("this November against last November", "this winter against the winter
2022–23"), to see whether the house uses less energy and stays as warm as before.

## Goals

- On the history page (`/history/:id`), pick a **calendar period**: Day, Week (Monday–Sunday),
  Month, Year, or a **custom** from–to range of dates. 1h and 6h stay as "recent" views ending
  now.
- Step back and forth one period at a time (‹ ›), back to the entity's first reading; "Back to
  now" returns to the current period.
- **Compare with the same period of another year**: one toggle, "Compare with [year ▾]",
  defaulting to the year before; any year with data can be picked. The compared period is
  drawn behind the shown one on the same axis (day 1 on day 1).
- Under the chart, a summary: the shown period, the compared one, and the difference. Counters
  give totals (kWh, and the difference in kWh and %); sensors give mean, min and max (and the
  difference of the means). While the shown period is still running, the compared period
  counts **the same days only** (Nov 1–26 against Nov 1–26).
- Clicking a bar or a point zooms in: a month of a Year view opens that month, a day of a
  Week/Month/custom view opens that day.
- Long windows stay cheap: multi-year charts read the hourly rollups (`history_hourly`), never
  millions of raw rows.
- The hub API accepts any window, so the MCP `get_history` tool can read any period too
  (daily kWh for a whole winter, a series for a given month).

## Non-goals

- Comparing with the **previous** period (October against November). The user dropped it:
  comparison is only ever against the same dates of another year.
- More than two periods on one chart (several winters stacked).
- Periods or comparison on dashboard widgets, the system chart, the Energy tab's day card or
  the entity cards' sparklines. They keep their fixed ranges and behaviour.
- A comparison endpoint on the hub (aligned pair + totals in one call). The app computes the
  comparison; a hub-side one can come later if the assistant needs it.
- Comparing word charts (on/off bands, modes): comparison is for numeric charts only.
- The Android app.

## Anti-goals (a failure even if it works)

- Existing callers break: dashboards stored with `"range": "24h"`, MCP calls with
  `range` / `days`, links to `/history/:id?range=7d`, the series cache of the cards.
- A long window scans raw `entity_state` rows (six years of a thermometer is millions of rows)
  or returns tens of thousands of points.
- Calendar periods cut at the browser's midnight instead of the house's: a day must be a
  house-local day, like `daily` and `grid_day` already do.
- The comparison compares unequal things without saying so: a running month against a whole
  month, or totals over days where one side has no data shown as a fall in usage.
- Bare UI strings (every word goes through `src/i18n`, FR and EN).

## Constraints

- Rust workspace: hub `crates/nidoo` (axum + sqlx/Postgres), wire types `crates/nidoo-api`, web
  `crates/nidoo-web` (Leptos, wasm). TDD.
- Web strings in `src/i18n` (FR/EN) or `tests/bare_text_allow.txt`; run `cargo test -p
  nidoo-web` (not only `--lib`). Never `wasm-pack test` over all web tests (fills the disk);
  run browser test files one at a time.
- Leptos: brace closure attributes before a self-closing `/>`.
- No real room, device, host or person names in code, tests or docs (public repo).
- Hourly rollups live in `history_hourly` (`crates/nidoo/src/history/rollup.rs`);
  `query::hours` already merges stored rows with a live rollup of the last hours.

## What exists today

- `nidoo_api::HistoryRange` (1h, 6h, 24h, 7d, 30d, 1y): `window_secs` (step) and `span_secs`,
  always ending now. Up to 24h the series is rolled up from raw samples; 7d and longer merge
  hourly buckets (`query::series`).
- `GET /history/series?entity=&range=`, `GET /history/fields?entity=&range=` (raw states, one
  track per value, used by the value chips and the word bands), `GET
  /history/daily?entity=&days=&kind=` (days must be 7, 30 or 365), `GET /history/today`, `GET
  /history/grid_day?date=`. The MCP `get_history` tool mirrors them (`GetHistoryParams`:
  `what`, `range`, `days`, `kind`).
- Web: `screens/history_detail.rs` (range chips, `?range=&show=`), `components/history_chart/`
  (`HistoryChart` takes `range: Signal<HistoryRange>`; counters on 7d/30d/1y draw daily kWh
  bars, on day ranges their paired power sensor), `api/history.rs`, `api/series_cache.rs`
  (cards' 24h series), `dashboards/history_widget.rs` (`DashboardHistoryRange`).
- The Energy tab's grid day card already steps day by day (`shift_day`, `first_date`).

## Approaches

1. **The hub takes any from–to window; the app does periods and comparison** — *chosen.* The
   history routes accept a window of house-local dates besides `range`; the hub picks the step
   from the window's length and resolves dates in the house zone. The app works out the
   calendar periods, fetches the shown and the compared window separately, aligns them and
   computes the summary. Smallest hub change that unlocks everything; MCP gains any window;
   presets keep working.
2. **The hub understands periods and comparisons** (`period=month&at=…&compare=…` returns both
   series aligned with totals). The assistant would get comparisons for free, but it is a much
   larger API carrying UI choices, for one screen. Rejected for now.
3. **App only**, stitching windows from today's presets. No hub work, but misaligned periods,
   wasted buckets on long spans and the 7/30/365-day limit on daily kWh stay. Rejected.

## Design

### Hub API

- **Window parameters.** `series`, `fields` and `daily` accept `from` and `to` as house-local
  dates (`YYYY-MM-DD`, both inclusive), as an alternative to `range` (series, fields) or `days`
  (daily). Exactly one of the two forms is required; both or neither is a 400. The window is
  `[local midnight of from, local midnight of the day after to)`, clipped at now; `to` before
  `from`, or `from` after today, is a 400. Dates (not unix seconds) keep the house zone on the
  hub and read naturally in MCP calls.
- **Step for a window**, by its length (same steps as today's presets where they overlap):
  up to 1 day → 600 s; up to 8 days → 1 h; up to 32 days → 5 h; up to 400 days → 1 day;
  longer → 7 days. Windows longer than 1 day read hourly buckets (`query::hours`), as 7d/30d/1y
  do today. A day-or-shorter window rolls up raw samples, as 24h does today.
- **`fields`** over a window reads raw states, so it is limited to windows of at most 400 days
  (today's 1y already does this); a longer window is a 400. The web does not ask for fields on
  longer windows (see below).
- **`daily`** over a window takes `per=hour`, `per=day` (default) or `per=month`: kWh per
  hour, per house-local day, or per calendar month (a month cut by the window counts only its
  days inside the window). `per=hour` windows are capped at 2 days, `per=day` at 1 100 days;
  `per=month` has no cap. `days` (7/30/365) keeps working unchanged.
- **Bucket alignment for a window**: buckets start at the window's local midnight (`from`) and
  step from there, not at epoch multiples as presets do, so day 1 of the compared year lines up
  with day 1 of the shown one. Presets keep their current alignment.
- **Empty window**: a recorded entity with no data in the window is a 200 with no points (or no
  days), never a 404; 404 keeps meaning "never recorded".
- **Extent.** A new `GET /history/extent?entity=` returns `{ first_date, today }`: the
  house-local date of the entity's first recorded value (`null` when none) and the house's
  current date. The web uses it to open on the current period, to disable ‹ before the first
  reading, and to list the years offered for comparison. MCP `get_history` gains
  `what: "extent"`.
- **MCP.** `GetHistoryParams` gains optional `from`, `to` and `per`, with the same rules and
  errors as HTTP; `range` and `days` keep working. The tool description says that a long
  window should use `daily` with `per=month` or a series.
- Errors and the 503 / 404 behaviour stay as today.

### Web: the history page

- **URL** (replace-navigation, as today, so Back leaves the page):
  `/history/:id?period=<1h|6h|day|week|month|year|custom>&at=<YYYY-MM-DD>&from=&to=&vs=<n>&show=`.
  - `at` is any date inside the shown period; absent means the current period.
  - `from`/`to` only with `custom`.
  - `vs` is how many years back the comparison sits (1 = the year before); absent means no
    comparison.
  - Old links keep working: `range=24h|7d|30d|1y` open the current Day/Week/Month/Year;
    `range=1h|6h` map to `period=1h|6h`.
- **Period row**, under the chips (see the mockup): ‹, the period name (e.g. "November 2026",
  "23 – 29 Nov 2026", "Thu 26 Nov 2026", "2026", "1 Nov 2025 – 31 Mar 2026") with "Today" /
  "This week" / "This month" / "This year" when it is the current one, ›, then "Back to now"
  when away from the current period. Clicking the period name, or the Custom chip, opens a
  from–to date picker (bounded by the first date and today) with quick picks: this winter
  (1 Nov – 31 Mar of the current or coming season), the last 12 months, all data.
- **Stepping.** ‹ › move by one period; a custom window moves by its own length. › is disabled
  when the next period starts after today; ‹ when the shown period starts on or before the
  first date. Switching unit keeps the date in view (Month of Nov 2024 → Day goes to
  1 Nov 2024; from the current period it goes to the current one).
- **Compare toggle**, on the right of the period row (stacked under it on a phone): off, or
  "Compare with [year ▾]". The years offered are those from the first date's year to the
  shown period's year minus one; the default is the year before. Stepping keeps the offset
  (`vs`), so going back from Nov 2026 vs 2025 gives Oct 2026 vs 2025. Hidden on 1h / 6h, on
  word charts, and when no earlier year has data.
- **Chart.**
  - Sensors (a line): the shown period in the existing mint line and band; the compared
    period as a dashed violet line on the same y-axis.
  - Counters: kWh bars per hour (Day), per day (Week, Month, custom up to 200 days) or per
    month (Year, longer custom windows), from `daily` with the window; the compared period as
    faint violet bars behind. (On Day this replaces today's 24h view of the paired power
    sensor, as in the approved mockup: the counter's own kWh reaches back as far as the
    counter does, its power pair often does not. 1h and 6h keep the power line.)
  - Alignment is by position in the period: the compared point at the same offset from its
    period's start (day 1 on day 1). A compared period longer than the shown one is cut at the
    shown period's end; a missing day (29 February) leaves a gap.
  - A dotted "now" line marks the current time on a running period.
  - The hover tooltip lists both values with their own dates.
  - Clicking a bar or point zooms in (Year → that month; Week/Month/custom → that day).
- **Summary cards** under the chart: shown period; compared period; difference. Counters:
  totals ("so far, to 26 Nov" while running), difference in kWh and %, the decrease in mint and
  the increase in rose. Sensors: mean with min/max, difference of the means. While the shown
  period runs, the compared side counts the same elapsed span only, and its card says "same
  days". When the compared side has no data for part of the span, its card says so instead of
  showing a smaller total as a saving.
- **Value chips** (`show=`) work as today on windows up to 400 days; on longer windows the
  chart draws the headline from the series only and the chips are hidden.
- **Refresh.** A window that contains now refetches like today's presets (each minute for a
  day or shorter, every 10 minutes otherwise); a past window is fetched once.
- **i18n.** Every new word in `src/i18n/history.rs` (FR and EN): period units, period names,
  "Back to now", "Compare with", the quick picks, the summary labels, "same days", "so far",
  the no-data note.

### Unchanged

Dashboard widgets (`DashboardHistoryRange`), the system chart, the series cache of the cards,
the Energy tab's day card, and every existing preset of the HTTP and MCP API.

### Settled after the builder check

- **Where the old years live.** The 2026-11-26 import wrote into raw `entity_state` (rows before
  each entity's first native row), and `rollup_state.done_until` was rewound so the hub rebuilt
  `history_hourly` from them. Raw rows are never deleted (only compressed after 7 days). So
  `extent.first_date` is the date of the entity's first `entity_state` row, `fields` and the
  raw Day roll-up work for any past period, and nothing needs a fallback.
- **The compared window** is the shown window shifted by `vs` calendar years, date by date
  (chrono's year shift; 29 February maps to 28 February). Day, Month, Year and custom use the
  same dates. **Week** compares the same ISO week (week 48 against week 48, Monday on Monday),
  because a week's pattern follows weekdays; its dates therefore differ by a day or two.
- **Same days only.** While the shown period runs, the compared window ends at the same offset
  from its start as now does from the shown start (26 Nov 15:40 → 26 Nov 15:40 a year back).
  The app requests the compared window clipped there, with the same `per` as the chart, so a
  Year view shows a partial last month on both sides; one call per side.
- **Gaps.** Totals and means are taken over the buckets present on **both** sides (a day with
  kWh on one side only is left out of both). When any bucket is left out, the summary says how
  many days (or hours, months) were left out of the comparison. A compared window with no data
  at all shows the shown period only, with "No data for <period>" in the compared card.
- **Sensor statistics.** The mean is the mean of the series' bucket means (buckets are equal in
  length, so it is the time-weighted mean up to empty buckets); min and max come from the
  buckets' min and max.
- **Series reply.** No new wire fields are needed: the app knows the window it asked for and
  places points by their offset from its own `from`.

## Left to the builder

- How `HistoryChart` takes its window: e.g. an enum of a preset range or a date window, so the
  dashboard widget keeps passing a preset.
- Where calendar maths lives on the web (period of a date, stepping, names), as pure, tested
  functions; the hub's existing `day_starts` / `local_midnight` / `day_bounds` for the hub side.
- The exact query-param parsing and the error texts.
- The date picker's markup (native `<input type="date">` is fine).
- Test layout, following the repo's split test modules.

## Mockup

`.superpowers/brainstorm/73428-1795724590/content/period-compare.html` (local, git-ignored),
approved by the user on 2026-11-26. Since then the comparison was narrowed to "the same period
of another year": the mockup's "Previous month" option is gone and the toggle becomes
"Compare with [year ▾]".
