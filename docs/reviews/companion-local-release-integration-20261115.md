# Local release recovery — root integration review, 2026-11-15

Repository: `/Users/pietromastro/Documents/sbobino_tauri`, base `31d832b7083ba86588774c0230a9bf7d5996d5a7`.

The failed candidate run 43692826685 was cancelled. No further hosted run was used to develop these fixes.
The preceding tags remain immutable. This review permits preparation of the next candidate; it does not certify a public release.

## Fixes and review

DeepSeek reproduced the exact python.org universal2 LLVM-bitcode `python.o` failure through the full hydration command. The fix distinguishes every real Mach-O slice from bitcode, selects architectures explicitly for `otool`, modifies install IDs only on dylibs, and sanitizes embedded host dependencies to a fixpoint. A second complete hydration from Homebrew exposed and closed the latter defect.

Independent review found no P1 in that lane and three small P2 findings. Root closed them with behavioural RED/GREEN tests: conflicting host basenames are rejected during planning before any write; runtime deletion requires canonical containment and the personal-path guard; work directories are canonicalized and rejected inside the checkout before creation. A relative TMPDIR is refused before readiness creates temporary artifacts. Review source-text assertions were supplemented with executable filesystem/collision counterexamples.

The complete suite was also run in a genuine x86_64 Python process through Rosetta. Its first run exposed ARM-default fixture compilation and a child interpreter selecting another architecture. Fixtures now compile explicitly for the testing process and the ctypes subprocess is launched with the same architecture. All original assertions remain. These are Rosetta process-mode checks, not certification of an Intel Mac.

The local gate now snapshots exact source hashes before and after each run, fails if they differ, labels unit/verify/full-hydration modes separately, and does not hydrate after a failed unit phase.

## Measured local evidence

| Gate | Result |
| --- | --- |
| Exact original full hydration RED (official python.org input) | reproduced the hosted install_name_tool error |
| Full repaired python.org hydration, package, relocation and loader checks | PASS; 0 portability violations; strict closure not claimed |
| Fresh repaired Homebrew hydration after root collision/path fixes | PASS; 5 dylibs embedded; 0 violations; in-place and relocated speech entrypoint/imports/signatures pass |
| Packaging/staging/loader regression entrypoints, native ARM | 61 + 16 + 10 = 87 PASS |
| Same entrypoints, x86_64 Python via Rosetta | 87 PASS, no skip |
| Pinned Rust 1.95 workspace Clippy -D warnings, aarch64 target | PASS |
| Actual full release_readiness.sh, public ARM app | PASS; regenerated runtime, packaged/extracted archives, probes, 217 frontend tests and 566 Rust tests |
| Existing real-runtime Rust smoke tests | 5 ignored for explicitly required model/audio/runtime fixtures; not counted as passes |
| Public Tauri build --target aarch64-apple-darwin | PASS; com.sbobino.desktop 2.0.38, deep/strict ad-hoc signature; FluidAudio 11/11 |
| Bundle vs extracted updater tar | identical executable sha256 3d611b871777642efd05dd02e416eb0e804ffb8f4c9d974faf63ee10aa8affc3 |
| DMG checksum | VALID; local filesystem mounting did not complete; owned read-only attachment detached without force |

Artifacts and commands: `/Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/`.
`native-app/evidence/full-readiness-proof.json` records the complete gate and its timing boundary: the first hydration began before the additional review fixes; the subsequent fresh Homebrew gate and final regression runs cover those fixes. A final source-bound full gate and a native build will be recorded for the next version before pushing.

## Remaining release gates

No Windows native execution, Linux target lint, or native Intel payload build is claimed locally. Those remain mandatory official CI/distribution checks. The local updater tar is unsigned because the private updater key is only configured in the release environment; no key was read or changed. The app has not been installed over personal Sbobino, and no personal data/profile, iPhone, simulator, VPN or firewall was modified.

Only after final source-bound local checks and root review may main CI be triggered once. Release Candidate must then wait for that CI to be green. Promotion still requires the complete official platform matrix, same-tag/SHA public proofs/manifests, updater signatures and promotion checks. No partial ARM/Rosetta success substitutes for Intel/Windows proofs.

## Primary references used for the Rosetta fixture correction

[Apple: Building a universal macOS binary](https://developer.apple.com/documentation/apple-silicon/building-a-universal-macos-binary) and [Apple: posix_spawn architecture preferences](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man3/posix_spawnattr_setbinpref_np.3.html).
Explicit compiler architecture plus explicit child interpreter architecture was selected over universal fixtures (would invalidate thin-architecture mismatch assertions), native-Intel-only execution (unavailable locally), per-load architecture changes (would not exercise x86_64 consistently), or PATH tool wrappers (could hide real tool behavior).

## Final 2.0.39 pre-push checkpoint

The native public 2.0.39 app is built and deep/strict ad-hoc verified: `com.sbobino.desktop`, arm64, executable SHA-256 `4f481bbb827f76aa526893f645e4219e31ba3fde2d275640df4f43e62b1886e9`. Its native build, 217 frontend tests, pinned Rust 1.95 full workspace (566 passed / 0 failed / 5 declared ignored) and Clippy -D warnings pass. Version and release-notes gates pass. The updater public key is byte-identical to stable v2.0.36.

The final **full hydration** gate on the frozen 2.0.39 toolchain passes with the official python.org input. Its nine source hashes before/after match each other and the final working files: `/Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/final-gate-2039/evidence/20261115T111013Z`. Both in-place and relocated product speech entrypoints and import orders pass; signatures are checked; the original fat-bitcode object is unchanged. This closes the earlier timing boundary for the final packaging gate. No strict-closure claim is made.

Exact final checkpoint: `/Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/native-final-2039/evidence/local-product-proof.json`. The earlier full release_readiness.sh run remains explicitly a 2.0.38 integration run; final 2.0.39 app/test/version/hydration checks are separately recorded rather than relabeling that run. All runtime, app and code checks required locally are now green. The default CI and its original platform matrix are next; no release dispatch is allowed until those checks finish green.
