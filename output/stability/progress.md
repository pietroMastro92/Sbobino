# Stability progress

Updated: 2026-09-16 Europe/Rome
Snapshot: branch `codex/stability-completion-20260916`; exact source SHA is recorded by the post-commit manifest and CI evidence.
Worktree: `/Volumes/UltraDisk/codex-sbobino-tests/stability-20260916`

## Current status

- PASS — UltraDisk cleanup: 13 obsolete Cargo targets removed; free space increased from 2.7 GiB to 91 GiB.
- PASS — Provisioning lifecycle regression tests: 13/13 on the current snapshot.
- PASS — Frontend suite: 35 files, 183 tests.
- PASS — Frontend production build (`tsc && vite build`).
- PASS — Parakeet worker capability tests: exact `--threads`, longer-option rejection and timeout; 3/3.
- PASS — Rust workspace: 433 passed, 4 ignored because they require explicitly supplied real runtimes/models/audio.
- PASS — `cargo fmt --check` and Clippy with warnings denied.
- PASS — Workflow contracts: 20/20; validation workflow YAML and shell syntax checked.
- PASS — Source and post-build artifact manifest helpers exercised locally.
- IN PROGRESS — Native validation workflow prepared; hosted execution awaits the reviewed branch push.
- PASS — Clean macOS arm64 technical package: `Sbobino.app` and `Sbobino_2.0.32_aarch64.dmg` built on macOS 27 with Rust 1.95.0 after disabling release stripping for the affected validation build. Artifact manifest aggregate SHA-256: `ff887feaae8eeb8dc150987ccd87f6f1276dedfbd248cf437a8327bae0d97367`.
- PASS — Native arm64 launch smoke: the packaged executable remained alive for five seconds with `HOME` and `TMPDIR` isolated on UltraDisk, then was intentionally stopped (`SIGINT`, exit 130). The isolated app-data directory was created under the test profile.
- BLOCKED — The local macOS bundle is ad-hoc linker-signed only and fails strict bundle verification because resources are not sealed. Developer ID signing and notarization were not available and are not claimed.
- BLOCKED — Native Intel macOS and Windows functional evidence until the validation branch is pushed and runners execute the same final commit.
- BLOCKED — VoiceOver/Narrator, microphone, GPU and three offline app relaunches require native interactive sessions.

## Evidence rules

Only commands executed from this worktree against the current changes are marked PASS. Historical logs, synthetic tests and results from other platforms are not promoted to native proof.
