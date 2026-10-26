# Repository state — 2026-10-26

`main` is the canonical development branch of https://github.com/pietroMastro92/Sbobino. The canonical local checkout is `/Users/pietromastro/Documents/sbobino_tauri`.

## Release baseline

`v2.0.36` is the published stable release, commit `aa171fa31ebc77e00a49b8530a3053cdfc1ae079`. Main now contains that entire history plus the later main studies/benchmarks; the integration merge is `3d5d744f824d0e34e4c0a2b4d89a8d7e34541c9c`. Version remains 2.0.36; this consolidation does not create or certify another release.

Use the tag to reproduce the exact public release. Main also preserves subsequent research, which is not a product capability. Historical release tags, public artifacts and test evidence remain available for rollback and provenance. Old dated plans/worktree paths are historical and must not override this checkout.

## Pending work and iPhone integration

Local uncommitted companion, transcription/storage and encryption changes were preserved across the merge; they are not included in the public release or automatically included in main commits. Do not commit, remove or overwrite another agent's pending files during cleanup.

The public v2.0.36 tree has no companion server, panel or protocol capability. The local draft uses opt-in TLS and pairing with bearer token/certificate pinning. Its routes are authenticated `GET /v1/health`, `PUT /v1/jobs/{uuid}`, `GET /v1/jobs/{uuid}`, and `DELETE /v1/jobs/{uuid}` (cancellation). Upload query: `extension`, optional `title`, and optional `diarization=true|false`; the mobile client should send the diarization choice explicitly after checking `companion_diarization_v1` in health capabilities.

The draft result includes transcript, artifact ID, duration, engine/model and optional `timeline_v2` (JSON encoded as a string), `diarization_status` and `diarization_error`. These optional fields are not proof that diarization succeeded; inspect the returned status. Desktop reports completion after verifying the saved artifact. Job IDs deduplicate only within the current server's in-memory retention; results are not a durable cross-restart transport queue. There is currently no separate delivery-ACK route: DELETE cancels, it does not acknowledge durable iPhone storage. AI summary/correction endpoints are not part of this draft contract.

Before shipping companion, agree explicit ACK/recovery semantics and test idempotent durable iPhone storage, reconnect/server restart, cancellation, and actual device transport. The observed iPhone-to-Mac TLS timeout remains unresolved; loopback smoke does not certify LAN E2E. Preserve VPN, firewall and user network settings. Do not advertise companion based on the desktop version string alone.

## Recovery and validation

Consolidation backups are private on UltraDisk under `codex-sbobino-tests/repository-consolidation-20261026`: verified Git bundle, original refs/worktree inventory, per-checkout binary patches and file archives with SHA-256 manifests. They are recovery archives, not parallel development checkouts. Pending work from retired experimental branches is archived, not assumed merged or validated.

A fresh main CI must validate the integrated commit on standard GitHub runners; previous release CI is historical evidence, not a new result. Keep native interactive/GPU/microphone, spoken screen-reader, multilingual-quality and OS signing limits stated in release notes. Do not publish, tag or promote as part of this cleanup.
