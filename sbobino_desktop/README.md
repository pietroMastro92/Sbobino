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
- `docs/distribution-validation-plan.md`: native macOS and Windows clean-room distribution matrix
- `docs/self-hosted-release-runners.md`: runner labels, machine preparation, and self-hosted validation requirements
- `docs/first-real-candidate-runbook.md`: operational checklist for the first live candidate with GitHub Actions + self-hosted Macs
- `docs/feature-migration-matrix.md`: feature-by-feature parity checklist
- `THIRD_PARTY_NOTICES.md`: licenses and attribution for FFmpeg, whisper.cpp, pyannote, Hugging Face models, and related runtime components
- `docs/github-release-template.md`: copy-paste text for GitHub Release notes (third-party disclaimer + links)

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
4. Public installers do not bundle pyannote or the full local runtime. On first launch the app installs FFmpeg, Whisper CLI, Whisper Stream, Whisper models, and pyannote assets into app data through the guided setup flow.

## Local Release

macOS local staging remains native per architecture. Windows x86_64 artifacts are built and clean-room tested on `windows-2025`; the release workflow assembles all three native staging sets before publication.

That folder now always includes:
- `Sbobino_<version>_aarch64.dmg`
- `Sbobino_<version>_x86_64.dmg`
- `Sbobino_<version>_aarch64.app.tar.gz` and `.sig`
- `Sbobino_<version>_x86_64.app.tar.gz` and `.sig`
- `Sbobino_<version>_windows_x86_64-setup.exe`
- `Sbobino_<version>_windows_x86_64.nsis.zip` and `.sig`
- `latest.json`
- `setup-manifest.json`
- `runtime-manifest.json`
- `speech-runtime-macos-aarch64.zip`
- `speech-runtime-macos-x86_64.zip`
- `speech-runtime-windows-x86_64.zip`
- `pyannote-manifest.json`
- `pyannote-runtime-macos-aarch64.zip`
- `pyannote-runtime-macos-x86_64.zip`
- `pyannote-runtime-windows-x86_64.zip`
- `pyannote-model-community-1.zip`
- `release-readiness-proof.json` (generated only when `release_readiness.sh` passed)
- `AS-THIRD.validation-report.json` and `INTEL-PRIMARY.validation-report.json` are required before stable promotion and must be uploaded with `status=passed`

Manual publish contract:
1. build the release locally
2. publish the GitHub release for the same `v<version>` as a prerelease candidate
3. upload the full asset set
4. run `./scripts/distribution_readiness.sh <version>`
5. write and upload `distribution-readiness-proof.json`
6. test that exact GitHub release against the machine matrix in `docs/distribution-validation-plan.md`
7. update and re-upload all machine validation reports
8. promote to stable only with `./scripts/promote_candidate_release.sh <version>`
9. if it fails, retire that release and cut a new patch version

Helper scripts:
- `./scripts/publish_candidate_release.sh <version>` publishes a prerelease candidate and refuses publishing if readiness proof/checksums/manifests/templates are inconsistent
- `./scripts/promote_candidate_release.sh <version>`
- `./scripts/run_release_machine_validation.sh <machine-class> <version>` validates the exact public candidate on `AS-PRIMARY`, `AS-THIRD`, or `INTEL-PRIMARY`
- `./scripts/install_self_hosted_runner_macos.sh <machine-class>` installs and registers a macOS self-hosted runner with the right labels
- `./scripts/preflight_self_hosted_runner.sh <machine-class>` checks whether a machine is truly ready to serve as a release runner
- `./scripts/check_release_runner_matrix.sh` confirms the full GitHub runner matrix is online before dispatch
- `./scripts/dispatch_release_candidate.sh v<version>` triggers the Release Candidate workflow only when the runner matrix is complete
- `./scripts/watch_release_candidate.sh` watches the latest candidate run and summarizes the jobs at the end
- `./scripts/retire_failed_candidate.sh <version>`

Stable release policy:
- never overwrite or “fix in place” a stable GitHub release
- prerelease candidate validation is mandatory before stable promotion
- mandatory assets for promotion include ARM64, Intel, and Windows distribution proofs, macOS portability proofs, `AS-THIRD.validation-report.json`, and `INTEL-PRIMARY.validation-report.json`
- the default promotion flow retains the latest two validated stable releases

Set `SBOBINO_RELEASE_PROFILE=standalone-dev` only for internal/offline builds that intentionally embed bundled pyannote assets.

## Open source and third-party licenses

Sbobino Desktop is released under the **MIT License** ([`LICENSE`](LICENSE)). Binary releases and first-launch provisioning also redistribute or install other open-source software (FFmpeg, whisper.cpp, Python, pyannote, PyTorch stack, and CC-BY-4.0 model data from Hugging Face). See **[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)** for version pins, links, and compliance notes. When publishing a GitHub Release, use **[`docs/github-release-template.md`](docs/github-release-template.md)** as a starting point for user-facing notes.

## Current Milestone

- Initial clean architecture scaffold
- First background transcription command
- Progress/completion events to frontend
- Minimal working desktop UI
