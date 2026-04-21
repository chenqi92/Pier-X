# pier-ui-tauri

Active desktop shell for Pier-X, built with Tauri 2 + React + TypeScript.

## Commands

```bash
npm ci
npm run tauri dev              # dev: vite + tauri dev
npm run tauri build            # release bundles
npm run tauri build -- --debug # debug bundles
```

### Release management

```bash
npm run bump <version>         # sync version across package.json, tauri.conf.json,
                               # pier-ui-tauri/src-tauri/Cargo.toml, pier-core/Cargo.toml;
                               # commit and create a v<version> git tag
npm run bump patch             # or minor / major — auto-increment
npm run bump <version> --dry-run  # preview without writing
```

Pushing the resulting `v<version>` tag triggers:

- `.github/workflows/release.yml` — builds and publishes Linux, Windows x64,
  Windows ARM64, and macOS universal bundles to GitHub Releases.
- `.gitea/workflows/release.yml` — builds `.deb` / `.rpm` / `.AppImage` on an
  `ubuntu-22.04` Gitea runner and uploads to the Gitea Release via API.
