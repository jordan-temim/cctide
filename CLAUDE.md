# cctide

A menu-bar / system-tray gauge for Claude Code usage. The Tauri app is at the
repo root.

> **Language policy:** everything in this repo is **English only** — code,
> comments, doc comments, UI strings, commit messages. No French in any file.

## What it is

A macOS menu-bar / Windows system-tray app that shows Claude Code usage,
similar to `claude.ai/settings/usage`, but **100% local — it never calls the
Anthropic API**. Built with **Tauri v2** (Rust backend + web UI).

The panel is organized into **five tabs** (Usage, Sessions, Settings, Analytics, Extras):

1. **Usage tab** — **Session (5h)** (15-segment fuel-gauge bar, rolling 5-hour window) and **Weekly limit** (15-segment fuel-gauge bar, anchored to user-entered reset date).
2. **Sessions tab** — **Open sessions**: one entry per interactive session (sub-agent processes are filtered out via the session file's `kind`; sessions that have consumed nothing yet — a fresh tab with no assistant turn, `context_tokens == 0` — are also hidden, in `context.rs::active_sessions`), showing context-window fill (e.g. 150k/200k), share of the current 5h window's consumption, idle/active status + last activity, and a VSCode/CLI badge. A **per-project filter** (the same dropdown component as Analytics, [`project-filter.ts`](src/project-filter.ts)) narrows the list to one cwd; its options are the cwds of the currently open sessions, and it auto-hides with fewer than two. The same filter also scopes the **Memory** section below (`get_memory` takes the selected cwd, reloaded when the filter changes while Memory is open). Per-session actions (two-step inline confirmation): **Copy resume** (`claude --resume <id>`), **Close** (SIGTERM the process), **Delete** (remove the transcript `.jsonl`; if the session has activity in the current 5h window, a reinforced warning explains the gauges will under-count until the next reset and to recalibrate after). A **Clean orphans** button removes `sessions/<pid>.json` files whose process is dead. Also hosts **Memory** (viewer of active sessions' project memory files, with per-file delete that also drops the file's line from the `MEMORY.md` index).
3. **Settings tab** — **Calibrate** (date picker + % fields to anchor the session/weekly bars), and **System notifications** (toggle + three configurable alert levels, default 33 / 66 / 90 %).
4. **Analytics tab** — **Weekly window** (chart of token consumption per model in the current 7-day window), plus **Outcomes** (collapsible, lazy-loaded): classifies the week's quota spend by what the work became in git — see "Outcomes" below.
5. **Extras tab** — **RTK** (tokens saved, only shown if the `rtk` binary is installed).

**Header controls.** Top-right of the panel: last-refresh timestamp + a **tracking toggle** (pause/resume all data refresh and icon updates; when off, the tray icon shows a diagonal slash over the empty C's). Below the header: five **tab buttons** (Usage, Sessions, Settings, Analytics, Extras) to switch between sections.

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

The **tray title** (text appended to the right of the CC icon, standard macOS
NSStatusItem behaviour) shows the 5h window's reset time in `HH:MM` local format
when a live session is running (`session.reset_at` → `reset_time_label()` in
`tick.rs`); cleared to empty when no session is active or tracking is disabled.

**Dev builds** draw a **"D" glyph inside the left C** (black on macOS, orange on
Windows/Linux), compiled in via `cfg!(debug_assertions)` and absent from release
binaries. It sits in the *left* C deliberately, so it never collides with the
**"U" update glyph** drawn in the *right* C (see "Releases & auto-update").

## Local data sources (no network)

Everything is read from `~/.claude`:

- **Token usage**: `~/.claude/projects/<project>/<sessionId>.jsonl` — each
  `type:"assistant"` line has `message.usage` (`input_tokens`, `output_tokens`,
  `cache_creation_input_tokens`, `cache_read_input_tokens`), `message.model`,
  `timestamp`. Records are **deduplicated** by `message.id`+`requestId`
  (`scan.rs`): transcripts log the same API response multiple times (resumes,
  sidechains, multiple files), which would otherwise ~2× the counts.
- **Active sessions**: `~/.claude/sessions/<pid>.json` — one per running Claude
  Code process (`pid`, `sessionId`, `cwd`, `version`, plus on recent versions
  `kind`, `entrypoint`, `status`, `updatedAt`). PIDs are checked for liveness;
  non-`interactive` kinds are filtered out.
- **Memory**: `~/.claude/projects/<project>/memory/*.md`.
- **App config**: the app's own data dir from the bundle id `com.cctide`
  (macOS `~/Library/Application Support/com.cctide/cctide.json`; Windows
  `%APPDATA%\com.cctide\`; Linux `~/.config/com.cctide/`). Holds calibration
  anchors, context-limit overrides, refresh interval, `notifications_enabled`,
  `alert_levels`, `dynamic_icon`, `tracking_enabled`.

Quota weights, pricing and model metadata are **not** in `~/.claude` — they ship
with the app at [`models.json`](models.json).

A background ticker thread (spawned in `lib.rs` via `tick::start_ticker()`) re-evaluates usage every
`refresh_secs` and fires native notifications via `notify.rs` when a threshold is
crossed — independently of whether the panel is open. It is edge-triggered: one
notification per crossing, re-armed once the bar drops back below the threshold.

The official weighted % from claude.ai is **not** stored locally, so the
session/weekly bars are reconstructed by **single-point calibration**: the user
reports the % shown by `/usage` **once**. cctide derives a budget
(`budget = K_now / (percent/100)`) and then `percent = weighted / budget × 100`
(linear through the origin). A single point currently suffices because the fitted
relationship looks close to linear through the origin; the former two-point fit
(`percent = a·tokens + b`) and its recalibration nudge were removed.

**Plan-agnostic design.** cctide never stores or asks for the user's plan
(Pro / Max 5× / Max 20×). The plan only changes the absolute size of the quota
(what 100% is worth in tokens/dollars). Calibration captures this automatically.
Per-model quota weights and the 5h/weekly window mechanics are identical across
plans. If the user changes plans, they recalibrate once.

## Calculation model

- Consumption is summed as **quota-weighted tokens** using **empirical quota
  weights** (per model), not API prices:
  `weight = input·w_in + output·w_out + cache_write_5m·w_5m + cache_write_1h·w_1h`
  (cache reads are not currently fed in). The weights are **provisional estimates**
  from the regression below, not published figures — the exact quota formula isn't
  public, so treat them as a best-effort approximation refined as more data arrives.
- **Model data is a JSON file shipped with the app**, not hard-coded:
  [`models.json`](models.json) at the app root, compiled into the binary via
  `include_str!`. Each model carries **two independent weight sets**:
  `input`/`output`/`cache_write_*` are the **$/MTok prices** (reference/info only),
  and a **`quota`** block holds the empirical quota weights used by
  `quota_units`. Plus context window (tokens). Edit it when Anthropic changes the
  quota mechanics / prices or ships new models, then rebuild. Nothing is written to
  `~/.claude`. Parsing/fallback defaults live in
  [`models.rs`](src-tauri/src/models.rs). The quota weights are re-derivable from
  scratch (see "Deriving the quota weights"). Only ratios matter (calibration
  normalises scale).
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
  Some models technically accept more than 200k tokens (e.g. claude-sonnet-4-6's
  1M theoretical limit), but beyond ~200k answer quality tends to drop and each
  turn gets far more token-hungry, so Claude Code works against an effective
  **~200k context** (auto-compacting around there). All entries in `models.json`
  use 200k as `context_window`. Verified 2026-05-31 via `/context`
  in Claude Code showing `148.5k / 200.0k` for a claude-sonnet-4-6 session.

## Outcomes (Analytics tab)

Classifies the weekly window's quota spend by the fate of each session's work
in git (no other tool does this in quota terms; codeburn's `yield` does it in $).
Backend in [`outcome.rs`](src-tauri/src/outcome.rs), session data from
`scan.rs::session_edit_spans` (each transcript's `cwd` + its Edit/Write tool
calls with timestamps — the project folder name can't be decoded back to a
path, `encode_cwd` is lossy, so the `cwd` field in the transcript lines is the
only reliable source). **One span per interactive session:**
`session_edit_spans` folds **sub-agent transcripts** (Claude Code stores them
under `…/<sessionId>/subagents/agent-*.jsonl`, picked up by the recursive
`WalkDir` scan) into their parent session via `session_root()` — their quota and
edits are attributed to the parent. Without this each sub-agent would count as
its own "session", inflating every category's `session_count` (the Sessions tab
already filters sub-agents out, so this keeps the two notions of "a session"
consistent). Token totals stay correct either way: the weekly gauges dedup
globally by `key`, and folded points are deduped by `key` per session.

**Per-edit model.** A commit carries every uncommitted change to the files it
touches, so for each Edit/Write the **first commit touching that file
afterwards** is the one that shipped it. Commits slice time per file: several
sessions editing the same file each resolve unambiguously, and two sessions
sharing one commit are both legitimately credited. A session is classified by
the **majority fate of its edits** (each edit resolves on its own; the fate held
by the most edits wins, ties breaking toward the better outcome
Shipped > OnBranch > Reverted > Uncommitted): **shipped** (commits reachable from main, not
reverted), **on_branch** (committed on a branch not yet on main), **reverted**
("This reverts commit <sha>" matched by SHA, never by keyword), **uncommitted**
(no commit carries those edits — typically still in the working tree),
**non_repo** (session outside any git repo). Sessions with a repo but no edits
fall back to a coarse temporal rule (commits during the session +1h). The
displayed % is each category's share of the window's weighted quota.

Git usage is **strictly read-only** (`rev-parse`, `symbolic-ref`, `log` — an
invariant of `outcome.rs`), batched **once per repo** (two `log` passes: all
branches with `--name-only`, and main-only SHAs); the main branch is whatever
`origin/HEAD` says (falls back to local `main`, then `master`). Computation is
**lazy**: `get_outcomes` runs only when the panel section is opened, cached
300 s in `AppState.outcome_cache`, never from `do_tick`.

Known v1 limits: file-level matching, not line-level (hand-rewriting a
session's work before committing still credits it); squash merges leave
sessions on_branch/uncommitted (branch SHAs never reach main); renames not
followed.

## Deriving the quota weights

The `quota` weights in `models.json` are **empirical**, not guessed: they were fit
by regressing the real `/usage` 5h % (ground truth) against the local token counts.
The method:

- Each **measurement point** pairs a 5h window's deduped token sums — per model
  family and per category (input / output / cache_write_5m / cache_write_1h /
  cache_read, plus request count and web-tool calls) — with the `/usage` % reported
  at that instant. Points are taken right after `/usage`, anchored on the displayed
  reset, so the measuring turn itself doesn't pollute the snapshot.
- Candidate formulas are fit with **non-negative least squares** (NNLS; weights ≥ 0)
  and ranked by **leave-one-window-out cross-validation** (LOWO-RMSE: drop a whole 5h
  session and predict its points — honest error, since points inside one window are
  auto-correlated). Confidence intervals come from a **block bootstrap over windows**.

Preliminary observations (provisional, limited data — **not settled facts**): a
plain price-as-weight model fits poorly; output appears to carry most of the
weight; opus and sonnet look interchangeable but fable measures ~3.3× sonnet
(close to its output price ratio); `cache_write_1h` tracks ~0.11× the model's own
output weight (cross-validated on windows with very different output/cache mixes);
cache reads seem to matter little; and the fit looks close to linear through the
origin (which is why a single calibration point currently suffices). A plan change
only rescales the budget (Max 5× measured at ~5.3× Pro), which calibration absorbs.
The remaining soft spots are the resume regime (big cache rewrite, low output) and
`cache_write_5m` (rarely exercised). Re-run the collection as more data accrues, or
if the quota mechanics seem to shift.

## Project layout (repo root)

```
src/                  Frontend (Vite + vanilla TS)
  index.html
  main.ts             App entry point + tab routing + event listener
  styles.css
  tab-usage.ts        Usage tab: session/weekly bars
  tab-sessions.ts     Sessions tab: open sessions + actions + memory viewer
  tab-settings.ts     Settings tab: calibration + notification levels
  tab-analytics.ts    Analytics tab: weekly window charts + outcomes
  tab-extras.ts       Extras tab: RTK integration
  project-filter.ts   Reusable per-project filter dropdown (Analytics + Sessions)
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
    tick.rs           background ticker thread (refresh loop, tray title, notifications)
    update_svc.rs     update check + install + restart
    scan.rs           JSONL discovery + parsing + mtime cache
    usage.rs          5h window + weekly calibration math
    context.rs        per-session context window
    outcome.rs        edit-fate classification via read-only git (Outcomes)
    memory.rs         memory file reader
    rtk.rs            `rtk gain --format json` integration (optional)
    notify.rs         threshold-crossing native notifications (de-duped)
    icon.rs           runtime CC-gauge tray icon (mac mono+blink / win colour)
    config.rs         persisted config load/save (calibration, thresholds)
    models.rs         per-model data (models.json): quota weights + prices, context window
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
cargo test --manifest-path src-tauri/Cargo.toml     # Rust unit tests
npx tsc --noEmit           # frontend typecheck
npm test                   # frontend unit tests (Vitest)
```

Tests: Rust units live in each module's `#[cfg(test)]`; the frontend uses
**Vitest** for pure helpers (e.g. `nextWeeklyReset` in `utils.ts` → `utils.test.ts`).
Both run in CI (`lint.yml`).

Builds are **unsigned** (no Apple/Windows code-signing certificate) — see
`README.md` for the first-launch steps users must take.

> **Dev tray icon invisible on macOS?** macOS gates menu bar icons per app:
> System Settings → Control Center → **Allow in the Menu Bar**. The
> bare dev binary (`target/debug/cctide`, no `.app` bundle, generic icon) gets
> its **own entry** there, separate from the installed app — if its toggle is
> off, the tray item is silently hidden while the app runs fine and the logs
> still say "tray icon created". Check that panel before suspecting the code.
> Startup milestones and a panic hook print to stderr (the `tauri dev`
> terminal) since v0.6.x — release builds have no console.

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
