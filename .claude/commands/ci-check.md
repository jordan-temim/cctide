# ci-check

Run all CI checks locally (mirrors .github/workflows/security.yml + lint.yml).
Auto-fix what can be fixed (fmt, clippy --fix), report the rest.

## Steps

Run from the repo root. Steps in order — stop and report clearly on first hard failure (exit ≠ 0 and not fixable).

### 1. cargo fmt
```
cd src-tauri && cargo fmt --all
```
Always auto-fixes. Verify with `cargo fmt --all -- --check`. Report if still dirty.

### 2. cargo test
```
cd src-tauri && cargo test
```
Report any failures with test name and output.

### 3. cargo clippy
```
cd src-tauri && cargo clippy --all-targets -- -D warnings
```
If it fails, attempt auto-fix:
```
cd src-tauri && cargo clippy --fix --allow-dirty --all-targets -- -D warnings
```
Then re-run the check. Report remaining errors with file:line references.

### 3. tsc
```
npx tsc --noEmit
```
Report errors with file:line references. Cannot auto-fix.

### 4. cargo audit
```
cd src-tauri && cargo audit
```
Warnings (unmaintained GTK3 Tauri transitive deps) are expected and non-blocking. Only fail on `error:` lines.

### 5. npm audit
```
npm audit --audit-level=high
```
Fail only on high/critical.

### 6. cargo deny (if installed)
```
cargo deny --manifest-path src-tauri/Cargo.toml check licenses sources bans
```
Skip with a note if `cargo deny` is not installed.

### 7. gitleaks (if installed)
```
gitleaks detect --source . --redact
```
Skip with a note if not installed. Failure = secret detected, must fix manually.

### 8. semgrep (if installed)
```
semgrep scan --config p/rust --config p/typescript --config p/javascript --error .
```
Skip with a note if not installed.

## Output format

Print a summary table at the end:

| Check | Status | Notes |
|---|---|---|
| cargo fmt | ✅ / ✅ auto-fixed / ❌ | |
| cargo test | ✅ / ❌ | test name if failures |
| cargo clippy | ✅ / ✅ auto-fixed / ❌ | file:line if errors |
| tsc | ✅ / ❌ | file:line if errors |
| cargo audit | ✅ (N warns) | expected GTK3 warns OK |
| npm audit | ✅ / ❌ | |
| cargo deny | ✅ / ❌ / ⚠️ not installed | |
| gitleaks | ✅ / ❌ / ⚠️ not installed | |
| semgrep | ✅ / ❌ / ⚠️ not installed | |

End with one sentence: either "Ready to push." or "X issues to fix before pushing."
