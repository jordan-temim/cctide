# cctide

<p align="center">
  <img src="docs/app-icon-master.png" width="96" />
</p>

<p align="center">
  <a href="https://github.com/jordan-temim/cctide/actions/workflows/security.yml"><img src="https://github.com/jordan-temim/cctide/actions/workflows/security.yml/badge.svg" alt="Security" /></a>
  <a href="https://github.com/jordan-temim/cctide/actions/workflows/lint.yml"><img src="https://github.com/jordan-temim/cctide/actions/workflows/lint.yml/badge.svg" alt="Lint" /></a>
  <a href="https://github.com/jordan-temim/cctide/releases/latest"><img src="https://img.shields.io/github/v/release/jordan-temim/cctide" alt="Latest release" /></a>
  <a href="https://github.com/jordan-temim/cctide/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="License" /></a>
  <a href="https://github.com/jordan-temim/cctide/releases"><img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey" alt="Platform" /></a>
  <a href="https://tauri.app"><img src="https://img.shields.io/badge/built%20with-Tauri%20v2-24C8DB" alt="Built with Tauri v2" /></a>
  <a href="#no-credentials-ever"><img src="https://img.shields.io/badge/telemetry-none-brightgreen" alt="No telemetry" /></a>
</p>

**Local Claude Code usage gauge — menu bar on macOS, system tray on Windows.**

Track your session and weekly quota, open sessions' context windows, project memory, and RTK savings.

**Your usage data never leaves your machine — no Anthropic API, no telemetry, no analytics.**
**No credentials required — no API key, no session cookie, no keychain access.**

All data is read directly from `~/.claude`. The only network request cctide makes
is the update check to GitHub (see [Updates](#updates)) — no usage data is sent with it.

<p align="center">
  <img src="docs/overview.png" width="380" />
</p>

---

## What it shows

The panel is organized into **four tabs**:

| Tab | Sections | What it tracks |
| --- | --- | --- |
| **Usage** | Session (5h), Weekly limit, Open sessions | Rolling 5-hour quota, cumulative weekly usage, each active Claude Code process' context fill (X / 200k) |
| **Settings** | Calibrate, System notifications | Anchor the bars to `/usage` once; configure three alert levels — drive segment colours, tray icon, and OS notifications |
| **Analytics** | Weekly window, Memory | Per-model token breakdown for the current week; active sessions' project memory files |
| **Extras** | RTK | Tokens saved (shown only if `rtk` is installed) |

The session and weekly bars are **15-segment fuel gauges**. The tray icon is live: two C-shapes fill with session (left) and weekly (right) usage. On macOS they blink at an escalating rate as levels are crossed. On Windows each C is tinted green → orange → red. When an update is available a **"U"** appears in the right C (see [Updates](#updates)); development builds draw a **"D"** in the left C.

Every 60 seconds (configurable via `refresh_secs`), cctide re-reads the local JSONL files and a small notch briefly sweeps both C arcs — a visual confirmation that the data just refreshed. The same sweep also plays when you save a calibration, toggle tracking, or update alert levels. The notch is a transparent gap on macOS and a grey dip on Windows; it completes in about 2 seconds and has no effect on the displayed values.

---

## Installation

> Builds are **unsigned** (no code-signing certificate). The OS will warn you on first launch — this is expected.

### Download a pre-built release

Pre-built binaries are available on the [**Releases page**](https://github.com/jordan-temim/cctide/releases/latest):

| Platform | File |
|---|---|
| macOS (Intel + Apple Silicon) | `cctide-*-universal.dmg` |
| Windows 10 / 11 | `cctide-*.msi` |

Skip to the platform section below for first-launch instructions.

### Build from source

#### macOS — universal build (Intel + Apple Silicon)

Prerequisites: Xcode Command Line Tools, Rust with both Apple targets.

```sh
xcode-select --install
rustup target add aarch64-apple-darwin x86_64-apple-darwin
npm install
npm run build:mac
```

Output: `build/cctide-*-universal.dmg`

1. Open the `.dmg`, drag **cctide** into `/Applications`.
2. First launch — Gatekeeper will block the app (unsigned build). Two ways to allow it:
   - **System Settings → Privacy & Security → Security** (scroll down) → **Open Anyway**
   - **Terminal:**
     ```sh
     xattr -dr com.apple.quarantine /Applications/cctide.app
     ```
3. The icon appears in the **menu bar** (top right). Click to open.

#### Windows 10 / 11

Prerequisites: [Build Tools for Visual Studio](https://visualstudio.microsoft.com/visual-cpp-build-tools/) (MSVC toolchain), [Node.js](https://nodejs.org), [Rust](https://rustup.rs).

```powershell
npm install
npm run build:win
```

Output: `build\cctide-*.msi`

1. Run the installer. SmartScreen may show "unknown publisher" → **More info** → **Run anyway**.
2. The icon appears in the **system tray** (bottom right). Click to open.

---

## Updates

cctide updates itself — **you stay in control of when**. It checks for a new
version at launch and every couple of hours while running. When one is available:

- a small **"U"** appears in the tray icon, and
- a banner shows at the top of the panel: **Update available: vX.Y.Z**, with a
  **What's new** link to the release notes.

Click **Install** to download it, then **Restart now** to apply. Nothing is
installed or restarted without your click.

> Auto-update works from the first release that shipped it onward. If you're on an
> older build, grab the latest `.dmg`/`.msi` from the
> [Releases page](https://github.com/jordan-temim/cctide/releases/latest) once.

---

## Uninstall

### macOS

Drag **cctide** from `/Applications` to the Trash — that removes the app.

To also remove the config file (calibration anchors, alert levels):

```sh
rm -rf ~/Library/Application\ Support/com.cctide
```

To remove the notification entry, go to **System Settings → Notifications**, find **cctide**, and delete it.

### Windows

Go to **Settings → Apps**, search for **cctide**, and click **Uninstall**. The MSI uninstaller removes the app and its registry entries automatically.

To also remove the config file:

```
%APPDATA%\com.cctide\
```

Delete that folder manually in Explorer or with:

```powershell
Remove-Item -Recurse -Force "$env:APPDATA\com.cctide"
```

---

## First run — calibrate the bars

<p align="center"><img src="docs/calibration.png" width="287" /></p>

cctide reconstructs your quota locally from token weights. A single calibration point is all it needs.

1. In Claude Code, run `/usage` and note your **session %**, **weekly %**, and **weekly reset date**.
2. In cctide, open **Calibrate** (● means pending), enter those values, and click **Save**. The indicator turns ✓ and cctide starts tracking.

> **Reset date tip:** enter the date exactly as shown by `/usage` — past or future, any format accepted (`YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`). cctide treats it as a recurring weekly anchor: once saved, the field shows your **next upcoming reset** (it rolls forward 7 days at a time), so you never see a stale past date.

After that, no further action is needed. If you change plans, recalibrate once.

### Plan-agnostic design

You don't tell cctide which plan you're on (Pro / Max 5× / Max 20×), and it never stores one.

The budget is derived from the % you report:

```
budget = tokens_so_far / (your_% / 100)
```

This automatically captures your plan's actual quota size — Pro users get a smaller budget, Max users a larger one, from the exact same calibration step. Per-model quota weights and the window mechanics (rolling 5h session, weekly reset) are identical across plans. If you switch plans, recalibrate once.

### How consumption is weighted

Tokens are weighted using **empirical quota weights** rather than API prices, then turned into a percentage by calibration. The weights live in `models.json` at the app root (compiled into the binary; nothing is written to `~/.claude`). Only the ratios matter — calibration absorbs the absolute scale — so you can edit them and rebuild if the quota mechanics seem to have changed.

These weights come from a **regression experiment**, and it's still evolving: across many sessions, each data point pairs the `/usage` % with the local token counts of that 5-hour window, and a **non-negative least-squares** (NNLS) fit looks for the weights that best reproduce the %, scored by **leave-one-window-out cross-validation** (LOWO-RMSE — hold out a whole session and predict it). The exact quota formula isn't public, so these remain **best-effort estimates** that may be refined as more data comes in.

### Context window

The "Open sessions" panel shows each active Claude Code process and how full its context window is. Some models can technically accept more than 200k tokens, but past roughly that point answer quality tends to degrade and each turn becomes far more token-hungry — so Claude Code works against an effective **~200k-token context** (compacting around there). cctide measures against that same 200k, so the percentage aligns with what `/context` shows in Claude Code.

---

## Alert levels & notifications

<p align="center"><img src="docs/system_notification.png" width="346" /></p>

Three global alert levels (default **33 / 66 / 90%**, editable in the **macOS / Windows notifications** section) drive everything at once:

- **Segment colours** — neutral → green → orange → red as usage crosses each level
- **Tray icon** — macOS blinks at an escalating cadence until you open the panel; Windows tints each C
- **OS notifications** — one notification per level crossing per bar, re-armed when the bar drops back below

The icon reacts independently of the notifications toggle.

---

## No credentials, ever

**cctide never queries Anthropic and never sends your data anywhere.** Most Claude usage trackers need a credential to query the API on your behalf — a browser session cookie extracted from DevTools, a session key stored in the system Keychain, or an OAuth token read from disk. That credential can expire, be revoked, or be accidentally exposed.

cctide takes a different approach: it reads the JSONL transcripts that Claude Code writes locally to `~/.claude`, and computes everything in-process. No cookie, no token, no Keychain entry, no Anthropic API call — there is nothing to leak or rotate. (The one exception is the update check to GitHub, which carries no usage data.)

The trade-off: because cctide never queries Anthropic's servers, the session and weekly percentages require a one-time manual calibration from `/usage`. After that, they track automatically.

---

## Privacy

cctide reads `~/.claude` files on disk and renders everything in-process. No usage data is sent anywhere and no analytics are collected. The only outbound request is the periodic update check to GitHub (to fetch `latest.json` and, if you choose to install, the new build) — it carries none of your data.

---

## Development

### Architecture

cctide is built with **[Tauri v2](https://tauri.app)**: a Rust backend embedded in a native OS window, with a lightweight web frontend (Vite + vanilla TypeScript). There is no server, no runtime dependency, and no Electron-style bundled browser — the OS webview renders the UI.

**Backend (Rust — `src-tauri/src/`):**

- Reads and parses `~/.claude` files directly on disk (JSONL transcripts, session files, memory files)
- Computes usage windows, context fill, and model totals in-process
- Exposes results to the frontend via typed Tauri commands (`invoke`)
- Runs a background ticker thread (every 60 s by default) via `do_tick()` for live tray icon updates and threshold notifications; mutations trigger an immediate extra `do_tick` for instant feedback
- Persists app config (calibration anchors, alert levels, settings) to the OS config dir via `config.rs`

**Frontend (TypeScript — `src/`):**

- Organized into separate tab modules: `tab-usage`, `tab-settings`, `tab-analytics`, `tab-extras`
- Listens to `refresh` events emitted by the backend ticker; uses `invoke` only for mutations and lazy section loads (memory)
- Renders the segmented gauge bars, open session context bars, and model breakdown
- No framework — vanilla TypeScript with direct DOM manipulation and tab routing

**Model data (`models.json`):**

- Compiled into the binary at build time via `include_str!`
- Two weight sets per model: **$/MTok prices** (reference only) and **empirical quota weights** (the `quota` block — what actually drives the %), plus context window
- Edit and rebuild to update the quota mechanics, prices, or add new models

### Running tests

```sh
# Rust unit tests (usage math, model lookup, scan filtering, config validation)
cargo test --manifest-path src-tauri/Cargo.toml

# TypeScript typecheck
npx tsc --noEmit

# Frontend unit tests (Vitest)
npm test
```

The Rust test suite covers the core business logic: session/weekly window calculation, calibration math, model entry lookup (longest-match), quota weighting, JSONL dedup filtering, and config sanitisation. The frontend suite (Vitest) covers pure helpers such as the weekly-reset rollover (`nextWeeklyReset`).

### Running locally

```sh
npm install
npm run tauri dev      # hot-reload dev build
```

### Commit convention

This project follows [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<optional scope>): <description>
```

| Type | When |
|---|---|
| `feat` | New user-facing feature |
| `fix` | Bug fix |
| `ci` | CI/CD workflow changes |
| `docs` | Documentation only |
| `refactor` | Code change with no behaviour change |
| `test` | Adding or updating tests |
| `chore` | Maintenance with no dedicated type |
| `style` | Formatting only (rustfmt, etc.) |
| `build` | Build system / dependencies |
| `perf` | Performance improvement |

Examples: `feat: add weekly models breakdown` · `fix(scan): dedupe by message id` · `ci: bump actions to node 24`

### Project layout

```
src/                  Frontend (Vite + vanilla TypeScript)
  main.ts             App entry point + tab routing
  tab-usage.ts        Usage tab (session/weekly bars + open sessions)
  tab-settings.ts     Settings tab (calibration + notifications)
  tab-analytics.ts    Analytics tab (chart + memory)
  tab-extras.ts       Extras tab (RTK)
  types.ts            Shared TypeScript types
  update.ts           Update logic
  utils.ts            DOM helpers
src-tauri/src/
  lib.rs              Tauri plugins, tray, window, module wiring
  commands.rs         Tauri command handlers
  state.rs            Shared app state
  tick.rs             Background ticker (refresh loop)
  update_svc.rs       Update check/install/restart
  scan.rs             JSONL discovery, parsing, mtime cache + dedup
  usage.rs            5h window + weekly calibration math
  context.rs          Per-session context window fill
  models.rs           Per-model data loader (models.json)
  notify.rs           Threshold-crossing native notifications
  icon.rs             Live CC-gauge tray icon renderer
  config.rs           Persisted config (calibration, settings)
  memory.rs           Memory file reader
  rtk.rs              RTK integration (optional)
models.json           Per-model quota weights + prices + context window (edit to update)
```

---

## Disclaimer

See [DISCLAIMER.md](DISCLAIMER.md).

---

## License

MIT — see [LICENSE](LICENSE).
