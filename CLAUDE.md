# ccgauge

A menu-bar / system-tray gauge for Claude Code usage. The Tauri app is at the
repo root.

> **Language policy:** everything in this repo is **English only** — code,
> comments, doc comments, UI strings, commit messages. No French in any file.

## What it is

A macOS menu-bar / Windows system-tray app that shows Claude Code usage,
similar to `claude.ai/settings/usage`, but **100% local — it never calls the
Anthropic API**. Built with **Tauri v2** (Rust backend + web UI).

Panel contents:
1. **Session (5h)** — 15-segment fuel-gauge bar (each segment ≈ 6.7%), rolling 5-hour window.
2. **Weekly limit** — 15-segment fuel-gauge bar, anchored on a user-entered reset date.
3. **Open sessions** — each active session's context-window fill (e.g. 150k/200k).
4. **Memory** — read-only viewer of the active sessions' project memory files.
5. **RTK** — tokens saved (only shown if the `rtk` binary is installed).
6. **System notifications** — toggle (in section header) + the three configurable
   **alert levels** (default 33 / 66 / 90 %). The section title is set dynamically
   at startup to "macOS notifications" or "Windows notifications" via `navigator.userAgent`.
   The toggle is a pill identical to the tracking toggle in the app header; clicking
   it does not expand/collapse the section.
7. **Calibrate** — date picker + % fields to anchor the session/weekly bars.
8. **Weekly models** — breakdown of token consumption per model in the current window.

**Header controls.** Top-right of the panel: last-refresh timestamp + a **tracking toggle** (pause/resume all data refresh and icon updates; when off, the tray icon shows a diagonal slash over the empty C's).

**Unified alert levels.** The three levels (default 33/66/90%, in `alert_levels`)
drive everything at once — the session/weekly **segment colours**, the **tray icon**,
and the **OS notifications**. `level_for()` in `config.rs` maps a % to a level 0..3.

The **tray icon is live**: the two C's fill with session (left) / weekly (right)
usage. As usage crosses the levels it escalates: on macOS the icon blinks (faster
per level) until the panel is opened (acknowledged), re-arming when a higher level
is reached; on Windows each C is tinted green→orange→red. The icon reacts
independently of the notifications toggle (gated only by `dynamic_icon`).
Rendered in `icon.rs`, driven by a ~400 ms thread in `lib.rs` that also fires the
notifications (`notify.rs`, once per level crossing, gated by
`notifications_enabled`). macOS notifications need permission (requested at
startup) and only surface reliably from the installed build.

## Local data sources (no network)

Everything is read from `~/.claude`:

- **Token usage**: `~/.claude/projects/<project>/<sessionId>.jsonl` — each
  `type:"assistant"` line has `message.usage` (`input_tokens`, `output_tokens`,
  `cache_creation_input_tokens`, `cache_read_input_tokens`), `message.model`,
  `timestamp`. Records are **deduplicated** by `message.id`+`requestId`
  (`scan.rs`): transcripts log the same API response multiple times (resumes,
  sidechains, multiple files), which would otherwise ~2× the counts.
- **Active sessions**: `~/.claude/sessions/<pid>.json` — one per running Claude
  Code process (`pid`, `sessionId`, `cwd`, `version`). PIDs are checked for
  liveness.
- **Memory**: `~/.claude/projects/<project>/memory/*.md`.
- **App config**: the app's own data dir from the bundle id `com.ccgauge`
  (macOS `~/Library/Application Support/com.ccgauge/ccgauge.json`; Windows
  `%APPDATA%\com.ccgauge\`; Linux `~/.config/com.ccgauge/`). Holds calibration
  anchors, context-limit overrides, refresh interval, `notifications_enabled`,
  `alert_levels`, `dynamic_icon`.

Pricing and model metadata are **not** in `~/.claude` — they ship with the app at
[`models.json`](models.json).

A background ticker thread (spawned in `lib.rs::run`) re-evaluates usage every
`refresh_secs` and fires native notifications via `notify.rs` when a threshold is
crossed — independently of whether the panel is open. It is edge-triggered: one
notification per crossing, re-armed once the bar drops back below the threshold.

The official weighted % from claude.ai is **not** stored locally, so the
session/weekly bars are reconstructed by **manual calibration**: the user
reports the % shown by `/usage`, and we derive a token "budget" from it, then
track deltas from the local JSONL. Re-calibrate any time it drifts.

**Plan-agnostic design.** ccgauge never stores or asks for the user's plan
(Pro / Max 5× / Max 20×). The plan only changes the absolute size of the quota
(what 100% is worth in tokens/dollars). Calibration captures this automatically:
`budget = tokens_so_far / (percent / 100)`. A Pro user calibrating at 6% and a
Max user calibrating at 6% will each get a plan-appropriate budget. Per-model
pricing ratios and the 5h/weekly window mechanics are identical across plans. If
the user changes plans, they re-calibrate once.

## Calculation model

- Consumption is summed as **quota-weighted tokens**, using Anthropic's pricing
  as the weights (per model), but **excluding cache reads**:
  `weight = input·input_price + output·output_price + cache_write_5m·… +
  cache_write_1h·…` (e.g. Opus 5/25, Sonnet 3/15, Haiku 1/5 for input/output).
  Cache reads are omitted because Anthropic's rate-limit metering counts
  `input + cache_creation` and **not** `cache_read`
  (<https://platform.claude.com/docs/en/api/rate-limits>); counting them made the
  estimate balloon with conversation length and drift upward. Raw cache-read
  tokens are still shown in **Models used** (`scan::Point::tokens`).
- **Model data is a JSON file shipped with the app**, not hard-coded:
  [`models.json`](models.json) at the app root, compiled into the binary via
  `include_str!`. Contains per-model: input/output/cache-write weights ($/MTok,
  **no cache_read**), context window (tokens). Edit it when Anthropic changes prices or releases new
  models, then rebuild. Nothing is written to `~/.claude`. Parsing/fallback
  defaults live in [`models.rs`](src-tauri/src/models.rs). Source:
  <https://platform.claude.com/docs/en/about-claude/pricing> and
  <https://platform.claude.com/docs/en/about-claude/models/overview>, **captured
  2026-05-30**. Only the pricing ratios matter (calibration normalises scale).
- Calibration absorbs the absolute scale: `budget = K_now / (percent/100)`,
  then `percent = weighted_now / budget × 100`.
- **Session**: rolling 5h window. **Weekly**: `week_start = reset_date − 7d`,
  rolling forward as the reset date passes.
- **Context per session**: full token sum of the latest assistant turn
  (`input + output + cache_creation + cache_read`) vs the model's context limit.
  Claude Code uses an effective **200k-token context** for all current models,
  regardless of a model's theoretical maximum (e.g. claude-sonnet-4-6 has a 1M
  theoretical limit but Claude Code auto-compacts at 200k). All entries in
  `models.json` use 200k as `context_window`. Verified 2026-05-31 via `/context`
  in Claude Code showing `148.5k / 200.0k` for a claude-sonnet-4-6 session.

## Project layout (repo root)

```
src/                  frontend (Vite + vanilla TS) — index.html, main.ts, styles.css
src-tauri/
  Cargo.toml          Rust manifest
  build.rs            Tauri build script
  tauri.conf.json     Tauri configuration (bundle ID, window, permissions)
  src/
    lib.rs            Tauri commands + tray + popup window wiring
    main.rs           binary entry point
    scan.rs           JSONL discovery + parsing + mtime cache
    usage.rs          5h window + weekly calibration math
    context.rs        per-session context window
    memory.rs         memory file reader
    rtk.rs            `rtk gain --format json` integration (optional)
    notify.rs         threshold-crossing native notifications (de-duped)
    icon.rs           runtime CC-gauge tray icon (mac mono+blink / win colour)
    config.rs         persisted config load/save (calibration, thresholds)
    models.rs         per-model data (models.json): pricing, context window
```

## Develop / build

Toolchain: Node + npm, Rust (with `aarch64-apple-darwin` + `x86_64-apple-darwin`
targets for the universal macOS build), Xcode Command Line Tools (macOS).

```sh
npm install
npm run tauri dev          # run with hot reload
npm run build:mac          # macOS universal .dmg → build/  (run on a Mac)
npm run build:win          # Windows .msi → build\          (run on a Windows machine)
cargo check --manifest-path src-tauri/Cargo.toml    # fast Rust check
npx tsc --noEmit           # frontend typecheck
```

Builds are **unsigned** (no Apple/Windows code-signing certificate) — see
`README.md` for the first-launch steps users must take.
