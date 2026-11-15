# Repository state — 2026-11-15

`main` is the canonical development branch of https://github.com/pietroMastro92/Sbobino. The canonical local checkout is `/Users/pietromastro/Documents/sbobino_tauri`.

## Current release preparation — 2026-11-15

The stable baseline remains v2.0.36. Candidate tags v2.0.37 and v2.0.38 are retained unchanged; neither produced a publishable candidate. Run 43692826685 was cancelled after its ARM hydration failure.

The next product version is **2.0.39**, prepared locally after reproducing the failure through the complete hydration path. DeepSeek's changes plus root review corrections cover universal LLVM bitcode, explicit per-architecture Mach-O queries, embedded-library sanitization, path/collision guards, Rust lint and Windows pairing-state error classification. See `docs/reviews/companion-local-release-integration-20261115.md`.

The single main CI at `1188c64` (43709717704) passed ARM, Windows and Linux but failed four Intel lifecycle assertions; verification installers 43709717718 passed all three platforms. Those failures were reproduced and repaired locally before another push. The reviewed product commit is now `153a86c194ca43b1172d570743e0adc0297980f0`; the v2.0.39 workflow is pinned to that exact product tree. See `docs/reviews/companion-lifecycle-final-integration-20261115.md`.

Final local evidence for that product: 572 workspace tests passed on native ARM and in an x86_64 process through Rosetta (5/4 declared real-runtime smokes ignored), all-targets Clippy clean on both, and 20/20 complete infrastructure stress runs of 229 tests per architecture. The public ARM app was rebuilt and its ad-hoc deep/strict signature verified. The nine packaging sources are byte-identical to the prior complete hydration/packaging gate, so those source-bound proofs and the unchanged 217-test frontend evidence are carried forward. Native Intel/Windows distribution proofs remain mandatory; Rosetta is not Intel hardware certification. At this local checkpoint no v2.0.39 tag, candidate or promotion exists. One new main push is allowed only for the unchanged official CI/verification-installer matrix; no Release Candidate dispatch until that exact head is green.

The official runner matrix and promotion contract are unchanged. Main CI must finish green before a Release Candidate dispatch; candidate publication, all same-tag/SHA proofs/manifests, updater signature verification and the promotion gate precede any stable update. No test profile pairing is copied into the official identity.

The dated notes below are historical snapshots wherever they describe 2.0.37 or 2.0.38 as current, or describe old local/remote branch state.

## Release baseline

`v2.0.36` is the published stable release, commit `aa171fa31ebc77e00a49b8530a3053cdfc1ae079`. Main now contains that entire history plus the later main studies/benchmarks; the integration merge is `3d5d744f824d0e34e4c0a2b4d89a8d7e34541c9c`. The product version on main is 2.0.37 since commit `f9055b289ede447a265e1cb51a177b304dbcf3bd`; that commit is unreleased preparation plus the Companion implementation, not a release. Local `main` is that commit, two commits ahead of `origin/main` (`692cdb58a5853ea0d9d5ea910da1c0365b11f0e0`) and not pushed; `v2.0.36` is still the newest tag locally and remotely, so it remains the latest stable release and the only version the updater endpoint serves.

### Committed product version 2.0.37 (2026-11-14)

Commit `f9055b289ede447a265e1cb51a177b304dbcf3bd` ("prepare Desktop 2.0.37 with persistent iPhone Companion", 36 files) is the reviewed product tree of this cycle. It commits the 2.0.37 metadata — `sbobino_desktop/apps/desktop/package.json`, `package-lock.json`, `src-tauri/tauri.conf.json`, the four product `Cargo.toml` files and the four local `Cargo.lock` entries — together with `sbobino_desktop/docs/release-notes/v2.0.37.md`, the Companion server/protocol/pairing code, the AI v2 job routes, measured speaker turns, diarization readiness reason codes, the Companion UI/i18n/accessibility work, and the portable Pyannote packaging/sanitize/audit tooling with its documentation. `check_release_versions.sh 2.0.37` and the release-notes gate pass on that tree.

This is **not** a published version. There is no `v2.0.37` tag, no candidate prerelease, no promotion and no public artifact; the Pyannote portable tooling still has to produce archives with a zero-violation audit before any public claim.

The candidate and promotion workflows are now bound to that reviewed commit: `release.yml` locks the tag to `v2.0.37` with `REVIEWED_PRODUCT_SHA=f9055b289ede447a265e1cb51a177b304dbcf3bd` and still requires `git diff` to be empty between that SHA and the tagged tree for `sbobino_desktop/apps`, `sbobino_desktop/crates`, `sbobino_desktop/scripts`, `sbobino_desktop/Cargo.toml` and `sbobino_desktop/Cargo.lock`; `promote-release.yml` is locked to `v2.0.37`; `verification-packages.yml` calls `check_release_versions.sh 2.0.37`. Pins move only with a reviewed product commit, never ahead of it.

`ci.yml` keeps its Intel Pyannote/Parakeet smoke pinned to release `2.0.36`: that argument selects the published runtime assets to download, and no 2.0.37 assets exist yet. Historical validation workflows (`arm-runtime-validation.yml`, `intel-runtime-validation.yml`, `windows-port.yml`) stay pinned to `v2.0.34` on purpose.

Use the tag to reproduce the exact public release. Main also preserves subsequent research, which is not a product capability. Historical release tags, public artifacts and test evidence remain available for rollback and provenance. Old dated plans/worktree paths are historical and must not override this checkout.

## Pending work and iPhone integration

The Companion implementation plus the linked transcription/storage and encryption changes are committed on `main` at `f9055b289…` and remain unreleased: they are in no public release and no 2.0.37 artifact exists. Do not commit, remove or overwrite another agent's pending files during cleanup, and do not infer a published release from the committed version string.

The public v2.0.36 tree has no companion server, panel or protocol capability. The unreleased implementation uses opt-in TLS and pairing with bearer token/certificate pinning. Its routes are authenticated `GET /v1/health`, `PUT /v1/jobs/{uuid}`, `GET /v1/jobs/{uuid}`, and `DELETE /v1/jobs/{uuid}` (cancellation). Upload query: `extension`, optional `title`, and optional `diarization=true|false`; the mobile client should send the diarization choice explicitly after checking `companion_diarization_v1` in health capabilities.

The ASR result includes transcript, artifact ID, duration, engine/model and optional `timeline_v2` (JSON encoded as a string), `diarization_status` and `diarization_error`. These optional fields are not proof that diarization succeeded; inspect the returned status. Desktop reports completion after verifying the saved artifact. ASR job IDs deduplicate only within the current server's in-memory retention; those results are not a durable cross-restart transport queue. There is no separate ASR delivery-ACK route: DELETE cancels, it does not acknowledge durable iPhone storage.

The additional AI implementation is committed on main and specified in [companion-ai-v1.md](companion-ai-v1.md). Its authenticated `/v1/ai/jobs/{uuid}` operations and `/ack` route have separate encrypted durable records and explicit acknowledgement/recovery semantics. They do not upgrade the legacy ASR queue and do not exist in public v2.0.36. Provider selection is explicit and local-only; availability is advertised separately from ASR.

Before shipping companion, validate idempotent durable iPhone storage, reconnect/server restart, cancellation, and actual device transport. The 2026-10-27 isolated iPhone AI attempt stopped at health after 31.014 seconds, before generation, save or ACK. Mac loopback TLS/HTTP succeeded; macOS Application Firewall logs show a pending filtering decision for the synthetic helper. No firewall exception was applied; explicit user confirmation is pending. This is a blocked environment check, not LAN E2E certification. Preserve VPN, firewall and user network settings. Do not advertise companion based on the desktop version string alone.

The synthetic SQLite import/history suite now passes 10 tests on native macOS arm64 after correcting two fixtures to set the real external audio path, as production does. This does not certify the complete companion implementation or other platforms. Raw logs, hashes and the sanitized phone failure report remain on UltraDisk under `codex-sbobino-tests/ai-companion-20261026`.

The AI implementation's key-points prose regression is fixed in the reviewed tree: operation-specific structured output is validated and rendered as bullets in the unchanged public `result.text`. A subsequent real Foundation failure exposed duplicate citation keys from free-form generation; companion requests now use native guided generation while the existing desktop mode remains free-form. On 2026-10-27 the then-current macOS arm64 infrastructure unit suite passed 162 tests (including 19 companion tests), build, all-targets Clippy and Tauri compilation passed, and five synthetic operations each with real Apple Foundation and local Ollama/Qwen3 passed loopback protocol/format/replay/ACK checks. Proofs and 179-file source/configuration, helper/test/bridge and model hashes are preserved under `ai-companion-20261026/providers-grounding-final-20261027`. Source-only semantic quality remains unverified beyond the toy-case review, not an accuracy PASS. This is not packaged-app, Intel/Windows, iPhone transport, WAN or comparative performance certification. Windows CI now includes targeted companion execution, with valid YAML; the complete committed snapshot still needs a hosted CI run on the pushed commit.

## Recovery and validation

Consolidation backups are private on UltraDisk under `codex-sbobino-tests/repository-consolidation-20261026`: verified Git bundle, original refs/worktree inventory, per-checkout binary patches and file archives with SHA-256 manifests. They are recovery archives, not parallel development checkouts. Pending work from retired experimental branches is archived, not assumed merged or validated.

A fresh main CI must validate the integrated commit on standard GitHub runners; previous release CI is historical evidence, not a new result. Keep native interactive/GPU/microphone, spoken screen-reader, multilingual-quality and OS signing limits stated in release notes. Do not publish, tag or promote as part of this cleanup.
