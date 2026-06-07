# cctide

A menu-bar / system-tray gauge for Claude Code usage. The Tauri app is at the
repo root.

> **Language policy:** everything in this repo is **English only** — code,
> comments, doc comments, UI strings, commit messages. No French in any file.

## What it is

A macOS menu-bar / Windows system-tray app that shows Claude Code usage,
similar to `claude.ai/settings/usage`, but **100% local — it never calls the
Anthropic API**. Built with **Tauri v2** (Rust backend + web UI).

The panel is organized into **four tabs** (Usage, Settings, Analytics, Extras):

1. **Usage tab** — **Session (5h)** (15-segment fuel-gauge bar, rolling 5-hour window), **Weekly limit** (15-segment fuel-gauge bar, anchored to user-entered reset date), and **Open sessions** (each active session's context-window fill, e.g. 150k/200k).
2. **Settings tab** — **Calibrate** (date picker + % fields to anchor the session/weekly bars), and **System notifications** (toggle + three configurable alert levels, default 33 / 66 / 90 %).
3. **Analytics tab** — **Weekly window** (chart of token consumption per model in the current 7-day window), and **Memory** (read-only viewer of active sessions' project memory files).
4. **Extras tab** — **RTK** (tokens saved, only shown if the `rtk` binary is installed).

**Header controls.** Top-right of the panel: last-refresh timestamp + a **tracking toggle** (pause/resume all data refresh and icon updates; when off, the tray icon shows a diagonal slash over the empty C's). Below the header: four **tab buttons** (Usage, Settings, Analytics, Extras) to switch between sections.

**Unified alert levels.** The three levels (default 33/66/90%, configured in Settings tab under "System notifications")
drive everything at once — the session/weekly **segment colours**, the **tray icon**,
and the **OS notifications**. `level_for()` in `config.rs` maps a % to a level 0..3.

The **tray icon is live**: the two C's fill with session (left) / weekly (right)
usage. As usage crosses the levels it escalates: on macOS the icon blinks (faster
per level) until the panel is opened (acknowledged), re-arming when a higher level
is reached; on Windows each C is tinted green→orange→red. The icon reacts
independently of the notifications toggle (gated only by `dynamic_icon`).
Rendered in `icon.rs`, driven by a ticker thread in `tick.rs` (every
`refresh_secs`, default 60 s) via `do_tick()`, which also fires notifications
(`notify.rs`, once per level crossing, gated by `notifications_enabled`).
`do_tick` is also called immediately — in a spawned thread — after each mutation
(via commands like `set_calibration`, `set_tracking`, `set_notifications`) so the icon and panel
update without waiting for the next tick. The shimmer animation (5 frames ×
400 ms sweep) plays on every `do_tick` call. macOS notifications need permission
(requested at startup) and only surface reliably from the installed build.

**Dev builds** show a small dot at the centre of the right C (black on macOS,
orange on Windows/Linux), compiled in via `cfg!(debug_assertions)` and absent
from release binaries.

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
- **App config**: the app's own data dir from the bundle id `com.cctide`
  (macOS `~/Library/Application Support/com.cctide/cctide.json`; Windows
  `%APPDATA%\com.cctide\`; Linux `~/.config/com.cctide/`). Holds calibration
  anchors, context-limit overrides, refresh interval, `notifications_enabled`,
  `alert_levels`, `dynamic_icon`, `tracking_enabled`.

Pricing and model metadata are **not** in `~/.claude` — they ship with the app at
[`models.json`](models.json).

A background ticker thread (spawned in `lib.rs` via `tick::start_ticker()`) re-evaluates usage every
`refresh_secs` and fires native notifications via `notify.rs` when a threshold is
crossed — independently of whether the panel is open. It is edge-triggered: one
notification per crossing, re-armed once the bar drops back below the threshold.

The official weighted % from claude.ai is **not** stored locally, so the
session/weekly bars are reconstructed by **two-point calibration**: the user
reports the % shown by `/usage` twice (once at first launch, then again when
cctide notifies them ~25 percentage-points later). The two points let cctide fit
`percent = a·tokens + b`, correcting both scale error and any constant offset
between local token weights and Anthropic's internal metering. Until the second
point is saved the bar uses a single-point fallback (`budget = tokens / (pct/100)`).
The two most recent calibration points are always kept; a third replaces the oldest.

**Plan-agnostic design.** cctide never stores or asks for the user's plan
(Pro / Max 5× / Max 20×). The plan only changes the absolute size of the quota
(what 100% is worth in tokens/dollars). Calibration captures this automatically.
Per-model pricing ratios and the 5h/weekly window mechanics are identical across
plans. If the user changes plans, they restart the two-step calibration.

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
  2026-06-03**. Only the pricing ratios matter (calibration normalises scale).
- Calibration absorbs the absolute scale: `budget = K_now / (percent/100)`,
  then `percent = weighted_now / budget × 100`.
- **Session**: rolling 5h window. **Weekly**: rolling 7-day window anchored to
  `reset_date`. `week_start` is the most recent past occurrence of `reset_date`
  (found by stepping backward in 7-day increments until `week_start ≤ now`);
  `next_reset = week_start + 7d`. Works correctly whether `reset_date` is in
  the past or more than 7 days in the future (e.g. the user's next upcoming
  reset at first launch).
- **Context per session**: full token sum of the latest assistant turn
  (`input + output + cache_creation + cache_read`) vs the model's context limit.
  Claude Code uses an effective **200k-token context** for all current models,
  regardless of a model's theoretical maximum (e.g. claude-sonnet-4-6 has a 1M
  theoretical limit but Claude Code auto-compacts at 200k). All entries in
  `models.json` use 200k as `context_window`. Verified 2026-05-31 via `/context`
  in Claude Code showing `148.5k / 200.0k` for a claude-sonnet-4-6 session.

## Project layout (repo root)

```
src/                  Frontend (Vite + vanilla TS)
  index.html
  main.ts             App entry point + tab routing + event listener
  styles.css
  tab-usage.ts        Usage tab: session/weekly bars + open sessions
  tab-settings.ts     Settings tab: calibration + notification levels
  tab-analytics.ts    Analytics tab: weekly window chart + memory viewer
  tab-extras.ts       Extras tab: RTK integration
  types.ts            Shared TypeScript types (PanelData, Config, etc.)
  update.ts           Update banner + install/restart logic
  utils.ts            DOM helpers ($, updateLastUpdated)
src-tauri/
  Cargo.toml          Rust manifest
  build.rs            Tauri build script
  tauri.conf.json     Tauri configuration (bundle ID, window, permissions)
  src/
    lib.rs            Tauri plugins, tray, popup window, module wiring
    main.rs           binary entry point
    commands.rs       Tauri command handlers (invoke → Rust)
    state.rs          AppState struct + shared mutable state
    tick.rs           background ticker thread (refresh loop + notifications)
    update_svc.rs     update check + install + restart
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

## Releases & auto-update

### CI pipeline (`.github/workflows/release.yml`)

Triggered by pushing a `v*` tag (real release) or `workflow_dispatch` (test run,
no GitHub Release). The pipeline is a single chain so a bad commit can't ship:

```
lint ──┬──► build-frontend ──► build-mac ──┬──► release
security ──┘                └──► build-win ──┘
```

- `lint` / `security` run the existing `lint.yml` / `security.yml` via
  `workflow_call` (they expose it alongside their own push/PR triggers). They are
  **blocking** — builds don't start unless both pass. Those workflows use
  minimal **per-job** `permissions` (not `read-all`), otherwise `workflow_call`
  fails because the caller can't grant more than it holds. `release.yml` grants
  `contents: write` (create the release) + `security-events: write` (gitleaks /
  semgrep).
- `build-frontend` runs `npm run build` once on Ubuntu and uploads `dist/`. The
  two OS build jobs download it and patch `beforeBuildCommand` to `""` via `jq`
  so Tauri doesn't rebuild the frontend on the (slower, pricier) mac/win runners.
- `build-mac` builds the universal target; `build-win` builds the MSI. Both copy
  their outputs into a flat `upload/` dir before `upload-artifact` — otherwise
  the action keeps the `dmg/` + `macos/` subdirs (least-common-ancestor
  behaviour) and the release job's `artifacts/*.app.tar.gz` glob misses them. The
  `cp` also fails loudly in the build job if an expected file is absent, instead
  of the multi-path upload silently skipping it.
- `release` (tag only) downloads both artifact sets, generates `latest.json`, and
  publishes the GitHub Release with the `.dmg`, `.msi`, their `.sig`s, and
  `latest.json` attached.

### Signing & updater bundles

Updates are verified with a **Tauri signing keypair** (separate from OS code
signing, which we don't have). The keypair was generated once with
`tauri signer generate`; the **public key** is in `tauri.conf.json`
(`plugins.updater.pubkey`, compiled into every binary), the **private key** is the
`TAURI_SIGNING_PRIVATE_KEY` GitHub Actions secret. Because the pubkey is baked
into the binary, it **cannot change** without breaking updates for installed
clients.

`bundle.createUpdaterArtifacts: true` is what makes `tauri build` emit the
`.app.tar.gz` (+ `.sig`) on macOS and the `.msi.sig` on Windows — without it the
build only produces the installer and the updater has nothing to fetch. The
updater downloads the `.app.tar.gz` / `.msi` (not the `.dmg`, which is
install-only), so `latest.json`'s `darwin-universal` URL points at the
`.app.tar.gz`.

`latest.json` is served from the **latest** GitHub Release
(`releases/latest/download/latest.json`), which is also the `endpoints` value in
`tauri.conf.json`. Shape:

```json
{
  "version": "v0.2.6",
  "pub_date": "…Z",
  "platforms": {
    "darwin-universal": { "url": "…/cctide.app.tar.gz", "signature": "…" },
    "windows-x86_64":   { "url": "…/cctide_x64.msi",    "signature": "…" }
  }
}
```

### Client behaviour (`update_svc.rs` + `update.ts`)

Updates are **user-initiated**, not silent. Detection is **check-only**:
`spawn_update_check` (backend) runs at startup and then every `UPDATE_CHECK_INTERVAL` (2h)
via a background thread. When a newer version is found it records an `UpdateInfo`
(version, release notes, GitHub release-tag URL) in `AppState.available_update`,
sets `UPDATE_AVAILABLE`, and fires an OS notification **once per version**
("open cctide to install"). It never downloads on its own.

Surfaced two ways: a **panel banner** (`#update-banner`, fed via `PanelData.update`
on the normal refresh poll) showing "Update available: vX.Y.Z" + a "What's new"
link (opens the release page via the opener plugin), and a **"U" glyph** drawn in
the right C of the tray icon (`icon.rs`, gated by `IconParams.update_available`,
driven by `UPDATE_AVAILABLE`).

The user clicks **Install** → the `install_update` command downloads + installs
(`UPDATE_STAGED` set on success); the button becomes **Restart now** →
`restart_app` command calls `app.restart()`. We never force-restart on our own.
Guards: `UPDATE_CHECKING` (no concurrent checks), `UPDATE_STAGED` (stop checks
once staged).

> The first version able to **receive** updates is the first release that shipped
> a working signed `.app.tar.gz` + `latest.json`. Earlier installs must be
> updated manually.

> `latest.json`'s `version` field must be **plain semver** (no leading `v`) or the
> updater silently fails to parse it; the macOS platform keys are
> **`darwin-x86_64` + `darwin-aarch64`** (both pointing at the universal
> `.app.tar.gz`) — the updater matches by runtime arch, not `darwin-universal`.
> Both are handled in `release.yml`'s "Generate latest.json" step.
