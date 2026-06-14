Security audit of the codebase. Run `/security-review` as the base pass, then apply the threat model below.

## Threat model

cctide is a local desktop app. It:
- reads `~/.claude` (JSONL transcripts, session files, memory markdown)
- writes one config file to the OS config dir
- renders a web UI in a Tauri webview
- sends no usage data anywhere; its **only** outbound request is the updater's
  check/download to GitHub (`latest.json` + the signed bundle)

Attacker surfaces:
1. **Malicious JSONL content** — a rogue Claude Code session could write crafted JSONL to `~/.claude`
2. **Path traversal** — filenames or paths read from JSONL/session files used in filesystem ops
3. **XSS via memory files** — memory `.md` content rendered in the webview
4. **Config tampering** — an attacker with local access modifies `cctide.json`
5. **Dependency vulnerabilities** — Rust crates and npm packages
6. **Malicious update** — a compromised GitHub release or MITM serves a rogue
   bundle / `latest.json`

## Audit checklist

**Input validation**
- [ ] JSONL parsing: is `serde_json` used safely? Any `unwrap()` on untrusted fields?
- [ ] File paths from `~/.claude`: are they validated/confined before being opened?
- [ ] Memory file content: is it sanitized before being sent to the frontend and rendered?
- [ ] Config file: what happens if `cctide.json` contains unexpected/extreme values?

**Tauri security**
- [ ] CSP in `tauri.conf.json`: is it restrictive enough?
- [ ] `allowlist` / capabilities: are Tauri commands scoped correctly?
- [ ] `openPath` usage: can an attacker trigger arbitrary path opens via crafted content?
- [ ] `shell` permissions: is shell execution gated?

**Filesystem**
- [ ] Does any code path write outside the config dir or `~/.claude`? (must be read-only on `~/.claude`)
- [ ] Symlink following: could a symlink in `~/.claude` redirect reads outside the intended tree?
- [ ] Tmp file for atomic save: is it on the same volume? Are permissions correct?

**Auto-update**
- [ ] Update signature verification: is `plugins.updater.pubkey` set in `tauri.conf.json` (so a compromised release / MITM can't push an unsigned bundle)?
- [ ] `latest.json` `version` is interpolated into the changelog URL — is it sanitized before being passed to `openUrl` (no scheme/host injection)?
- [ ] `update.body` (release notes from an unsigned `latest.json`): if ever rendered in the panel, is it escaped (no `innerHTML`)?
- [ ] `install_update` / `restart_app` commands: harmless if invoked from a hypothetical XSS (install is signature-gated, restart is benign)?
- [ ] Privacy claims (README/CLAUDE.md) stay accurate: the updater contacts GitHub, so "zero network requests" must be scoped to *usage data / telemetry*.

**Dependencies**
Run the following and report findings:
```sh
cd src-tauri && cargo audit
npm audit --audit-level=moderate
# --config is required: deny.toml lives at the repo root, not in src-tauri/
cargo deny --manifest-path src-tauri/Cargo.toml check --config deny.toml licenses sources bans
```

**Secrets / data leakage**
- [ ] No API keys, tokens, or credentials in code or config
- [ ] No telemetry or outbound connections (verify with `gitleaks detect --source . --redact`)
- [ ] Memory file content: not logged or persisted beyond the session panel

## Output format

For each finding, output a short inline note during the audit pass:
`file:line — severity (Critical/High/Medium/Low) — description — remediation`

Then print a **checklist summary table** covering every audit area:

| Area | Check | Status | Notes |
|---|---|---|---|
| Input validation | JSONL parsing (`serde_json` / `unwrap`) | ✅ / ❌ | file:line if issue |
| Input validation | Path traversal (paths from `~/.claude`) | ✅ / ❌ | |
| Input validation | Memory file XSS sanitization | ✅ / ❌ | |
| Input validation | Config extreme/unexpected values | ✅ / ❌ | |
| Tauri | CSP in tauri.conf.json | ✅ / ❌ | |
| Tauri | Capabilities / allowlist scope | ✅ / ❌ | |
| Tauri | `openPath` / shell permissions | ✅ / ❌ | |
| Filesystem | No writes to `~/.claude` | ✅ / ❌ | |
| Filesystem | Symlink following risk | ✅ / ❌ | |
| Filesystem | Atomic save tmp-file volume + permissions | ✅ / ❌ | |
| Auto-update | Update signature verification (pubkey set) | ✅ / ❌ | |
| Auto-update | `latest.json` version sanitized before `openUrl` | ✅ / ❌ | |
| Auto-update | Privacy claims scoped to usage data (not "zero network") | ✅ / ❌ | |
| Dependencies | cargo audit | ✅ (N warns) / ❌ | expected GTK3 warns OK |
| Dependencies | npm audit | ✅ / ❌ | |
| Dependencies | cargo deny | ✅ / ❌ / ⚠️ not installed | |
| Secrets | gitleaks (no keys / outbound) | ✅ / ❌ / ⚠️ not installed | |
| Secrets | Memory content not logged/persisted | ✅ / ❌ | |

Then print a **findings table** (omit if no findings):

| # | File:Line | Severity | Finding | Remediation |
|---|---|---|---|---|
| 1 | scan.rs:42 | High | description | fix |
| … | | | | |

Rows sorted by severity (Critical → High → Medium → Low).

End with: **Overall risk: Low / Medium / High** and the top 3 action items (or "No action items." if clean).
