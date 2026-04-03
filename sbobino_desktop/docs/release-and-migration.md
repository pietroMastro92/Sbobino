# Release, Distribution, and Migration

## CI/CD Pipeline

### CI (`.github/workflows/ci.yml`)
- Rust: `fmt`, `clippy`, `cargo test --workspace`.
- Frontend: `npm ci` and production build.
- Goal: keep main branch releasable at all times.

### Release (`.github/workflows/release.yml`)
- Trigger: git tag `v*` (example `v0.3.1`).
- First public release target:
  - `macos-14` -> `aarch64-apple-darwin` (DMG + APP).
- Produces updater artifacts/signatures (`createUpdaterArtifacts: true` in `tauri.conf.json`).
- Publishes all generated bundle assets to the GitHub Release for that tag.
- Must also publish pyannote provisioning assets for the same tag:
  - `pyannote-runtime-macos-aarch64.zip`
  - `pyannote-model-community-1.zip`
  - `pyannote-manifest.json`

## Signing and Notarization

### Required Secrets
- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
- `APPLE_CERTIFICATE`
- `APPLE_CERTIFICATE_PASSWORD`
- `APPLE_SIGNING_IDENTITY`
- `APPLE_ID`
- `APPLE_PASSWORD`
- `APPLE_TEAM_ID`
- `TAURI_UPDATER_PUBLIC_KEY`

### macOS
- Build signed bundles in CI.
- Submit DMG to Apple Notary Service with `xcrun notarytool`.
- Staple notarization ticket with `xcrun stapler`.
- Inject the updater public key in CI before `tauri build`:
  - `./scripts/prepare_release_updater_config.sh apps/desktop/src-tauri/tauri.conf.json "$TAURI_UPDATER_PUBLIC_KEY"`
- Before `tauri build`, hydrate the bundled pyannote resources on the macOS build machine:
  - `./scripts/setup_bundled_pyannote.sh --force`
  - this populates `apps/desktop/src-tauri/resources/pyannote/model` and `apps/desktop/src-tauri/resources/pyannote/python/<target-triple>`
  - the packaged app then ships pyannote offline and auto-installs it on first launch without asking the end user to download anything
- Build pyannote release assets before publishing the tag:
  - `./scripts/package_pyannote_asset.sh <runtime_aarch64_dir> python <output_zip>`
  - `./scripts/package_pyannote_asset.sh <model_dir> model <output_zip>`
  - generate `pyannote-manifest.json` with checksums for the same GitHub Release as the app bundles.

### Windows
- Out of scope for the initial public release.

## Auto-Updates

- `tauri.conf.json` enables updater artifact generation.
- Updater plugin config is enabled and points to:
  - `https://github.com/pietroMastro92/sbobino_tauri/releases/latest/download/latest.json`
- The repository version of `tauri.conf.json` intentionally keeps a placeholder `pubkey`; CI injects the real public key during release builds.
- The frontend performs:
  1. silent update check on startup when enabled;
  2. manual "Check Updates" action in settings;
  3. in-app install when Tauri updater returns an installable update;
  4. manual GitHub download fallback if native updater is unavailable.

## Semantic Versioning Strategy

- Version policy: `MAJOR.MINOR.PATCH`.
- Release tags: `vX.Y.Z`.
- Rules:
  - `PATCH`: bugfix/internal behavior parity improvements.
  - `MINOR`: new non-breaking capabilities.
  - `MAJOR`: intentionally breaking domain/API changes.
- Keep migration notes per release to track parity with legacy Python behavior.

## Migration Plan (Feature-Safe)

1. Freeze a Python parity matrix (feature-by-feature + expected artifacts).
2. Keep the new Rust vertical slice as the baseline: ingest file -> transcribe -> persist.
3. Port each remaining feature behind explicit domain/application services, not UI-driven logic.
4. Add acceptance fixtures (audio + expected outputs) for each migrated workflow.
5. Migrate live transcription and recorder controls with a process/session manager abstraction.
6. Migrate model management (download, validation, checksum, upgrades).
7. Migrate advanced post-processing prompts and language variants.
8. Validate updater flow by shipping `v0.1.0`, then publish `v0.1.1` and verify in-app update from the notarized arm64 release.
9. Run side-by-side comparison runs between Python and Tauri app outputs for confidence.
10. Cut over when parity checklist is green and error telemetry is stable.

## Behavior-Safety Strategy

- Keep domain/application contract tests mandatory in PR checks.
- Add adapter integration tests for ffmpeg, whisper-cli, and Gemini against staging fixtures.
- Maintain a "known differences" ledger during migration to avoid accidental regressions.
- Ship internal alpha and external beta before replacing legacy Python distribution.
