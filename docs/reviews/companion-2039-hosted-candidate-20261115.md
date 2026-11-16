# Companion 2.0.39 — hosted Release Candidate verification (DeepSeek, independent)

Date: 2026-11-15
Verifier role: independent Release Candidate verification sub-agent (read-only; no dispatch, rerun, push, tag, upload, promote, release edit)
Repo: `/Users/pietromastro/Documents/sbobino_tauri` (source frozen; only this document written)
Task output root: `/Volumes/UltraDisk/sbobino-companion-product-20261115/hosted-candidate-2039/{logs,evidence,assets,work}`

## Verdict

**FAIL — the candidate was never published.** Run 43728198825 ended `failure` at 15:48:04Z because the
native macOS **x86_64** build failed while hydrating the Pyannote runtime. `publish-candidate` was skipped,
so **no `v2.0.39` prerelease and no public candidate assets exist** (release API 404). There is therefore no
public artifact, manifest, proof JSON, or updater signature to audit; every public-proof scope is
**UNVERIFIED**, not PASS. The stable channel was not touched.

## Provenance (independently re-checked)

| Item | Value |
| --- | --- |
| Run | 43728198825, attempt **1**, `workflow_dispatch`, `.github/workflows/release.yml` |
| Head | `67b83f9cf4ebdbda4b0f579fce624eaaf1f9c488` (`main`) |
| Annotated tag | `v2.0.39` object `7a474098e6a7ea7c5349a6d8196a8da03b98d34d` → peels to `67b83f9` |
| Reviewed product pin | `153a86c194ca43b1172d570743e0adc0297980f0` |
| Product-scope diff `153a86c..67b83f9` | **empty** (`apps`, `crates`, `scripts`, `Cargo.toml`, `Cargo.lock`) |
| Non-product diff `153a86c..67b83f9` | `.github/workflows/release.yml` (`REVIEWED_PRODUCT_SHA` → `153a86c`), `docs/REPOSITORY_STATE.md` |
| Precondition CI | 43724115449 **success** at 15:02:25Z (frontend, rust, native ARM/Intel/Windows) — before dispatch 15:08:30Z |
| Updater pubkey | unchanged from `v2.0.36` (`evidence/pubkey-from-tag-67b83f9.txt`, keyid `dedce240f42a108d`) |

## Official job matrix (terminal)

| Job | Result | Window (UTC) |
| --- | --- | --- |
| Validate v2.0.39 candidate tag | success | 15:08:32–15:08:37 |
| Build native macOS aarch64 (`macos-15`) | success | 15:08:39–15:46:13 |
| Build native Windows x86_64 (`windows-2025`) | success | 15:08:39–15:44:20 |
| **Build native macOS x86_64 (`macos-15-intel`)** | **failure — ROOT** | 15:08:41–15:20:40 |
| publish-candidate | skipped | — |
| distribution-readiness (ARM/Intel) | skipped | — |
| windows-distribution-readiness | skipped | — |
| portability-smoke (ARM/Intel) | skipped | — |
| Synthetic ASR/live contract tests | skipped | — |
| intel-pyannote-parakeet-smoke | skipped | — |
| upload-validation-assets | failure — **cascade** | 15:46:15–15:46:21 |

## Root failure (deterministic, not infrastructure)

Job step: **Hydrate native Pyannote assets** → `../../scripts/setup_bundled_pyannote.sh --force`, substage
`Bundling portable Python native dependencies` (Conda env `conda-forge/osx-64 ncurses-6.6-hcc0dc9a_1`):

```
dependency '…/python/x86_64-apple-darwin/lib/libtinfo.6.dylib' of
'…/python/x86_64-apple-darwin/lib/libncurses.6.dylib' resolves to
'…/python/x86_64-apple-darwin/lib/libtinfo.6.dylib' but that file does not declare the
reference as an install name (declared: @rpath/libtinfo.6.dylib); refusing an unverifiable rewrite
##[error]Process completed with exit code 1.
```

The fail-closed rewrite guard in `sbobino_desktop/scripts/lib/pyannote_portable_runtime.py` accepts an
absolute dependency only when the target file itself declares that exact absolute path as an install name.
In the Intel conda layout `libncurses.6.dylib` links `libtinfo.6.dylib` by absolute path while
`libtinfo.6.dylib` declares `@rpath/libtinfo.6.dylib`, so the guard refuses by design. ARM hydration passed
the same guard, so this is a **platform-specific, reproducible packaging defect of the Intel Pyannote
runtime path**, not a flake. The guard was introduced for 2.0.38 (commit `6424139`); it is absent in
`v2.0.36`/`v2.0.37`, and the last fully green release run predates it (2026-10-12). This run is the first
hosted Intel hydration exercising it.

Cascade, clearly separated: `upload-validation-assets` failed only on
`actions/download-artifact` (`Artifact not found for name: distribution-readiness-aarch64`), which is a
direct consequence of the skipped validation jobs — not an independent defect. `publish-candidate` and all
five validation jobs were skipped by `needs`.

No cancellation was required or performed: the run was already terminal. No retry, rerun, dispatch, push,
tag, upload, promotion or release edit was made.

## Release state at end

- `v2.0.39`: **absent** (`GET /repos/pietroMastro92/Sbobino/releases/tags/v2.0.39` → 404); no draft, no assets.
- Latest stable: **`v2.0.36`** (id 410558977, `prerelease=false`), 28 assets whose **ids, sizes and digests are
  byte-identical to the pre-dispatch snapshot** (`evidence/stable-2.0.36-release-at-start.json`,
  `…-at-end.json`). No candidate promotion.

## Unverified scopes (explicitly not PASS)

Asset digests vs downloaded SHA-256; the seven public proof JSONs (release-readiness, ARM/Intel/Windows
distribution readiness, windows-gui-smoke, ARM/Intel portability smoke); the four operational manifests and
`release-notes.md`; `latest.json` platform URLs/signature embedding; the three updater minisign signatures;
canonical `com.sbobino.desktop` / `2.0.39` / minOS `13.0` / arch inside archives; DMG↔tar executable identity;
public Pyannote ARM/Intel portability audit; speech-runtime arch audit; Windows GUI proof and CPU-only live
preflight rejection; Intel expected realtime rejection.

Local DMG mount: **not attempted** (this environment blocks mounts) — disclosed for completeness; it was moot
because no candidate DMG was ever published.

## Verifier tooling (prepared, unused on assets)

Independent verifiers retargeted to 2.0.39 in `evidence/` (2038 originals preserved in
`evidence/templates-2038-originals/`): `verify_candidate_assets.py`, `verify_signatures.py`
(minisign double-encoded Tauri form, self-tested green), `verify_runtime_arch.py`,
`verify_pyannote_public_assets.py` (strict_closure=false + zero forbidden absolute/host references),
`verify_dmg_identity.py` (rewritten to **no local mount**: UDIF trailer + tar bundle identity).

## Next root action

1. Fix the Intel Pyannote hydration guard for the `libncurses.6 → libtinfo.6 (@rpath id)` pair (e.g. accept an
   inside-payload resolution whose target id is `@rpath/<basename>` before rewriting to `@loader_path`, or
   normalize the conda `libtinfo` id earlier in hydration), without weakening the anti-basename-collision rule.
2. Re-run the local Intel hydration gate for the packaged runtime, then decide (root) whether a new patch
   candidate tag is required — this run cannot be retried and `v2.0.39` has no published artifacts.
3. Only after a fully green hosted run: audit public assets, then promotion **dry-run**. No promotion was
   performed here.

Evidence: `evidence/failure-analysis.json` (machine-readable, with SHA-256 of every artifact),
`logs/run-watch.log`, `logs/run-43728198825-log-failed.txt`, `logs/hydrate-x86_64-step.log`.
