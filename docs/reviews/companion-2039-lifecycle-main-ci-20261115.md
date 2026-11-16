# Companion 2.0.39 — official main CI lifecycle verification (2026-11-15)

Independent verification of the single post-freeze main CI run, observed to terminal
completion. No dispatch, rerun, push, tag, release, asset, promotion, code edit, app
launch, device or VM was performed.

## Verdict

**PASS.** Run `43724115449` reached terminal state `completed / success` at
`2026-11-15T15:02:26Z` on head `67b83f9cf4ebdbda4b0f579fce624eaaf1f9c488`
(`.github/workflows/ci.yml`, event `push`, attempt 1). The four Intel lifecycle failures
that broke `1188c64` are gone, and this run is the first green same-head official matrix
for the 2.0.39 product tree.

## Provenance

| Item | Value |
| --- | --- |
| Verified head | `67b83f9cf4ebdbda4b0f579fce624eaaf1f9c488` |
| Local `HEAD` = `origin/main` | identical to verified head |
| Product pin | `153a86c194ca43b1172d570743e0adc0297980f0` |
| Product-tree diff pin → head | **empty**; only `.github/workflows/release.yml` + `docs/REPOSITORY_STATE.md` changed |
| `release.yml` delta | `REVIEWED_PRODUCT_SHA` `7374914…` → `153a86c…` |
| `v2.0.39` tag / release | **absent** |
| Stable `v2.0.36` | intact, 28 assets, still `Latest` |

Product source hashes at the verified head, byte-identical to root's
`final-local-integration.json`:

```
a869d5f766b45452043f8d0f008f71e6a0ee2286d0600947beb08dc9b9f8c4f1  adapters/whisper_stream.rs
f357720b13757f1e6ab5f2fa7f36ee3a8a07b2ff00d74f4fbfe595274f1af85a  companion.rs
815c25b300e442c3f6bb153fe317f89c14286df5bf5727c248970527f0a77aa8  companion_identity.rs
d9bc60d4649c8feae4520d6dfa443b300dcf9fd1783895eca8411243bee1d251  companion_ai.rs
```

Exactly one workflow ran on this head — the CI run. The optional `verification-packages`
(installers) workflow did **not** fire, so no `1188c64` installer artifact was reused as
proof for this head. Installers remain optional for the release contract.

## Platform matrix

| Job | Conclusion | Proof scope |
| --- | --- | --- |
| frontend | success | 38 files, **217/217** tests, build 1819 modules, version coherence 2.0.39, no-skipped-tests clean |
| rust (Linux, ubuntu-latest) | success | Clippy `-D warnings` clean; **397 passed / 3 ignored** (lib **227**); runtime contract tests ok |
| Native Rust tests (aarch64-apple-darwin) | success | native `arm64`; Clippy `--workspace --all-targets` clean; **572 passed / 5 ignored** (lib **229**); FluidAudio Swift **11/11**; **ARM offline inference passed** (2 speakers) |
| Native Rust tests (x86_64-apple-darwin) | success | native `x86_64` on `macos-15-intel` — **not Rosetta**; Clippy clean; **572 passed / 4 ignored** (lib **229**); FluidAudio Swift **11/11**; **Intel published 2.0.36 Pyannote + Parakeet smoke passed** |
| Native Rust tests (Windows x86_64) | success | native `AMD64`, Windows 10.0.26100; portable sweep **242 passed / 1 ignored** across 5 binaries; 7 live-stream record tests, credential-manager roundtrip, 53 companion/AI tests, FluidAudio adapter protocol |
| windows-bootstrap | success | `continue-on-error: true` — **not** native proof (native Windows is the job above) |
| Diagnose published 2.0.34 Intel Live runtime | skipped | `if: workflow_dispatch` — legitimately skipped on push |

### Counts, not assumed totals

Cross-platform totals differ for a real reason and were captured, not normalized:
`lib` is **229** on both Apple targets and **227** on Linux because exactly two tests in
`crates/infrastructure/src/lib.rs` are `#[cfg(target_os = "macos")]`. ARM ignores 5 and
Intel ignores 4 declared real-runtime smokes. Windows portable binaries are a smaller
platform-gated set (242). These match root's local 572/5 (ARM) and 572/4 (Intel).

### Intel smoke is 2.0.36, not a 2.0.39 asset

`proof.json` records `version: 2.0.36`, `release_tag: v2.0.36`, harness commit
`67b83f9…`. The consumed asset digests equal the live `v2.0.36` release digests
(`speech-runtime-macos-x86_64` `a29d02ca…`, `pyannote-runtime-macos-x86_64` `a1e47a0e…`,
`pyannote-model-community-1` `00ebe187…`), with API digest == downloaded SHA-256. Result:
Pyannote `passed` (65 s audio, CPU), Parakeet `passed` (WER 0.043478, CER 0.010638, CPU).
Parakeet realtime on Intel is explicitly **not certified** and is not claimed here.

## What this run does and does not prove

Proved: the official CI matrix is green on this exact head, the lifecycle fixes hold on
native ARM, native Intel and native Windows, and the Intel Pyannote/Parakeet inference
path works against the published 2.0.36 runtime.

Not proved by this run: a 2.0.39 Release Candidate. No 2.0.39 tag, candidate, installer
set, updater signature, or promotion exists. Rosetta/local evidence is not a substitute
for the native Intel and Windows distribution proofs above — those are now satisfied in
CI, but the release build itself has not been produced.

## Gates still pending before RC

1. Release Candidate dispatch at this head (first allowed dispatch after green CI).
2. All three platform installers built for 2.0.39 (ARM, Intel, Windows).
3. Seven proofs / four manifests on the same tag+SHA as the candidate.
4. Updater signature verification and the promotion gate.
5. Any stable-update decision — `v2.0.36` stays untouched until the gate passes.

## Evidence

Root: `/Volumes/UltraDisk/sbobino-companion-product-20261115/ci-lifecycle-2039/`
- `evidence/verification-matrix.json`, `per-job-state.json`, `run-final-full.json`,
  `watcher-state.log`
- `logs/run-43724115449-full.log`
- `artifacts/{fluid-diarizer-aarch64-apple-darwin, fluid-diarizer-x86_64-apple-darwin,
  intel-supported-inference-67b83f9…}` (small reports only; no installers downloaded)

## Next concrete action for root

Dispatch the 2.0.39 Release Candidate from `67b83f9cf4ebdbda4b0f579fce624eaaf1f9c488`,
which builds all three platforms and must emit the 7 proofs / 4 manifests, then verify
updater signatures before any promotion. Do not retag or touch `v2.0.36`.
