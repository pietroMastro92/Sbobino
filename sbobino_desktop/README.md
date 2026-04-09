# Sbobino Desktop Rewrite (Tauri v2 + Rust Clean Architecture)

This repository is the production-grade rewrite of the original Python Sbobino app.

## Workspace Layout

- `crates/domain`: pure business model and rules
- `crates/application`: use cases and ports (interfaces)
- `crates/infrastructure`: adapters (whisper/ffmpeg/gemini/filesystem/sqlite)
- `apps/desktop`: React presentation layer
- `apps/desktop/src-tauri`: Tauri command host and runtime composition
- `docs/architecture.md`: architecture and dependency rules
- `docs/release-and-migration.md`: release pipeline and migration plan
- `docs/feature-migration-matrix.md`: feature-by-feature parity checklist

## Quick Start

1. Install Rust stable and Node 20+.
2. `cd apps/desktop`
3. `npm ci`
4. `npm run tauri:dev`

## Runtime Setup (first run)

From workspace root:

1. `./scripts/setup_runtime.sh` (downloads `ggml-base.bin` into app data models dir)
2. In app, keep `Model = Base` for first run.
3. For public local releases, run `./scripts/setup_bundled_pyannote.sh` only to hydrate the source assets that will be zipped as separate first-launch provisioning artifacts.
4. The public DMG does not bundle pyannote or the full local runtime. On first launch the app installs FFmpeg, Whisper CLI, Whisper Stream, Whisper models, and pyannote assets into app data through the guided setup flow.

## Local Release

Use `./scripts/prepare_local_release.sh <version>` from `sbobino_desktop/` to build and validate a macOS Apple Silicon candidate release locally without publishing anything to GitHub. The default `public` profile prepares a lightweight DMG, keeps pyannote out of the app bundle, signs the updater artifacts with a stable local Tauri updater keypair, and writes the full candidate folder into `dist/local-release/v<version>`.

That folder now always includes:
- `Sbobino_<version>_aarch64.dmg`
- `Sbobino.app.tar.gz`
- `Sbobino.app.tar.gz.sig`
- `latest.json`
- `setup-manifest.json`
- `runtime-manifest.json`
- `speech-runtime-macos-aarch64.zip`
- `pyannote-manifest.json`
- `pyannote-runtime-macos-aarch64.zip`
- `pyannote-model-community-1.zip`

Manual publish contract:
1. build the candidate locally
2. create a GitHub prerelease for the same `v<version>`
3. upload the full asset set manually
4. run `./scripts/distribution_readiness.sh <version>`
5. test that exact GitHub prerelease on a second Apple Silicon Mac
6. promote the same prerelease to stable only after it passes

Set `SBOBINO_RELEASE_PROFILE=standalone-dev` only for internal/offline builds that intentionally embed bundled pyannote assets.

## Current Milestone

- Initial clean architecture scaffold
- First background transcription command
- Progress/completion events to frontend
- Minimal working desktop UI
