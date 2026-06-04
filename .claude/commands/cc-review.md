Expert code review of the current diff. The reviewer has deep expertise in Rust, TypeScript, and Tauri v2.

Run `/code-review high` as the base review pass, then go further on the angles below that a generic reviewer would miss.

## cctide-specific angles

**Rust backend**
- `scan.rs`: deduplication correctness — is the `message.id + requestId` key stable across resume/sidechain scenarios? Any JSONL line that could be silently skipped?
- `usage.rs`: two-point calibration math (`percent_from`) — does the linear fit hold at boundary values (0%, 100%, one point = the other)?
- `usage.rs`: `week_window_from_reset` — DST edge cases (`.single()` returns None on ambiguous local times), timezone correctness
- `config.rs`: atomic save (write tmp → rename) — is the tmp path on the same filesystem as the target? Cross-device rename fails silently on some setups
- `icon.rs`: render correctness — fill values outside [0, 1] passed to render; any clamping?
- `lib.rs`: icon thread — lock ordering (config_cache → cache → icon_state), any potential for deadlock?

**TypeScript frontend**
- `$()` helper casts `getElementById` result without null check — any ID that could be missing from the DOM?
- `setSegmentedBar`: `Math.ceil(pct / (100 / SEGMENTS))` — behaviour at pct = 0, pct = 100, pct > 100?
- Event listeners and `setInterval` — any leak if the panel is opened/closed repeatedly?

**Data integrity**
- Does any code path write to `~/.claude`? (must never happen)
- Is the config save atomic on Windows (rename semantics differ)?

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
