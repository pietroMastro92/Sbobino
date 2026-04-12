# Release, Distribution, and Migration

## CI/CD Pipeline

### CI (`.github/workflows/ci.yml`)
- Rust: `fmt`, `clippy`, `cargo test --workspace`.
- Frontend: `npm ci` and production build.
- Goal: keep main branch releasable at all times.

### Release (`.github/workflows/release.yml`)
- Trigger: manual `workflow_dispatch` for an existing tag.
- First public release target:
  - `macos-14` -> `aarch64-apple-darwin` (DMG + APP).
- Produces updater artifacts/signatures (`createUpdaterArtifacts: true` in `tauri.conf.json`).
- Publishes all generated bundle assets to the GitHub Release for that tag.
- Production origin is the current public GitHub repository: `pietroMastro92/Sbobino`.
- Default recommendation: prepare every release locally first with `./scripts/prepare_local_release.sh <version>`, publish the GitHub release as stable by default, run remote validation, and test that exact release on a second Mac.
- Candidate versions are optional. If you intentionally use a prerelease and it fails validation on a third-party Mac, retire it and cut a new patch version instead of overwriting a stable release or reusing the same candidate.
- Required public asset set for every distributable version:
  - `Sbobino_<version>_aarch64.dmg`
  - `Sbobino.app.tar.gz`
  - `Sbobino.app.tar.gz.sig`
  - `latest.json`
  - `setup-manifest.json`
  - `speech-runtime-macos-aarch64.zip`
  - `runtime-manifest.json`
  - `pyannote-runtime-macos-aarch64.zip`
  - `pyannote-model-community-1.zip`
  - `pyannote-manifest.json`
- `setup-manifest.json` is the single bootstrap contract for first-launch setup and repair. Runtime and pyannote manifests are no longer treated as independent entrypoints.

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
- Build signed bundles in CI when Apple credentials are available.
- Submit DMG to Apple Notary Service with `xcrun notarytool`.
- Staple notarization ticket with `xcrun stapler`.
- If Apple signing or notarization credentials are not configured, the workflow still produces an unsigned Apple Silicon release. In that fallback mode, users must open the app once via Gatekeeper by Control-clicking **Sbobino** and choosing **Open**, or by using **System Settings > Privacy & Security > Open Anyway**.
- To avoid consuming GitHub Actions minutes, prefer the local release flow:
  - `cd sbobino_desktop`
  - `./scripts/prepare_local_release.sh <version>`
  - publish the release with `./scripts/publish_candidate_release.sh <version>` or create the GitHub release `v<version>` manually
  - upload the generated files from `dist/local-release/v<version>/`
  - run `./scripts/distribution_readiness.sh <version>`
  - test that GitHub release on a second Apple Silicon Mac
  - if you intentionally published a prerelease first, promote it only after it passes with `./scripts/promote_candidate_release.sh <version>`
  - if the release fails, delete/retire it with `./scripts/retire_failed_candidate.sh <version>` and cut a new patch version
  - the default `public` profile keeps pyannote out of the app bundle and installs it from release assets during first launch
  - the script automatically generates and reuses a stable local Tauri updater keypair under the user's config directory when one is not already present
  - `SBOBINO_RELEASE_PROFILE=standalone-dev` is reserved for internal/offline builds that intentionally embed bundled pyannote assets
- Inject the updater public key in CI before `tauri build`:
  - `./scripts/prepare_release_updater_config.sh apps/desktop/src-tauri/tauri.conf.json "$TAURI_UPDATER_PUBLIC_KEY"`
- Before packaging the separate pyannote release assets, hydrate the pyannote source assets on the macOS build machine:
  - `./scripts/setup_bundled_pyannote.sh --force`
  - this populates `apps/desktop/src-tauri/resources/pyannote/model` and `apps/desktop/src-tauri/resources/pyannote/python/<target-triple>`
  - the public packaged app must not bundle these files inside the DMG; they are zipped as release assets and installed during first-launch setup
- Build the local runtime and pyannote setup assets before publishing the tag:
  - `./scripts/package_pyannote_asset.sh <runtime_aarch64_dir> python <output_zip>`
  - `./scripts/package_pyannote_asset.sh <model_dir> model <output_zip>`
  - `./scripts/package_macos_runtime_asset.sh <output_zip>`
  - `./scripts/generate_release_manifests.sh <version> <asset_dir>`
  - the manifest generator writes `runtime-manifest.json`, `pyannote-manifest.json`, and `setup-manifest.json` with checksums for the same GitHub Release as the app bundles.

### Windows
- Out of scope for the initial public release.

## Auto-Updates

- `tauri.conf.json` enables updater artifact generation.
- Updater plugin config is enabled and points to:
  - `https://github.com/pietroMastro92/Sbobino/releases/latest/download/latest.json`
- The repository version of `tauri.conf.json` intentionally keeps a placeholder `pubkey`; CI injects the real public key during release builds.
- Local public releases also inject a real updater public key and sign the updater tarball with the stable local Tauri updater keypair. This updater signing is independent from Apple code signing and does not require an Apple Developer account.
- The repo slug is fixed in production code; normal user settings no longer control where setup or updates are downloaded from.
- The frontend performs:
  1. silent update check on startup when enabled;
  2. manual "Check Updates" action in settings;
  3. in-app install when Tauri updater returns an installable update;
  4. manual GitHub download fallback if native updater is unavailable.

## Validation Gates

### Build Readiness
- `./scripts/release_readiness.sh <version> [app-path]`
- Runs local version checks, frontend tests, targeted Rust tests, runtime/pyannote packaging, and bundle sanity checks.
- Uses `SBOBINO_LOCAL_RELEASE_ASSETS_DIR` only as a local validation override; it is not treated as a distributable origin.

### Distribution Readiness
- `./scripts/distribution_readiness.sh <version> [repo-slug]`
- Runs only after the full asset set is uploaded to a GitHub release.
- Verifies HTTP availability, JSON parsing, `app_version` consistency, checksum integrity, updater tarball/signature wiring, and that `setup-manifest.json` points only to assets present in the same release.

## Stable Release Policy

- Stable GitHub releases are immutable for distribution purposes.
- Do not replace stable assets in place to repair a bad public release.
- The supported correction path is:
  1. retire the failed public release
  2. cut a new patch version
  3. publish and validate a fresh release
  4. use prerelease promotion only when you intentionally opted into that flow

## Startup Contract

- First launch on a clean machine depends on the published GitHub release assets for that exact version.
- Once setup completes and `setup-report.json` is trusted for the current build, startup becomes local-first.
- A machine with a valid local runtime and pyannote install must reopen directly even if GitHub release assets are missing or temporarily unavailable.
- Remote asset failures should degrade to a non-blocking warning in Local Models or Settings, not force the user back into the setup gate.

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
8. Validate updater flow by shipping `v0.1.0`, then publish `v0.1.1` and verify in-app update from the public arm64 release.
9. Run side-by-side comparison runs between Python and Tauri app outputs for confidence.
10. Cut over when parity checklist is green and error telemetry is stable.

## Behavior-Safety Strategy

- Keep domain/application contract tests mandatory in PR checks.
- Add adapter integration tests for ffmpeg, whisper-cli, and Gemini against staging fixtures.
- Maintain a "known differences" ledger during migration to avoid accidental regressions.
- Ship internal alpha and external beta before replacing legacy Python distribution.
