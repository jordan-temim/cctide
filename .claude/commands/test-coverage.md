Audit the Rust test suite and add missing tests.

## Step 1 — Inventory

Read every `.rs` file under `src-tauri/src/`. For each public function or non-trivial private function, check whether a test exists in the same file's `#[cfg(test)]` block.

Focus on files that have business logic worth testing:
- `usage.rs` — session/weekly window math, `week_window_from_reset`, `percent_from` (two-point calibration)
- `scan.rs` — JSONL parsing, deduplication, `model_totals`
- `config.rs` — `level_for`, `sanitize_levels`, `parse_config`
- `models.rs` — model lookup (longest-match), fallback to default

Ignore `lib.rs`, `main.rs`, `notify.rs` — wiring/IPC/threading code without
testable pure logic (the auto-update flow there is integration-only: it needs the
live Tauri updater + network, so it's not unit-testable without heavy mocking).

`icon.rs` is rendering code, but its pure `render()` output is worth a smoke test
— there's already one asserting the update "U" adds opaque pixels. Add similar
pixel-level assertions only when render behaviour changes; don't chase coverage on
the geometry math.

## Step 2 — Gap report

List the missing tests as a table:

| File | Function | What's untested | Priority |
|------|----------|-----------------|----------|
| ...  | ...      | ...             | H/M/L    |

Priority:
- **H** = correctness-critical (wrong output silently breaks the UI)
- **M** = edge case that could cause subtle drift
- **L** = defensive / nice-to-have

## Step 3 — Write the tests

Add the missing H and M priority tests directly to the appropriate `#[cfg(test)]` blocks in the source files.

Rules:
- Follow the existing style in each file (helper functions like `pt()`, `cal()` are already defined — reuse them)
- One `#[test]` per scenario, descriptive name
- No external crates — use only `std` and what's already imported
- Run `cargo test --manifest-path src-tauri/Cargo.toml` after adding tests and fix any failures before reporting done

## Step 4 — Report

Show the final test count before/after and list which tests were added.
