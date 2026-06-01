# Disclaimer

## No affiliation with Anthropic

ccgauge is an independent personal project. It is **not affiliated with, endorsed by, sponsored by, or associated with Anthropic PBC** in any way. "Claude", "Claude Code", and related names are trademarks of Anthropic. This project has no relationship with Anthropic and is not an official product.

## Estimates, not guarantees

The session and weekly usage percentages displayed by ccgauge are **estimates reconstructed locally** from JSONL transcripts written by Claude Code to `~/.claude`. They may differ from the values shown by Anthropic's own `/usage` command. Factors that can cause drift:

- Calibration age (re-calibrate if you notice persistent divergence)
- Models or pricing changes not yet reflected in the bundled `models.json`
- Edge cases in transcript deduplication

**Do not rely on ccgauge for billing decisions.** Always verify your actual usage and billing at [claude.ai/settings/usage](https://claude.ai/settings/usage) or through Anthropic's official channels.

## No warranty

ccgauge is provided **as-is, without any warranty of any kind**, express or implied. The author makes no guarantees regarding accuracy, availability, fitness for a particular purpose, or freedom from defects. Use at your own risk.

## Local data access

ccgauge reads files from `~/.claude` on your local machine. It makes **no network requests** and never transmits any data externally. However, by running this software you accept that it accesses your local Claude Code transcript files.

## Third-party dependencies

ccgauge is built on open-source libraries including [Tauri](https://tauri.app), [Serde](https://serde.rs), [sysinfo](https://github.com/GuillaumeGomez/sysinfo), and others listed in `src-tauri/Cargo.toml`. These dependencies are governed by their own licenses and maintained by their respective authors. The author of ccgauge makes no representations regarding their security, correctness, or continued availability.

## Unsigned binary

Distributed builds are **unsigned** (no Apple Developer ID, no Microsoft Authenticode certificate). Your OS will warn you on first launch — this is expected for a tool distributed outside official app stores. Only install from a source you trust.
