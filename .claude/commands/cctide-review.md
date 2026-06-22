Expert code review of the current diff. The reviewer has deep expertise in Rust, TypeScript, and Tauri v2.

Run `/code-review high` as the base review pass, then go further on the angles below that a generic reviewer would miss.

## Architecture — layered structure (check first)

The codebase follows a strict layered architecture. Verify the dependency rule holds in the diff:

```
lib.rs (wiring)
  └─ commands.rs · tick.rs · update_svc.rs  (application layer)
       └─ state.rs                           (shared state)
            └─ scan · usage · config · context · memory · rtk · icon · notify · models  (domain)
```

- **Domain modules** (`scan.rs`, `usage.rs`, `config.rs`, `context.rs`, `memory.rs`, `rtk.rs`, `icon.rs`, `notify.rs`, `models.rs`) must not import from `commands`, `tick`, `update_svc`, `state`, or `lib`. Pure logic, no Tauri deps.
- **`state.rs`** holds `AppState` + `UpdateInfo` + stateless helpers (`now_ts`, `refresh_*`). Must not contain `#[tauri::command]` or business logic.
- **`commands.rs`** is a thin IPC layer: orchestrates domain calls, no business logic. Each command should read state, call domain fn, return result.
- **`tick.rs`** owns exactly one responsibility: the ticker loop (`start_ticker`) and one recompute cycle (`do_tick`). Nothing else.
- **`update_svc.rs`** owns the full update flow: check, atomics, install, restart. The three `AtomicBool`s live here and nowhere else.
- **`lib.rs`** is wiring only: Tauri builder, plugins, tray, window, calls `tick::start_ticker` and `update_svc::spawn_update_check`. No commands, no business logic.

**Frontend (TypeScript)**

```
main.ts (composition root)
  └─ tab-usage · tab-settings · tab-analytics · tab-extras · update  (feature modules)
       └─ utils · types                                               (shared)
```

- Each `tab-*.ts` is scoped to its tab: no cross-tab imports, no DOM ids from another tab.
- `main.ts` is the only orchestrator: it calls `refresh()`, wires `setupTabs`, `setupCollapse`, `setupAutoResize`, `setupTracking`.
- `update.ts` holds module-level state (`currentUpdate`, `updateStaged`) — verify they remain module-local, not re-exported.
- `utils.ts` must stay pure (no Tauri imports, no DOM side effects at module level).

## Rust backend — domain-level checks

- **`scan.rs`**: deduplication correctness — is the `message.id + requestId` key stable across resume/sidechain scenarios? Any JSONL line that could be silently skipped? Watch for `last_ctx` being updated from records with `ts == 0` (before the timestamp guard).
- **`usage.rs`**: two-point calibration math (`percent_from`) — does the linear fit hold at boundary values (0%, 100%, one point = the other)? Single-point fallback has no upper clamp (inconsistent with two-point's `clamp(0.0, 200.0)`).
- **`usage.rs`**: `week_window_from_reset` — uses `.earliest()` (correct for DST fall-back); the `+ WEEK_SECS` stepping may drift ±1 h at DST transitions — acceptable?
- **`config.rs`**: atomic save (write tmp → rename) — `path.with_extension("json.tmp")` is same-dir, so same filesystem. But `set_calibration` releases `config_cache` before `save()` — concurrent calibration saves race on the tmp file.
- **`icon.rs`**: fill values are clamped in `render()` before use. Update "U" glyph is drawn before arc sampling — unaffected by shimmer/blink.
- **`tick.rs`**: lock ordering across `do_tick` — `config_cache` cloned first (released), then `cache`+`system` held simultaneously; `notify_state` and `rtk_cache` acquired separately after. No deadlock if this ordering is consistent.
- **`update_svc.rs`**: `UPDATE_CHECKING` cleared at thread level (outside `block_on`), so early `return` inside the async block does not prevent the clear. Panic in the thread would leave it permanently `true`. `spawn_update_check` and `install_update` are mutually exclusive via `compare_exchange`. `available_update` is released before `notification.show()`. "Found" notification fires once per version via the `is_new` flag.

## TypeScript frontend — UI-level checks

- `$()` throws on missing ID — verify every ID passed to `$()` exists in the HTML for all code paths (including `import.meta.env.DEV`-only branches).
- `setSegmentedBar`: `Math.ceil(pct / (100 / SEGMENTS))` — at pct=0 → 0 segments; at pct=100 → floating-point gives ~15.000…002 → `Math.ceil` = 16 → `Math.min(15, 16)` = 15 ✓; verify pct > 100 is capped.
- Event listeners wired once in `DOMContentLoaded` (not per refresh) — no accumulation on panel open/close.
- `renderUpdateBanner`: once `updateStaged = true`, the banner text and button state must not be overwritten by subsequent refreshes.

## Data integrity

- No code path must write to `~/.claude` — only reads (`scan.rs`, `context.rs`, `memory.rs`). `config.rs` writes to `<os-config-dir>/com.cctide/`, never to `~/.claude`.
- `std::fs::rename` on macOS is atomic within the same volume ✓.

## Output format

For each finding, output a short inline note during the review pass:
`file:line — severity (H/M/L) — description — suggested fix`

Then print a **summary table** at the end:

| # | File:Line | Severity | Finding | Suggested fix |
|---|---|---|---|---|
| 1 | scan.rs:42 | H | description | fix |
| 2 | main.ts:17 | M | description | fix |
| … | | | | |

Rows sorted by severity (H → M → L). Omit the table if there are zero findings.

End with one sentence: either "No issues found — good to merge." or "N finding(s) (X High, Y Medium, Z Low) — address before merging."
