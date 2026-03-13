# Release, Distribution, and Migration

## CI/CD Pipeline

### CI (`.github/workflows/ci.yml`)
- Rust: `fmt`, `clippy`, `cargo test --workspace`.
- Frontend: `npm ci` and production build.
- Goal: keep main branch releasable at all times.

### Release (`.github/workflows/release.yml`)
- Trigger: git tag `v*` (example `v0.3.1`).
- Build matrix:
  - `macos-14` -> `aarch64-apple-darwin` (DMG + APP).
  - `macos-13` -> `x86_64-apple-darwin` (DMG + APP).
  - `windows-latest` -> `x86_64-pc-windows-msvc` (NSIS + MSI).
  - `ubuntu-22.04` -> `x86_64-unknown-linux-gnu` (AppImage + DEB, optional distribution target).
- Produces updater artifacts/signatures (`createUpdaterArtifacts: true` in `tauri.conf.json`).
- Publishes all generated bundle assets to the GitHub Release for that tag.
- Must also publish pyannote provisioning assets for the same tag:
  - `pyannote-runtime-macos-aarch64.zip`
  - `pyannote-runtime-macos-x86_64.zip`
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

### macOS
- Build signed bundles in CI.
- Submit DMG to Apple Notary Service with `xcrun notarytool`.
- Staple notarization ticket with `xcrun stapler`.
- Build pyannote release assets before publishing the tag:
  - `./scripts/package_pyannote_release_assets.sh <version> <runtime_aarch64_dir> <runtime_x86_64_dir> <model_dir> <output_dir>`
  - Upload the generated zips and manifest to the same GitHub Release as the app bundles.

### Windows
- Bundle MSI + NSIS by default.
- Optionally add EV cert signing in a follow-up hardening pass (preferred for SmartScreen reputation).

## Auto-Updates

- `tauri.conf.json` enables updater artifact generation.
- Updater plugin config is scaffolded and intentionally disabled until production key material is provisioned.
- Before enabling:
  1. Generate updater keypair.
  2. Set real `pubkey` and release endpoint URL.
  3. Flip `"plugins.updater.active"` to `true`.
  4. Add frontend update UX (silent check + user-approved install).

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
8. Enable updater flow only after signed release pipeline is validated in staging.
9. Run side-by-side comparison runs between Python and Tauri app outputs for confidence.
10. Cut over when parity checklist is green and error telemetry is stable.

## Behavior-Safety Strategy

- Keep domain/application contract tests mandatory in PR checks.
- Add adapter integration tests for ffmpeg, whisper-cli, and Gemini against staging fixtures.
- Maintain a "known differences" ledger during migration to avoid accidental regressions.
- Ship internal alpha and external beta before replacing legacy Python distribution.
