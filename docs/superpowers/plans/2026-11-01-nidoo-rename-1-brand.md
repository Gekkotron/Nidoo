# Nidoo rename, phase 1: brand — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Everything a person reads or sees says **Nidoo** instead of Oikos / Oikos-os: logo, web UI, hub messages, README, docs, docs site and its URL.

**Architecture:** Text and asset changes only. Code identifiers (crate names, `oikos_*` modules, `OIKOS_*` env vars, `oikos/…` MQTT topics, `oikos_*` entity ids, `oikos.*` storage keys, container names, `oikos.toml`) are **not** touched here: phase 2 renames the code and deployment names, phase 3 the internal namespace. A CI script keeps the brand from coming back.

**Tech Stack:** Rust (hub `crates/oikos-os`, Leptos web UI `crates/oikos-web`), Docusaurus (`website/`, docs in `docs/`), GitHub Pages (`.github/workflows/docs.yml`), ImageMagick (`magick`) for the PNG icon.

**Spec:** The decisions recorded on 2026-11-01 (name chosen after GitHub, registry and domain checks): name **Nidoo**, logo direction **B · Peek** (canvas https://claude.ai/artifact/Jui6Dh4SQCixPiMxz1gXLa), logo sources in `docs/superpowers/plans/2026-11-01-nidoo-assets/`. Phase 2: `2026-11-01-nidoo-rename-2-code.md`.

## Global Constraints

- Product name: `Nidoo` (one word, capital N). Never `Nidoo-os`, `NidoOS` or `Nidoo OS`.
- GitHub repo: `Gekkotron/nidoo` (lowercase). Docs site: `https://gekkotron.github.io/nidoo/`.
- Never use the English word for a bird's home in tracked files: it is a Google smart-home brand. Describe the logo as a small round bird peeking out of a bowl. No name-origin line.
- Logo: the files in `docs/superpowers/plans/2026-11-01-nidoo-assets/`, unchanged except the header comment. Colours stay the web UI's tokens: navy `#0b1020`, cyan `#58e1ff`, amber `#ffc457`, Ivoire `#38738f` / `#8f6613`.
- Lowercase `oikos` in code, paths, topics, env vars, commands and ids stays as is in this phase, in docs too (``cargo run -p oikos-os`` stays until phase 2).
- `docs/superpowers/**` is history: never edited, except adding plans.
- Attribution in any new file header: `Gekkotron`. Project stays generic: no host names, IPs, rooms or family names in tracked files.
- Commit identity: `-c user.name=Gekkotron -c user.email=60887050+Gekkotron@users.noreply.github.com`.

## Review Focus

- A docs link built from `baseUrl` (images, `apple-touch-icon`, in-page links) still pointing at `/Oikos-os/` after the move → 404 on the new site. Task 3's link check covers it.
- The web UI's boot screen showing the animated mark at a size where the head is clipped or the bowl is cut by the 72 px box → the mark must sit whole inside the box. Task 1, step 5.
- `prefers-reduced-motion` users still seeing the bird move → the animated SVG already stops; Task 1 keeps its `@media` rule.
- A French string left in English (or the reverse) in `setup.rs` after the swap → Task 2 checks both columns.
- The repo rename done **after** the merge, so the Pages build deploys a `/nidoo/` site under `/Oikos-os/` → broken CSS. Task 4 orders the rename before the merge.

---

### Task 1: Logo assets

**Files:**
- Modify: `crates/oikos-web/assets/icon.svg`, `website/static/img/logo.svg`, `website/static/img/favicon.svg` (all three become the app icon)
- Create: `crates/oikos-web/assets/mark-animated.svg`, `website/static/img/mark-animated.svg` (copies of `nidoo-mark-night.svg`)
- Modify: `crates/oikos-web/assets/apple-touch-icon.png`, `website/static/img/apple-touch-icon.png` (180×180, regenerated)
- Modify: `crates/oikos-web/index.html` (boot image)

**Interfaces:**
- Produces: `/assets/icon.svg` (static icon on its navy plate; sidebar logo, favicon), `/assets/mark-animated.svg` (boot screen), `img/logo.svg`, `img/favicon.svg`, `img/mark-animated.svg` on the docs site. Task 3 uses `img/logo.svg` in the README and the navbar.

- [ ] **Step 1: Copy the sources**

`nidoo-app-icon.svg` → the three `icon.svg` / `logo.svg` / `favicon.svg`; `nidoo-mark-night.svg` → both `mark-animated.svg`. Put this first line in each: `<!-- Nidoo icon (Gekkotron). Source of the Android adaptive icon layers. -->` (the old files carry the same comment with Oikos-os).

- [ ] **Step 2: Regenerate the PNG icons**

Run: `magick -background none -density 384 crates/oikos-web/assets/icon.svg -resize 180x180 crates/oikos-web/assets/apple-touch-icon.png && cp crates/oikos-web/assets/apple-touch-icon.png website/static/img/apple-touch-icon.png`
Then: `magick identify crates/oikos-web/assets/apple-touch-icon.png`
Expected: `PNG 180x180`.

- [ ] **Step 3: Boot screen uses the animated mark**

In `crates/oikos-web/index.html`, the `#boot` image: `src="/assets/mark-animated.svg"`, `width="96" height="96"`. The favicon and apple-touch links keep their paths.

- [ ] **Step 4: Validate**

Run: `for f in crates/oikos-web/assets/*.svg website/static/img/*.svg; do xmllint --noout "$f" || echo "BAD $f"; done`
Expected: no `BAD` line.

- [ ] **Step 5: Look at it**

Run `tools/restart.sh` and open the web UI: the boot screen shows the bird rising out of the bowl, whole, centred above the spinner; the sidebar logo is the static icon and the double-click doorbell egg still shakes it. Screenshot both themes (headless Chrome via CDP, width ≥ 500 px).

- [ ] **Step 6: Commit**

```bash
git add crates/oikos-web/assets crates/oikos-web/index.html website/static/img
git commit -m "feat(brand): Nidoo logo, favicon and animated boot mark"
```

### Task 2: Hub and web UI text

**Files:**
- Modify: `crates/oikos-web/index.html:23` (`<title>`), `crates/oikos-os/src/http/static_ui.rs:19`
- Modify: `crates/oikos-web/src/i18n/settings.rs:272` (`T::ProductName`), `crates/oikos-web/src/i18n/setup.rs` (lines 67, 174, 220–244)
- Modify: `crates/oikos-web/src/screens/energy_day.rs:264` (docs link)
- Modify: `crates/oikos-os/src/cli.rs:6` (`about`), `crates/oikos-os/src/mcp/stdio_proxy.rs:27`, `crates/oikos-os/src/mcp/mod.rs:89` (server info), `crates/oikos-os/src/solar/entities.rs:78` (`manufacturer`)
- Create: `tools/ci/brand-check.sh`, `tools/ci/brand-allow.txt`
- Modify: `.github/workflows/ci.yml` (bundled stack job runs the check)

**Interfaces:**
- Produces: `tools/ci/brand-check.sh` — exits 1 and prints `file:line` for every `Oikos` (capital O, any suffix) in tracked files outside `docs/superpowers/` and the patterns in `tools/ci/brand-allow.txt` (one `git grep` pathspec or fixed string per line). Tasks 3 and phase 2 extend the allow list only when a line must keep the old name.

- [ ] **Step 1: Write the check and run it (fails)**

`brand-check.sh`: `git grep -n 'Oikos' -- . ':!docs/superpowers'` minus the allow list; shellcheck-clean, `set -euo pipefail`. Allow list starts with: `crates/*/tests/fixtures/**` (recorded MQTT payloads), `OikosPush` (the legacy project's name in comments and tests).
Run: `tools/ci/brand-check.sh`
Expected: FAIL, listing the strings of this task, the README, `docs/` and `website/`.

- [ ] **Step 2: Replace the strings**

| Where | New text |
|---|---|
| `<title>` (index.html, static_ui.rs) | `Nidoo` |
| static_ui.rs body | `The Nidoo web UI has not been built.` + the rest unchanged |
| `T::ProductName` | `("Nidoo", "Nidoo")` |
| `SetupText::WelcomeTitle` | `("Welcome to Nidoo", "Bienvenue dans Nidoo")` |
| setup.rs 220–244 | `Oikos` → `Nidoo` in both columns; FR `l'historique d'Oikos` → `l'historique de Nidoo` |
| energy_day.rs | `https://gekkotron.github.io/nidoo/docs/users/history` |
| cli.rs `about` | `Nidoo home-automation hub` (the `name = "oikos-os"` stays: it is the binary, phase 2) |
| stdio_proxy.rs | `cannot reach the Nidoo MCP server at {mcp_url}` |
| mcp/mod.rs server info | `Implementation::new("nidoo", …)` |
| solar/entities.rs | `"manufacturer": "Nidoo"` |

- [ ] **Step 3: Run the Rust checks**

Run (debug info off, per the repo's disk rule): `CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test -p oikos-os -p oikos-web && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check`
Expected: all pass. A test that asserted an old string is updated to the new one, not deleted.

- [ ] **Step 4: Wire the check into CI**

`ci.yml`, bundled stack job: `- run: tools/ci/brand-check.sh` after the shellcheck line (shellcheck already covers `tools/ci/*.sh`). The check still fails here; Task 3 makes it pass.

- [ ] **Step 5: Commit**

```bash
git add crates/oikos-web crates/oikos-os tools/ci .github/workflows/ci.yml
git commit -m "feat(brand): Nidoo in the web UI, hub messages and MCP server info"
```

### Task 3: README, docs and docs site

**Files:**
- Modify: `README.md`, `docs/intro.md`, `docs/users/*.md`, `docs/reference/*.md`, `docs/contributors/*.md`
- Modify: `website/docusaurus.config.ts`, `website/src/pages/index.tsx`, `website/src/css/custom.css:2`, `website/README.md`, `website/package.json` / `package-lock.json` (`name`)
- Modify: `tools/ci/brand-allow.txt` (only if a line truly must keep the old name)

**Interfaces:**
- Consumes: Task 1's `img/logo.svg`; Task 2's `tools/ci/brand-check.sh`.

- [ ] **Step 1: Prose rule, applied to every file listed**

`Oikos-os` and `Oikos` (product) → `Nidoo`; `the Oikos Android app` → `the Nidoo Android app`; `github.com/Gekkotron/Oikos-os` → `github.com/Gekkotron/nidoo`; `gekkotron.github.io/Oikos-os/` → `gekkotron.github.io/nidoo/`. Inside code blocks and backticks only the two URLs change. README logo `alt`: `Nidoo icon: a small round bird peeking out`; README `<h1>`: `Nidoo`.

- [ ] **Step 2: Docusaurus config**

`title: 'Nidoo'`, `baseUrl: '/nidoo/'`, `projectName: 'nidoo'`, apple-touch `href: '/nidoo/img/apple-touch-icon.png'`, `editUrl` → `https://github.com/Gekkotron/nidoo/edit/main/docs/${docPath}`, navbar `title: 'Nidoo'`, logo `alt: 'Nidoo logo'`, GitHub / Issues / LICENSE links → `Gekkotron/nidoo`, `copyright: 'Nidoo · Built by Gekkotron · Apache-2.0'`. Landing page (`index.tsx`): the four `Oikos-os` strings → `Nidoo`, hero shows `img/mark-animated.svg`.

- [ ] **Step 3: Build the site**

Run: `cd website && npm ci && npm run build`
Expected: build succeeds with Docusaurus's broken-link check passing; `grep -rl 'Oikos-os' build/ | wc -l` → `0`.

- [ ] **Step 4: Run the brand check**

Run: `tools/ci/brand-check.sh`
Expected: exit 0, no output.

- [ ] **Step 5: Commit**

```bash
git add README.md docs/intro.md docs/users docs/reference docs/contributors website tools/ci/brand-allow.txt
git commit -m "docs(brand): Nidoo across the README, docs and docs site"
```

### Task 4: Release (ordered; the user does the GitHub steps)

- [ ] **Step 1:** Open the PR; CI green (including `brand-check.sh` and the docs build).
- [ ] **Step 2 (user):** GitHub → Settings → rename `Oikos-os` to `nidoo`. GitHub redirects the old repo URL and git remotes; the old Pages URL is **not** redirected.
- [ ] **Step 3 (user):** Merge the PR. Check `.github/workflows/docs.yml` deploys and `https://gekkotron.github.io/nidoo/` loads with its CSS and logo.
- [ ] **Step 4:** `git remote set-url origin https://github.com/Gekkotron/nidoo.git` in every local clone/worktree; `git fetch` works.
- [ ] **Step 5 (user's go):** rebuild the web UI on the live hub; the tab title reads Nidoo and the boot screen shows the bird.
- [ ] **Step 6:** Out of this repo: the Android app's name and launcher icon (its own repo; the adaptive icon layers come from `crates/oikos-web/assets/icon.svg`). Optional for the user: rename the `oikos` MCP entry in their assistant configs; the server keeps working under either name.
