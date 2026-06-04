# cctide

<p align="center">
  <img src="docs/app-icon-master.png" width="96" />
</p>

<p align="center">
  <a href="https://github.com/jordan-temim/cctide/actions/workflows/security.yml"><img src="https://github.com/jordan-temim/cctide/actions/workflows/security.yml/badge.svg" alt="Security" /></a>
  <a href="https://github.com/jordan-temim/cctide/actions/workflows/lint.yml"><img src="https://github.com/jordan-temim/cctide/actions/workflows/lint.yml/badge.svg" alt="Lint" /></a>
  <img src="https://img.shields.io/badge/version-0.1.0-blue" alt="Version" />
  <a href="https://github.com/jordan-temim/cctide/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="License" /></a>
  <a href="https://github.com/jordan-temim/cctide/releases"><img src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows-lightgrey" alt="Platform" /></a>
  <a href="https://tauri.app"><img src="https://img.shields.io/badge/built%20with-Tauri%20v2-24C8DB" alt="Built with Tauri v2" /></a>
  <a href="#no-credentials-ever"><img src="https://img.shields.io/badge/network-100%25%20local-brightgreen" alt="No network" /></a>
</p>

**Local Claude Code usage gauge — menu bar on macOS, system tray on Windows.**

Track your session and weekly quota, open sessions' context windows, project memory, and RTK savings.

**No API call is ever made — not to Anthropic, not to any external service.**
**No credentials required — no API key, no session cookie, no keychain access.**

All data is read directly from `~/.claude`.

<p align="center">
  <img src="docs/overview.png" width="380" />
</p>

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

Every 30 seconds, when cctide re-reads the local JSONL files, a small notch briefly sweeps both C arcs — a visual confirmation that the data just refreshed. The notch is a transparent gap on macOS and a grey dip on Windows; it completes in about 2 seconds and has no effect on the displayed values.

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

Output: `build/cctide-*-universal.dmg`

1. Open the `.dmg`, drag **cctide** into `/Applications`.
2. First launch: right-click → **Open**, or run once:
   ```sh
   xattr -dr com.apple.quarantine /Applications/cctide.app
   ```
3. The icon appears in the **menu bar** (top right). Click to open.

### Windows 10 / 11

Prerequisites: [Build Tools for Visual Studio](https://visualstudio.microsoft.com/visual-cpp-build-tools/) (MSVC toolchain), [Node.js](https://nodejs.org), [Rust](https://rustup.rs).

```powershell
npm install
npm run build:win
```

Output: `build\cctide-*.msi`

1. Run the installer. SmartScreen may show "unknown publisher" → **More info** → **Run anyway**.
2. The icon appears in the **system tray** (bottom right). Click to open.

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

cctide reconstructs your quota locally from token weights. Two calibration points per bar are required for best accuracy — they let cctide fit a line through your actual usage rather than assuming a fixed origin.

**Step 1 — first calibration:**
1. In Claude Code, run `/usage` and note your **session %**, **weekly %**, and **weekly reset date**.
2. In cctide, open **Calibrate** (● means pending), enter those values, and click **Save**. The label reads "First calibration" and cctide starts tracking.

> **Reset date tip:** enter the date exactly as shown by `/usage` — past or future, any format accepted (`YYYY-MM-DD` or `YYYY-MM-DDTHH:MM`). If the date is in the future (e.g. your next upcoming reset), cctide automatically computes the current window by stepping back in 7-day increments.

**Step 2 — second calibration (triggered by cctide):**

3. When enough usage has accumulated (≈ 25 percentage-points later), cctide sends a system notification: *"Calibrate one final time for better accuracy."*
4. Run `/usage` again in Claude Code, enter the new percentages in cctide, and click **Save**. The indicator turns ✓ and the two-point linear fit activates.

After that, no further action is needed. If you change plans, start over from step 1.

### Plan-agnostic design

You don't tell cctide which plan you're on (Pro / Max 5× / Max 20×), and it never stores one.

The budget is derived from the % you report:

```
budget = tokens_so_far / (your_% / 100)
```

This automatically captures your plan's actual quota size — Pro users get a smaller budget, Max users a larger one, from the exact same calibration steps. Per-model pricing ratios and the window mechanics (rolling 5h session, weekly reset) are identical across plans. If you switch plans, start the two-step calibration over.

### How consumption is weighted

Tokens are weighted by Anthropic's published per-model pricing (input / output / cache-write rates), captured **2026-05-30**. **Cache reads are excluded** — Anthropic's own rate-limit metering doesn't count them, and including them caused usage to balloon with conversation length.

The weights live in `models.json` at the app root (compiled into the binary; nothing is written to `~/.claude`). Edit it and rebuild if pricing changes. Only the ratios matter — calibration absorbs the absolute scale.

> **Why two points?** A single-point calibration assumes the relationship between local tokens and Anthropic's metering passes through zero. In practice there is a small constant offset. The two-point fit (`percent = a·tokens + b`) corrects for both scale error and offset, keeping the displayed % within ~1–2% of `/usage` after calibration.

### Context window

The "Open sessions" panel shows each active Claude Code process and how full its context window is. Claude Code uses an effective **200k-token context** for all current models, regardless of a model's theoretical maximum. cctide uses that same 200k as its denominator, so the percentage aligns with what `/context` shows in Claude Code.

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

**cctide makes zero network requests.** Most Claude usage trackers need a credential to query the API on your behalf — a browser session cookie extracted from DevTools, a session key stored in the system Keychain, or an OAuth token read from disk. That credential can expire, be revoked, or be accidentally exposed.

cctide takes a different approach: it reads the JSONL transcripts that Claude Code writes locally to `~/.claude`, and computes everything in-process. No cookie, no token, no Keychain entry, no API call — there is nothing to leak or rotate.

The trade-off: because cctide never queries Anthropic's servers, the session and weekly percentages require a one-time manual calibration from `/usage`. After that, they track automatically.

---

## Privacy

cctide is **100% local**. It reads `~/.claude` files on disk and renders everything in-process. No data is sent anywhere, no network request is ever made, no analytics are collected.

---

## Development

### Architecture

cctide is built with **[Tauri v2](https://tauri.app)**: a Rust backend embedded in a native OS window, with a lightweight web frontend (Vite + vanilla TypeScript). There is no server, no runtime dependency, and no Electron-style bundled browser — the OS webview renders the UI.

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
