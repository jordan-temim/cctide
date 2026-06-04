# Security Policy

## Supported versions

| Version | Supported |
|---|---|
| Latest release | ✅ |
| Older releases | ❌ |

Only the latest release receives security fixes.

## Scope

cctide is a local-only desktop app. It:

- reads `~/.claude` (JSONL transcripts, session files, memory markdown) — **read-only**
- writes one config file to the OS config directory (`~/Library/Application Support/com.cctide/` on macOS)
- renders a web UI in a sandboxed Tauri webview
- makes **zero network requests**

No API keys, credentials, or personal data are transmitted or stored outside your machine.

## Reporting a vulnerability

If you discover a security vulnerability, please **do not open a public issue**.

Report it privately via [GitHub Security Advisories](https://github.com/jordan-temim/cctide/security/advisories/new).

Include:

- A description of the vulnerability and its potential impact
- Steps to reproduce
- Affected version(s)

You can expect an acknowledgement within 7 days and a fix or mitigation plan within 30 days depending on severity.

## Out of scope

- Vulnerabilities requiring physical access to an already-compromised machine
- Issues in dependencies not directly exploitable through cctide
- Social engineering
