# ccgauge

<p align="center">
  <img src="docs/app-icon-master.png" width="96" />
</p>

<p align="center">
  <a href="https://github.com/jordan-temim/ccgauge/actions/workflows/security.yml"><img src="https://github.com/jordan-temim/ccgauge/actions/workflows/security.yml/badge.svg" alt="Security" /></a>
  <a href="https://github.com/jordan-temim/ccgauge/actions/workflows/lint.yml"><img src="https://github.com/jordan-temim/ccgauge/actions/workflows/lint.yml/badge.svg" alt="Lint" /></a>
  <img src="https://img.shields.io/badge/version-0.1.0-blue" alt="Version" />
  <a href="https://github.com/jordan-temim/ccgauge/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="License" /></a>
  <a href="https://github.com/jordan-temim/ccgauge/releases"><img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey" alt="Platform" /></a>
  <a href="https://tauri.app"><img src="https://img.shields.io/badge/built%20with-Tauri%20v2-24C8DB" alt="Built with Tauri v2" /></a>
  <a href="#no-credentials-ever"><img src="https://img.shields.io/badge/network-100%25%20local-brightgreen" alt="No network" /></a>
</p>

**Local Claude Code usage gauge — menu bar on macOS, system tray on Windows.**

Track your session and weekly quota, open sessions' context windows, project memory, and RTK savings.

**No API call is ever made — not to Anthropic, not to any external service.**
**No credentials required — no API key, no session cookie, no keychain access.**

All data is read directly from `~/.claude`.

---

## What it shows

| Panel section                     | What it tracks                                                                                     |
| --------------------------------- | -------------------------------------------------------------------------------------------------- |
| **Session (5h)**                  | Rolling 5-hour quota consumption                                                                   |
| **Weekly limit**                  | Cumulative usage since your reset date                                                             |
| **Calibrate**                     | Anchor the bars to `/usage` once; they track on their own after                                    |
| **macOS / Windows notifications** | Toggle + three configurable alert levels — drive segment colours, tray icon, and OS notifications  |
| **Open sessions**                 | Each active Claude Code process — context fill (X / 200k)                                          |
| **Weekly models**                 | Per-model token breakdown for the current week                                                     |
| **Memory**                        | Active sessions' project memory files                                                              |
| **RTK**                           | Tokens saved (shown only if `rtk` is installed)                                                    |

The session and weekly bars are **15-segment fuel gauges**. The tray icon is live: two C-shapes fill with session (left) and weekly (right) usage. On macOS they blink at an escalating rate as levels are crossed. On Windows each C is tinted green → orange → red.

---

## Installation

> Builds are **unsigned** (no code-signing certificate). The OS will warn you on first launch — this is expected.
>
> There are no pre-built releases — build from source using the instructions below.

### macOS — universal build (Intel + Apple Silicon)

Prerequisites: Xcode Command Line Tools, Rust with both Apple targets.

```sh
xcode-select --install
rustup target add aarch64-apple-darwin x86_64-apple-darwin
npm install
npm run build:mac
```

Output: `build/ccgauge-*-universal.dmg`

1. Open the `.dmg`, drag **ccgauge** into `/Applications`.
2. First launch: right-click → **Open**, or run once:
   ```sh
   xattr -dr com.apple.quarantine /Applications/ccgauge.app
   ```
3. The icon appears in the **menu bar** (top right). Click to open.

### Windows 10 / 11

Prerequisites: [Build Tools for Visual Studio](https://visualstudio.microsoft.com/visual-cpp-build-tools/) (MSVC toolchain), [Node.js](https://nodejs.org), [Rust](https://rustup.rs).

```powershell
npm install
npm run build:win
```

Output: `build\ccgauge-*.msi`

1. Run the installer. SmartScreen may show "unknown publisher" → **More info** → **Run anyway**.
2. The icon appears in the **system tray** (bottom right). Click to open.

---

## First run — calibrate the bars

<p align="center"><img src="docs/calibration.png" width="287" /></p>

ccgauge reconstructs your quota locally from token weights, so it needs one anchor point to know what 100% means:

1. In Claude Code, run `/usage` and note your **session %**, **weekly %**, and **weekly reset date**.
2. In ccgauge, open **Calibrate** (a ● indicator means it's pending), enter those values, and click **Save**. The indicator turns ✓ when done.
3. That's it. Re-calibrate only if you change plans or notice real drift.

### Plan-agnostic design

You don't tell ccgauge which plan you're on (Pro / Max 5× / Max 20×), and it never stores one.

The budget is derived from the % you report:

```
budget = tokens_so_far / (your_% / 100)
```

This automatically captures your plan's actual quota size — Pro users get a smaller budget, Max users a larger one, from the exact same calibration step. Per-model pricing ratios and the window mechanics (rolling 5h session, weekly reset) are identical across plans. If you switch plans, re-calibrate once.

### How consumption is weighted

Tokens are weighted by Anthropic's published per-model pricing (input / output / cache-write rates), captured **2026-05-30**. **Cache reads are excluded** — Anthropic's own rate-limit metering doesn't count them, and including them caused usage to balloon with conversation length.

The weights live in `models.json` at the app root (compiled into the binary; nothing is written to `~/.claude`). Edit it and rebuild if pricing changes. Only the ratios matter — calibration absorbs the absolute scale.

> **Why the estimate is stable:** with cache reads excluded and the budget derived from your reported %, a single calibration should hold for weeks. Expect a small margin (1–5%) versus `/usage` — this is normal calibration drift. The segmented bar display absorbs that margin visually.

### Context window

The "Open sessions" panel shows each active Claude Code process and how full its context window is. Claude Code uses an effective **200k-token context** for all current models, regardless of a model's theoretical maximum. ccgauge uses that same 200k as its denominator, so the percentage aligns with what `/context` shows in Claude Code.

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

**ccgauge makes zero network requests.** Most Claude usage trackers need a credential to query the API on your behalf — a browser session cookie extracted from DevTools, a session key stored in the system Keychain, or an OAuth token read from disk. That credential can expire, be revoked, or be accidentally exposed.

ccgauge takes a different approach: it reads the JSONL transcripts that Claude Code writes locally to `~/.claude`, and computes everything in-process. No cookie, no token, no Keychain entry, no API call — there is nothing to leak or rotate.

The trade-off: because ccgauge never queries Anthropic's servers, the session and weekly percentages require a one-time manual calibration from `/usage`. After that, they track automatically.

---

## Privacy

ccgauge is **100% local**. It reads `~/.claude` files on disk and renders everything in-process. No data is sent anywhere, no network request is ever made, no analytics are collected.

---

## Development

### Architecture

ccgauge is built with **[Tauri v2](https://tauri.app)**: a Rust backend embedded in a native OS window, with a lightweight web frontend (Vite + vanilla TypeScript). There is no server, no runtime dependency, and no Electron-style bundled browser — the OS webview renders the UI.

**Backend (Rust — `src-tauri/src/`):**

- Reads and parses `~/.claude` files directly on disk (JSONL transcripts, session files, memory files)
- Computes usage windows, context fill, and model totals in-process
- Exposes results to the frontend via typed Tauri commands (`invoke`)
- Runs a background ticker thread (~400ms) for live tray icon updates and threshold notifications
- Persists app config (calibration anchors, alert levels, settings) to the OS config dir via `config.rs`

**Frontend (TypeScript — `src/`):**

- Polls the Rust backend every 30 seconds via `invoke` calls
- Renders the segmented gauge bars, open session context bars, and model breakdown
- No framework — vanilla TypeScript with direct DOM manipulation

**Model data (`models.json`):**

- Compiled into the binary at build time via `include_str!`
- Contains per-model: input/output/cache-write pricing weights, context window
- Edit and rebuild to update pricing or add new models

### Running tests

```sh
# Rust unit tests (usage math, model lookup, scan filtering, config validation)
cargo test --manifest-path src-tauri/Cargo.toml

# TypeScript typecheck
npx tsc --noEmit
```

The Rust test suite covers the core business logic: session/weekly window calculation, calibration math, model entry lookup (longest-match), quota weighting, JSONL dedup filtering, and config sanitisation.

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
src-tauri/src/
  lib.rs              Tauri commands, tray, popup window
  scan.rs             JSONL discovery, parsing, mtime cache + dedup
  usage.rs            5h window + weekly calibration math
  context.rs          Per-session context window fill
  models.rs           Per-model data loader (models.json)
  notify.rs           Threshold-crossing native notifications
  icon.rs             Live CC-gauge tray icon renderer
  config.rs           Persisted config (calibration, settings)
  memory.rs           Memory file reader
  rtk.rs              RTK integration (optional)
models.json           Per-model pricing + context window (edit to update)
```

---

## Disclaimer

See [DISCLAIMER.md](DISCLAIMER.md).

---

## License

MIT — see [LICENSE](LICENSE).
