# FluidAudio diarization — implementation in progress, 2026-10-27

Canonical checkout: `/Users/pietromastro/Documents/sbobino_tauri`, main base
`d7354070510126bff240860813ce61ba856da576`, existing pending changes preserved.
No release or promotion is authorized by this implementation package.

## Scope and platform policy

The user requested reuse of the lightweight iOS diarizer on Desktop, a real
comparison with packaged Pyannote, and investigation/integration of Live speaker
labels. iOS pins FluidAudio `793a2c5a0e99705a55fefbad00e65ca684329349`.

The SDK contains five diarizer families: offline Community-1/VBx, LS-EEND,
Sortformer, legacy Pyannote online, and Nemotron3. Streaming variants are separate
pipelines/models, not the small offline model already used by iOS. LS-EEND is the
first low-latency candidate; output frame cadence is not measured end-to-end
speaker-label latency. Stable/provisional speaker assignments and stop/tail
finalization need explicit validation.

The existing app minimum is macOS 13. FluidAudio requires macOS 14, but its
offline pipeline has a documented BNNS crash on macOS 14, including intermittent
Apple Silicon fallback failures. The initial safe policy is therefore Pyannote
on macOS 13–14, FluidAudio on macOS 15+ after runtime/model/native validation,
and Pyannote on Windows/Linux. This does not introduce support below macOS 13.
Intel native execution and older OS boundaries must have their own proof.

References:
- [Pinned diarizer overview](https://github.com/FluidInference/FluidAudio/blob/793a2c5a0e99705a55fefbad00e65ca684329349/Documentation/Diarization/GettingStarted.md)
- [macOS 14 BNNS crash](https://github.com/FluidInference/FluidAudio/issues/878)
- [Offline model provenance and license](https://huggingface.co/FluidInference/speaker-diarization-coreml)
- [LS-EEND model](https://huggingface.co/FluidInference/ls-eend-coreml)

## Ordered acceptance packages

1. **Native parity proof:** compiled Swift helper at a fixed SDK revision,
   offline/no implicit network fallback, CPU/ANE measurements, same public audio
   and reference as the actual packaged Pyannote pipeline. Record load/processing
   wall time, peak memory, labels, DER/JER where actual reference annotations
   exist; one cold, three genuinely model-warm passes, and a long recording.
2. **File integration:** reuse `SpeakerDiarizationEngine`, add the native adapter,
   platform routing, model/runtime availability and provisioning, packaging,
   existing settings/status UI and regressions. A missing native helper/model
   must remain visible; do not masquerade it as a valid Python installation.
3. **Streaming proof:** LS-EEND local-model loader and chunked input, bounded
   queues, actual per-update delay/backlog, zero dropped samples, complete tail,
   cancellation/restart and stable identity checks. Then connect both supported
   Live ASR paths and final saved metadata without changing transcript text.
4. **Native CI:** use standard GitHub macOS arm64/Intel and Windows runners for
   the same coherent snapshot. Linux gets only declared portable checks.
   Compilation/fixtures cannot certify microphone/GPU or an interactive desktop.

Do not enable an unvalidated pipeline merely because a small model loaded. Keep
public v2.0.36 unchanged until the relevant release acceptance gates are met.

## Checkpoint — 2026-10-28

All native artifacts and raw proofs are under
`/Volumes/UltraDisk/codex-sbobino-tests/fluid-diarization-20261027/evidence`.
Measurements below are macOS arm64 only. The first LS-EEND run overlapping an
iOS compile is excluded; its raw files remain preserved. Cold means first model
load/inference in process, without flushing the OS cache. Peak RSS excludes
external CoreML/ANE/system allocations. ANE is the requested compute policy,
not a per-operation hardware trace.

| Pipeline | Cold 120 s | Warm median (3) | Full 1049.35 s | Peak process RSS |
|---|---:|---:|---:|---:|
| Packaged Pyannote CPU, rerun current script | 73.548 s | 68.769 s | 669.319 s | 2,882,502,656 bytes |
| Fluid offline CPU | 1.303 s | 1.122 s | 12.786 s | 302,284,800 bytes |
| Fluid offline CPU + ANE requested | 1.471 s | 0.492 s | 3.965 s | 261,783,552 bytes |

Same public AMI mixed-headset meeting, same machine and settings per pipeline;
these numbers are diarization, not ASR or whole-app latency. Fluid helper hash
for this benchmark: `baeb39d028aaf0433fab2bc617078125d98c18f33c6c0c2e67fec2072ef02680`.
Raw `final-benchmark-fluid-*.json`, `.time.log`, `benchmark-pyannote-*`
and source/model manifests identify the actual inputs and implementations.

Zero-collar DER/JER includes overlap. Short excerpt has four reference voices:
Pyannote DER 24.89%, Fluid offline 27.30%; Fluid finds only two voices on that
excerpt. Full meeting DER is Pyannote 22.47%, Fluid 23.91%. Speed is demonstrated
on this fixture; equal or better speaker accuracy and Italian quality are not.

LS-EEND clean paced replay acknowledged all 1,920,000 samples with maximum
frame lag 1.024 s under the fixed 2 s budget. Full meeting acknowledged
16,789,675 samples; padding frames are recorded separately and output timestamps
are clipped to real audio. Initial long replay failed that coverage guard;
raw failure remains preserved. Tail-fixed binary SHA-256:
`38ce585ccfebea0afdf3b3dc2f5493e486a67bdf1c1607e60c91c28726d13aca`.
However full-meeting DER is **38.74%** (short 45.68%). Therefore **LS-EEND is not
enabled in the app**. Live speaker assignment remains open; throughput is not
an accuracy certificate. Whisper's microphone-owning CLI also needs a shared
PCM path before live integration, without a second capture.

File integration now includes the existing diarization trait, runtime selection,
settings, preflight diagnostics, post-update provisioning guard and native bundle
packaging. Auto selects a verified compatible Fluid bundle on macOS 15+;
Pyannote remains explicit and handles macOS 13–14/Windows. Existing MPS/CUDA
preferences keep Pyannote; they do not silently become ANE. Explicit ANE requires
Fluid/Apple Silicon. Missing/corrupt resources fail visibly and leave the base
transcript plus diarization failure metadata. No changes to personal profiles.

Models: 21 runtime files, 21,599,417 bytes; 25 files including provenance/license,
21,635,985 bytes. SDK and third-party licenses ship with the bundle. Bundle
manifest hashes the helper after signing and every packaged model/license file.
The real staged arm64 helper was signed ad hoc and verified on 2026-10-28:
`d49d65d82820870b8c98c385f42af158a3424a267d7a4ae8b5ba6c585005bba7`.
This is native resource staging, not a certified installer or notarization.

Current checks on the integrated snapshot: **489 Rust tests passed**, including
the real process timeout/cancellation contracts, platform routing and default/
explicit request error metadata. **Five native ASR/Live smokes were ignored**
because their external runtime/model/replay inputs were not supplied; they are
not PASS. Workspace Clippy with warnings denied passed. The earlier fixture
races, default-request metadata RED case and concurrent Rustdoc artifact failure
remain preserved separately; the current suite was rerun sequentially after fixes.
**199 frontend tests and TypeScript/Vite build passed with exact lockfile versions**;
previous shared node_modules had drifted and caused a failed app packaging attempt.
Old dependencies were preserved; current dependency tree is task-isolated on UltraDisk.
**11 native Swift unit tests and 38 packaging/model/protocol/workflow tests passed**;
one PowerShell-specific test was skipped on macOS.

A real isolated **debug** app (`com.sbobino.fluid-verification`, version 2.0.36)
was built on UltraDisk without updater artifacts, signed ad hoc and verified.
It includes the helper, verified models and licenses. This is a verification app,
not a release installer and not notarized. The packager signs a complete staged
copy and replaces the input app only after successful verification; failures at
all three codesign steps have regressions preserving original bytes. Building
the bundled helper requires macOS 15+, separately from the app's macOS 13 floor.

Standard GitHub macOS arm64/Intel and Windows validation is prepared but has
**not run for this diff**. Swift helper steps explicitly select installed Xcode
26.2 (Swift 6.2), rather than assuming the runner default. CI fixtures do not
certify microphone, desktop accessibility or native old-OS behavior. No release,
tag, promotion, merge or push occurred in this package. Current source manifest
must be regenerated after integration; the earlier snapshot is historical.

Remaining acceptance: whole app file-flow/UI proof, fresh screenshots/contact
sheet, coherent commit and hosted native CI, Intel/Windows/old-OS native proofs,
multilingual reference corpus and a Live
candidate that passes quality. The connected iPhone is reserved by the iOS agent
for the real ASR-to-speaker background regression; progress alone is not PASS.
