# Companion 2.0.39 — final lifecycle integration, 2026-11-15

The single main CI at `1188c64` failed four Intel lifecycle assertions. Those failures were reproduced locally before another hosted run. Version 2.0.39 is retained: no tag exists. Historical tags 2.0.37/38 and stable 2.0.36 are preserved.

## Final behavior

- Whisper classifies a worker failure observed during the initial interrupt wait as a crash, preserving the captured WAV and refusing to save a partial preview. An actual child terminated by SIGINT is tested as a successful intentional stop. The test for an already-exited worker awaits observable exit instead of sleeping 25 ms.
- Pairing and AI stores explicitly unlock on release/final ownership drop. Storage-initialization failures also unlock before returning. The accept/AI drain order is unchanged; overlapping starts and revocations still fail closed until ownership ends.
- Deterministic Unix descriptor-duplication tests exercise pairing explicit release, pairing Drop and final AI-owner Drop while a duplicate descriptor remains open. The AI test was captured RED before the Drop fix (`ai-lease-red.log`) and GREEN after it. These tests pin the explicit-unlock contract; they do not establish a kernel-level explanation for the original intermittent close-time observation.
- The occupied-port test owns the conflicting listener throughout. Restart tests persist a free port below the common default ephemeral ranges (20000–20799), with atomic allocation per probe. An unrelated process can still claim a port between probe and bind. This changes test setup, not production port allocation or identity rules.

## Review disposition

DeepSeek's independent read-only review found no P1 or blocking defect. Root added the requested positive SIGINT test and explicit unlocks on the two storage-opening error paths. Escalated TERM/KILL waits retain their prior behavior: exit attribution after forced termination is a separate robustness limit, not claimed fixed. The first-start ephemeral-listener path is still covered by the basic server tests; restart tests now use a pre-persisted identity and do not certify the former first-start/restart composition under ephemeral-port contention.

The implementation-lane report's claim of asynchronous kernel close-time release has been corrected to an unverified attribution. Observations, stress results and explicit-unlock behavior are distinguished. Review report and root's full source-bound logs are external evidence, not a fresh hosted certification.

## Final local gates

| Process target | Complete workspace | Full infrastructure suite at 32 test threads | Lint |
| --- | --- | --- | --- |
| aarch64-apple-darwin | 572 passed / 0 failed / 5 declared ignored | 20/20, 229/229 per run | Clippy all-targets clean |
| x86_64-apple-darwin | 572 passed / 0 failed / 4 declared ignored | 20/20, 229/229 per run | Clippy all-targets clean |

All 40 stress runs must pass; the driver stops at the first failure and does not retry. The 5/4 ignored real-runtime integration smokes are unchanged and declared; no first-party skip or assertion was introduced. Formatting, version/notes coherence, no-skipped guard and 35 release contract/quality tests pass (one pre-existing PowerShell-only skip on this Mac).

The nine packaging sources are byte-identical to the prior full source-bound hydration gate. Its actual hydration/packaging/relocation/import/signature evidence is carried forward by hash, without another model download. Native Intel/Windows distribution proofs remain required.

## Native public app

Rebuilt from the final four frozen Rust files with one Cargo job, incremental off, offline caches/TMPDIR/frontend output on mounted UltraDisk, HOME unchanged. Identity `com.sbobino.desktop`, 2.0.39, arm64, public resources (Pyannote excluded), ad-hoc deep/strict signature verified. Executable SHA-256: `cc83ddc880fe29270214cbf4bdedcc6c1738856f542dd978aa3ad7aecc9463dd`.

This is an app build, not GUI/LAN/iPhone validation. Local updater signing remains unavailable; no updater signature or public release PASS is inferred. The previous local DMG-mount limitation is unchanged and is not retried here.

## Provenance and scope

Evidence root: `/Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/root-lifecycle-review/` (`evidence/final-local-integration.json`, source-before/after manifests, RED/GREEN log, full workspace/Clippy logs and all 40 stress logs). App: sibling `native-lifecycle-2039/dist/bundle/Sbobino.app`; its source and executable receipt is in `evidence/final-local-integration.json`.

Four Rust files, release notes and these review documents form the product integration. No dependency/version/workflow change is part of that product fix. No personal app/profile, iPhone, simulator, Keychain, VPN or firewall was changed; test storage is synthetic and task-owned. Local x86_64 is Rosetta on ARM, not Intel hardware/macOS 15/Xcode 16.4 certification. No GitHub rerun, tag, candidate or promotion was performed by these local gates.

Next: one CI/docs-only commit pins this reviewed product tree, then exactly one main push triggers the unchanged official CI/verification-installer matrix. The Release Candidate remains held until all required native CI jobs succeed on that exact head.
