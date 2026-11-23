# History page: one chart, chips that combine — design

Date: 2026-11-23 · Status: draft for approval

## Intent

PR #240 put every value an entity reports on its history page as a stack of short rows
(`FieldTracks`): one row per value, on a shared time axis. On a real week of the Roborock the
stack reads as noise: "À la base" written five times because sub-pixel states (a 3 s clean, a
44 s return) split the docked bar, the actual cleaning runs are near-invisible slivers, the
battery dips carry smudgy min/max shading, and labels are truncated.

The owner wants **one full-size chart** and to **choose which values it shows**, with the
option to **combine** them — e.g. the battery line with the cleaning periods shaded behind it,
so the drop during each run is obvious.

## Goals

- One chart per history page, at the main chart's full size, for every entity kind.
- A row of chips under the range chips, one per value the entity reports; the user picks what
  the chart draws.
- A number value draws as the existing line chart (smooth line, area, tooltip, y axis, min/max
  band on bucketed ranges).
- A word value (vacuum activity, fan speed, HVAC mode…) draws as coloured periods across the
  full chart height, with a legend that names each value once.
- A number and a word value combine: the periods shade behind the line.
- Short events stay visible on long ranges.

## Non-goals

- Two number lines on one chart (two units, two y axes). Out — the reason combining is limited
  to one number.
- Changing the `/history/fields` or `/history/series` API. The data is already right.
- Energy counters (`Flavor::Counter`, daily kWh bars): unchanged, no chips.
- Comparing two entities on one chart.

## Anti-goals

- Anything that brings back repeated labels or unlabelled slivers — the reason for this change.
- A chart that becomes illegible because too much is drawn at once.

## Approaches considered

- **A. One value at a time** (chips are a single choice). Clean, half the work, but cannot show
  the battery drop against the cleaning runs.
- **B. Chips that combine: at most one number + at most one word value.** Chosen by the owner.
  Shows correlation, stays legible because only one line and one set of periods are ever drawn.
- **C. Keep the stacked rows, make them taller.** Least work; still small multiples, rejected.

## Design

### Chips (the selection)

- One chip per value, in the order the reply lists them: the entity's headline value first
  (the number `/history/series` draws, or the headline word track from `/history/fields`), then
  each field track (`battery`, `fan_speed`, …). Labels reuse `track_label` (Activité, Batterie,
  Vitesse d'aspiration, État…); the headline number chip uses the entity's own value word —
  *builder's call* on the exact wording (e.g. "Température" from the device class, else
  "Valeur").
- Two slots: **number** and **word**. Tapping a number chip puts it in the number slot,
  replacing the one there; same for word chips. Tapping a selected chip clears its slot, except
  when it is the only thing selected (the chart is never empty).
- **Default selection:** the headline (number or word) plus, when the headline is a word, the
  first number field. So a vacuum opens on Activité + Batterie, a thermometer on its
  temperature, a light on its on/off line.
- An entity with a single value shows no chip row (nothing to choose).
- The selection lives in the URL as `show=<key>,<key>` next to `range`, replace-navigation like
  the range chips. Key = the field name, or `state` for the headline. Unknown keys are ignored;
  if nothing valid remains, the default applies.

### Drawing a number

- The headline number keeps today's `HistoryChart` series path unchanged (`/history/series`,
  discrete step line for on/off kinds).
- A number field uses the same rendering with the track's `points`/`window`/`unit` from
  `/history/fields` (same `[t, mean, min, max]` shape). A `%` unit fixes the y axis at 0–100.

### Drawing a word value

- Periods come from the track's `changes` (`bands()` in `field_tracks/plan.rs` already turns
  them into stretches; reuse it).
- **Baseline value:** the value held for the longest total time in the window draws with no
  fill. Every other value gets a palette colour. For the vacuum that means docked is plain and
  cleaning / returning / error stand out. Unknown (`null`) stretches draw with no fill and are
  not in the legend.
- **Minimum width:** a coloured period is drawn at least ~3 px wide (in screen pixels, not
  viewBox units — the SVG stretches), so a 3 s clean on the 7 d view still shows as a tick.
- **Adjacent same values merge** before drawing (a sub-pixel interruption is still drawn as its
  own tick on top, but the baseline is never re-labelled).
- **No text inside periods.** A legend under the chart names each coloured value once with its
  swatch, and the baseline value as plain text ("À la base" with an empty swatch).
- **Alone** (no number selected): periods fill the full plot height at normal opacity.
- **Combined** with a number: periods fill the full plot height at low opacity behind the
  line's area, so the line stays the foreground.

### Tooltip

- Hovering anywhere shows the time, the number value at that point (as today) and, when a word
  value is selected, the word in force at that time ("Nettoyage"). With only a word value, the
  tooltip shows the word and the period's start–end, like today's band hover title.

### What goes away

- The stacked rows component (`components/field_tracks/mod.rs` view and its CSS) is removed.
  Its pure helpers (`bands`, `scalar_text`, `track_label`, `number_text`, `track_scale`) move to
  wherever the new chart needs them and keep their tests.
- The `headline` signal that hid the main chart for word-headline entities is replaced by the
  selection.

### Classifying values (author's call)

- A track is a **number** chip only when it is `TrackData::Number`. Every `Steps` track is a
  **word** chip drawn as periods, even when its values are numeric scalars (a fan level `3`).
- The headline is a **word** when the fields reply has a track with `field == None`
  (`has_headline`), else a **number** drawn from `/history/series`. Both keyed `state`.

### Loading and errors (author's call)

- Both fetches run in parallel (`/history/series`, `/history/fields`), refreshed on the same
  tick. The page cannot know the headline's type until the fields reply lands, so **chips and
  chart wait for it** (the chart's loading shimmer); the series reply is simply ignored when the
  headline turns out to be a word.
- Fields reply fails: number headline → draw it from the series reply, no chip row, no error
  banner. Word headline can't be known then, so the chart shows its normal error state with
  Retry (Retry refetches both).
- An entity whose only value is a headline word (no fields) shows no chip row and draws that
  word's periods alone.

### Selection vs. the reply (author's call)

- `show` is carried across range changes (`on_range` keeps it in the URL).
- Each key is checked against the current reply; unknown or absent keys are dropped silently
  (the URL is not rewritten). If no valid key remains, the default selection applies. At most
  one key per slot is honoured (the first of each type).

### Baseline and legend details (author's call)

- The baseline is the non-null value with the longest total time in the window; ties go to the
  value seen first. Unknown stretches never count.
- **A track with a single distinct value has no baseline**: that value draws coloured (otherwise
  "Turbo all week" would draw nothing).
- The legend lists the values present in the window, in order of first appearance, the
  baseline included as plain text with an empty swatch.

### Data source for a field number (author's call)

- `render_series` (or its replacement) takes the points, window, unit and kind as input instead
  of only the series fetch, so the headline series and a field number track share the same
  drawing, tooltip and y-axis code.

### Mobile

- Chips wrap onto as many lines as needed. Legend wraps. The chart keeps its current height.

## Testing

- Pure logic gets native unit tests: selection rules (slot replace, clear, never-empty, default,
  URL parse/format with unknown keys), baseline pick, merge of adjacent same values,
  minimum-width widening.
- One browser test (wasm-bindgen-test, like `tests/field_tracks.rs`) for a vacuum reply: chips
  render, default selection is Activité + Batterie, tapping Vitesse swaps the word slot, the
  legend names each value once.
- `tests/field_tracks.rs` is rewritten or replaced accordingly.
- New UI strings go through `src/i18n` (bare-text check).

## Left to the builder

- Component structure: extend `HistoryChart` or wrap it in a new component.
- Exact palette, opacities, and minimum-width value (≈3 px).
- The headline number chip's label wording.
- How the two fetches are coordinated in Leptos.
