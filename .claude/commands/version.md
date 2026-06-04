Bump the project version to the argument provided (e.g. `/version 0.3.0`).

1. Update `"version"` in `package.json`
2. Update `version = "..."` in `src-tauri/Cargo.toml`
3. Update `"version"` in `src-tauri/tauri.conf.json`

Then print the git commands to commit, tag, and push — but do NOT run them:

```
git add package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json
git commit -m "chore: bump version to v<NEW_VERSION>"
git tag v<NEW_VERSION>
git push origin main --tags
```

If no version argument is given, ask the user for the target version before doing anything.
