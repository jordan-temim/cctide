Bump the project version to the argument provided (e.g. `/version 0.3.0`).

1. Update `"version"` in `package.json`
2. Update `version = "..."` in `src-tauri/Cargo.toml`
3. Update `"version"` in `src-tauri/tauri.conf.json`
4. Update the `cctide` package `version` in `src-tauri/Cargo.lock` (the
   `[[package]] name = "cctide"` entry) so the lockfile stays in sync — otherwise
   it drifts behind the manifest. Prefer running
   `cargo update -p cctide --manifest-path src-tauri/Cargo.toml` (regenerates
   just that entry); if that's unavailable, edit the `version` line under the
   `cctide` package entry by hand.

Then print the git commands to commit, tag, and push — but do NOT run them:

```
git add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json src-tauri/Cargo.lock
git commit -m "chore: bump version to v<NEW_VERSION>"
git tag v<NEW_VERSION>
git push origin main --tags
```

If no version argument is given, ask the user for the target version before doing anything.
