# Companion Intel local Conda repair — 2026-11-15

> Historical implementation checkpoint: root subsequently closed the stdout blocker, moved signature refusal into read-only planning, and completed a fresh final-source Intel hydration plus ARM/Intel verification. Final integration and source-bound results: `companion-intel-conda-root-integration-20261116.md`.

Local repair of the Intel (`x86_64-apple-darwin`) Pyannote hydration that failed
in hosted candidate `43728198825`. **No commit, stage, push, tag, dispatch,
rerun, upload or promotion was performed.** `v2.0.39`
(`7a474098e6a7ea7c5349a6d8196a8da03b98d34d`) is untouched, version `2.0.39` is
unchanged, and the renewed NO-GITHUB hold in
`/Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/NO-GITHUB-DISPATCH.md`
was respected for the whole run. Nothing was created outside the task volume
`/Volumes/UltraDisk/sbobino-companion-product-20261115/intel-conda-repair`; `$HOME`,
personal environments, base conda, Keychain, devices, SDKs, firewall and other
agents' directories were not touched.

**Outcome in one paragraph.** The hosted Intel failure is fixed and proven by a
complete local Intel hydration: the runtime hydrates, sanitizes, audits with
**0 violations**, is byte-idempotent, packages at 755 MiB, both import orders
work, and `pyannote_diarize.py` produces a correct one-speaker result in place
and after relocation to a path with spaces. Three decisions are now root's:
(1) accept the adhoc-signature repair rule (§5), (2) approve expanding ownership
to `scripts/pyannote_diarize.py` for the stdout-purity fix that makes the gate
green (§9.3 — the JSON is correct but three Intel MKL warning lines precede it
on stdout, which the app's own whole-stdout `serde_json` parser rejects), and
(3) pick the next patch version/pins after reviewing this result.

Repo: `/Users/pietromastro/Documents/sbobino_tauri`, HEAD
`67b83f9cf4ebdbda4b0f579fce624eaaf1f9c488` (= `origin/main`).

Ownership used: `sbobino_desktop/scripts/lib/pyannote_portable_runtime.py`,
`scripts/test_pyannote_portable_runtime.py`,
`scripts/test_pyannote_review_regressions.py`,
`scripts/local_packaging_gate.sh`, plus this report.
`scripts/test_setup_pyannote_staging.py`, the Rust/iOS sources, the workflow and
the packaging setup scripts were **not** modified (no gate regression needed
them).

## 1. Failure and root cause (measured, not inferred)

Hosted step `Hydrate native Pyannote assets` (job `132854676167`, log
`hosted-candidate-2039/logs/hydrate-x86_64-step.log:709`) aborted with:

```
dependency '…/python/x86_64-apple-darwin/lib/libtinfo.6.dylib' of
'…/python/x86_64-apple-darwin/lib/libncurses.6.dylib' resolves to
'…/lib/libtinfo.6.dylib' but that file does not declare the reference as an
install name (declared: @rpath/libtinfo.6.dylib); refusing an unverifiable
rewrite
```

`resolve_internal_dependency()` required, for every absolute non-system
dependency, that the resolved target declare the same absolute string as one of
its `LC_ID_DYLIB` values. That requirement is invalid for the conda layout: the
conda-forge osx-64 `ncurses 6.6 hcc0dc9a_1` package ships
`LC_ID_DYLIB @rpath/libtinfo.6.dylib` while its client keeps the absolute
in-prefix path `<prefix>/lib/libtinfo.6.dylib` written by conda's own prefix
rewrite.

`LC_LOAD_DYLIB` is the path dyld opens; `LC_ID_DYLIB` is an identity used by the
link editor and by loader-side matching. Nothing in dyld requires the two to be
textually equal for an absolute load command, and conda documents loader-relative
identities as the normal relocatable form.

A **second** blocker appeared only once the first was fixed: conda's prefix
rewrite also replaces build-prefix *string data* inside Mach-O files
(`/Users/runner/miniforge3/conda-bld/...` → the local prefix), which invalidates
the embedded **adhoc** code signature of 15–16 package binaries without touching
a single load command. The audit correctly reported those as
`invalid_signature` violations, so no Intel payload could have passed the gate
even after the dependency fix.

## 2. Policy verification before changing either guard

Primary sources:

* conda-build, *Using shared libraries*
  <https://docs.conda.io/projects/conda-build/en/stable/resources/use-shared-libraries.html>
  — absolute links "work only on a system where that exact path exists";
  `@rpath` / `@loader_path` / `@executable_path` are the relocatable forms and
  are what conda-build rewrites with `install_name_tool`.
* `man 1 dyld` on this machine — fallback searching is described per recorded
  install path; `DYLD_LIBRARY_PATH` and the fallback variables are applied to
  the library a load command names.
* Local **loader** proof, not a string comparison:
  `/usr/bin/arch -x86_64` + `dlopen("<prefix>/lib/libncurses.6.dylib")` succeeds
  and resolves `tgetent` from the reexported `libtinfo.6.dylib` whose identity is
  `@rpath/libtinfo.6.dylib`
  (`evidence/loader-absolute-dep-portable-id.txt`).
* Signature fact measured directly: the conda package-cache copy of
  `libglib-2.0.0.dylib` verifies (`codesign --verify --strict` exit 0), the
  installed copy is byte-different exactly at the replaced build prefix
  (offset 701834, `/Users/runner/…` → `/Volumes/…`) and fails with
  `invalid signature (code or signature have been modified)`; the signature blob
  itself is identical, so nothing re-signed it.

Root's `evidence/policy-research.md` evaluated five remedies. Implemented:
direction 1 (distinguish an exact existing contained path from a guessed suffix
reconstruction). Directions 2–5 (rewrite before planning, conda-pack, swapping
the Intel stack, special-casing ncurses) were not used.

## 3. Real RED on the actual conda package

Created with the existing base conda executable only (used read-only; no
install/update in base; caches and prefixes task-owned):

```
CONDA_SUBDIR=osx-64 conda create -y -p <task>/red-ncurses-hcc0dc9a_1 \
  --no-deps --override-channels -c conda-forge ncurses=6.6=hcc0dc9a_1
```

`conda-meta` record:

| field | value |
| --- | --- |
| package | `ncurses 6.6 hcc0dc9a_1` (build number 1) |
| url | `https://conda.anaconda.org/conda-forge/osx-64/ncurses-6.6-hcc0dc9a_1.conda` |
| sha256 | `12c1d676b9a0e8109b576672519312c5428308f3f3d7706f8717e2f536d5d9e7` |
| md5 | `88a79390f87c8f3334b9999a892e78d4` |
| size / subdir | 834865 bytes / `osx-64` |

Measured load commands (both thin `x86_64`, `lipo -info` = non-fat):

```
libncurses.6.dylib  LC_ID_DYLIB   @rpath/libncurses.6.dylib
                    LC_LOAD_DYLIB <prefix>/lib/libtinfo.6.dylib   (reexport)
libtinfo.6.dylib    LC_ID_DYLIB   @rpath/libtinfo.6.dylib
```

The unmodified helper on that real prefix reproduces the hosted message
(`logs/red-sanitize.log`, exit 1, `evidence/red-sanitize.json`). No second full
hosted-style hydration was spent on the RED: the hosted log plus this exact
package-level reproduction are the same failure.

## 4. Fix 1 — dependency provenance

`resolve_internal_dependency()` now separates two provenances and relaxes only
the first:

* `inside` — the recorded absolute path exists **inside the payload at exactly
  that path**. That is the file dyld would open; no tail reconstruction happens.
  The target must still be a genuine `MH_DYLIB` in every slice, must not be a
  mixed Mach-O/non-Mach-O container, must declare at least one install name, and
  every declared identity must be portable **or** scheduled for the `@rpath`
  rewrite in the same pass (`planned_ids`). Only the "identical absolute string"
  demand is gone.
* `suffix` — the recorded path does not exist on this machine, so the target was
  reconstructed from the tail. Provenance is weaker, so the original rule stays:
  the target must declare the recorded reference as an install name.

Preserved unchanged: ambiguity refusal, basename-collision refusal, symlink
escape refusal, host-embedding refusal with basename-collision tracking,
unreadable/mixed container refusal, plan-before-mutate (nothing is written until
the whole tree plans), `MAX_SANITIZE_PASSES` convergence, and the whole audit.

## 5. Fix 2 — adhoc signatures invalidated by another tool

`sanitize_runtime()` now verifies **every** Mach-O it did not already modify and
repairs exactly one narrow case:

* verification fails with `invalid signature` / `code or signature have been
  modified`, **and** the recorded signature kind is `adhoc` → re-sign adhoc,
  verify again, and record the file under a new receipt key `repaired_signatures`;
* an unsigned file stays unsigned (audit keeps reporting `unsigned_macho` as a
  warning) — the sanitizer never signs code somebody else left unsigned;
* any other verification failure, or an invalid **non-adhoc** identity, aborts
  with `refusing to replace a real identity`.

This is a deliberate policy delta: without it the Intel branch cannot produce a
payload that satisfies the existing `invalid_signature` violation, and the
alternative (post-processing `codesign` in `setup_bundled_pyannote.sh`) is
outside this task's ownership. **It needs root's explicit sign-off.**

## 6. Measured GREEN

### 6.1 Isolated real package

Fresh prefix of the same package at a second task path (so the recorded absolute
dependency matches the prefix):

* sanitize: 2 Mach-O, 2 dependency rewrites, 0 rpaths, 0 ids, 2 signatures;
* after: `libncurses.6 → @loader_path/libtinfo.6.dylib`,
  `libncursesw.6 → @loader_path/libtinfow.6.dylib`;
* `audit --expect-arch x86_64`: **0 violations**, 15 `unsigned_macho` warnings;
* real loader proof on the relocated tree (path with a space): `dlopen`
  succeeds, `tgetent` resolved;
* a second `sanitize` on a copied, already sanitized tree is a byte-level no-op;
* measured negative that stays refused: sanitizing a **copy** of the prefix
  (load command still names the original prefix, therefore a suffix
  reconstruction) is refused with `does not declare the reference as an install
  name`.

### 6.2 Whole Intel runtime (rehearsal, before the final gate)

A standalone `conda create python=3.11 pytorch=2.9.1 ffmpeg=7 kaldi pkg-config
cmake pybind11` environment at a task prefix, then one uninterrupted
sanitize + audit:

* sanitize: 17 Mach-O, 18 rpaths, 8 install names, **3 dependency references**,
  17 signatures, **15 adhoc signatures re-signed**;
* audit: **0 violations**, 1154 Mach-O, 1013 `unsigned_macho` warnings and 2
  unresolved `@rpath` warnings (`strict_closure=false`);
* second sanitize: 0 changes, identical inventory `tree_sha256`.

The 15 damaged signatures were originally 15 audit violations; that is how
blocker 2 was found and measured before spending the full hydration.

## 7. Regression tests

`scripts/test_pyannote_portable_runtime.py` (+3 dependency-provenance tests,
+3 signature tests; every pre-existing negative assertion kept, no skips and no
allowlists added):

* `test_sanitize_accepts_a_contained_reference_with_a_loader_relative_identity`
  — conda shape: audit flags it, sanitize rewrites to `@loader_path/…`, audit is
  clean, the real loader loads the relocated dylib;
* `test_sanitize_refuses_a_contained_reference_to_a_non_dylib_target` — an
  `MH_EXECUTE` at an exactly contained path is refused, payload byte-identical;
* `test_sanitize_keeps_suffix_provenance_strict` — unique suffix match whose
  target declares `@rpath/…` is still refused;
* `test_sanitize_resigns_an_adhoc_signature_invalidated_by_another_tool` — real
  dylib, one string byte rewritten (no load-command change), signature verified
  invalid, repaired once and idempotent afterwards;
* `test_sanitize_refuses_to_replace_a_real_identity` — with a non-adhoc kind the
  run aborts and the damaged bytes stay untouched;
* `test_sanitize_does_not_sign_code_that_was_never_signed`.

`scripts/test_pyannote_review_regressions.py` (+1): the conda shape is trusted
in place and refused after a copy, with a real `ctypes` load after relocation.

Totals: **67 + 16 + 11** tests, green in a native `arm64` process and in a real
`x86_64` process (`/usr/bin/arch -x86_64 /usr/bin/python3`, fixtures compiled
`-arch x86_64`). The pre-existing stale-runtime negative
(`test_guard_rejects_stale_runtime_and_accepts_repaired_one`, the 2.0.36
host-`LC_RPATH` shape) and every ambiguity/collision/symlink negative still pass.

## 8. Local gate: measured target, real Intel branch

`scripts/local_packaging_gate.sh` no longer hardcodes `arm64`,
`python/aarch64-apple-darwin`, `lib/python3.11`, `arm64` or
`libpython3.11.dylib`. It now:

* derives `TARGET_TRIPLE`/`EXPECT_ARCH` from `uname -m` of the **actual process**
  and records `host_kernel_arch`, `target`, `expect_arch`, `process_mode`
  (`native` / `rosetta-translated` from `sysctl.proc_translated`), the gate
  interpreter with its `lipo -archs`, `python_version`, `conda_subdir`,
  `conda_pkgs_dirs` and the staging dir in `gate-summary.txt`;
* refuses a gate interpreter without a slice for the target (otherwise fixtures
  would be built for the other machine);
* on `x86_64` requires `conda`, exports `CONDA_SUBDIR=osx-64` (the local base
  resolver is an arm64 conda and would otherwise produce an osx-arm64 env) and
  keeps `CONDA_PKGS_DIRS`, `CONDA_ENVS_PATH`, `PIP_CACHE_DIR` and `TMPDIR`
  inside `--work-dir`, so base conda and personal environments are never
  written;
* runs the complete
  `setup_bundled_pyannote.sh --force --resource-root <work>` — the same pinned
  pipeline as the hosted step: `conda create` with `python=3.11`,
  `pytorch=2.9.1`, `ffmpeg=7`, `kaldi`, `pkg-config`, `cmake`, `pybind11`;
  `torchaudio 2.9.1` installed `--no-deps` from conda-forge; `torchcodec v0.8.1`
  compiled from source against that Torch; then the pip pyannote stack and the
  LFS model clone;
* adds `verify.idempotence` (a second `sanitize` on an independent copy must not
  change the inventory `tree_sha256`) and `verify.load_commands` (no load
  command may still contain the work/staging root, `$HOME`, `/Users/`,
  `/opt/homebrew`, `/usr/local/opt`, `miniconda`, `anaconda`);
* records `strict_closure=false` for the gate audit, the `--strict-closure`
  violation count separately as INFO, and the installed
  torch/torchaudio/torchcodec/pyannote/numpy versions.

## 9. Runs

### 9.1 Intel full hydration + verify (`/usr/bin/arch -x86_64 /bin/bash`, `--python /usr/bin/python3`)

Real pipeline, not a simulation: `setup_bundled_pyannote.sh --force
--resource-root <work>` under a real `x86_64` process, `CONDA_SUBDIR=osx-64`,
task-owned `CONDA_PKGS_DIRS`/`CONDA_ENVS_PATH`/`PIP_CACHE_DIR`/`TMPDIR`.

| step | result |
| --- | --- |
| preflight | `x86_64`, `rosetta-translated`, Python 3.11, conda 24.9.2, `CONDA_SUBDIR=osx-64`, ~40 GiB free |
| units (`x86_64` process) | 67 + 16 + 11 tests PASS |
| hydrate (conda create + torchaudio + TorchCodec v0.8.1 from source + pip pyannote + LFS model) | PASS, 1270 s |
| sanitize | 108 Mach-O, 73 rpaths, 32 install names, **3 dependency references**, 108 signatures, **16 adhoc signatures re-signed** |
| audit `--expect-arch x86_64` | **0 violations**, 1486 warnings (`strict_closure=false`); 285 closure violations reported separately as INFO |
| idempotence | byte-identical: 55546 files, `tree_sha256 5bd35870b58f9fe6…` (identical on the relocated copy) |
| load commands | 1445 Mach-O clean of build-root/host references |
| packager | `package_pyannote_asset.sh` 755 MiB `x86_64` zip, then removed |
| imports | both orders PASS (`pyannote.audio`→`torchcodec` and the reverse) |
| versions | torch 2.9.1, torchaudio 2.9.1, torchcodec 0.8.1, pyannote.audio 4.0.4, numpy 2.4.6 |
| audit after relocation to a path with spaces | **0 violations**, 1486 warnings |
| signatures | 244 Mach-O verified, 0 invalid |
| entrypoint | **FAIL**: the JSON is correct (one speaker turn) but three Intel MKL warning lines precede it on stdout — see 9.3 |

That run exited `2` at its very end because the gate script was edited *while
bash was still executing it* (bash reads a script incrementally). Every check
above had already completed and been recorded; the clean, frozen-source
re-verification is 9.1b.

### 9.1b Intel payload, frozen sources, gate `--verify-only`

`gate-intel-final`, `x86_64` (`rosetta-translated`) process, `--python
/usr/bin/python3`, on the payload produced in 9.1, with
`provenance: source hashes unchanged throughout this run`:

| check | result |
| --- | --- |
| layout (original, relocated) | PASS |
| audit `--expect-arch x86_64` (original, relocated) | **0 violations**, 1486 warnings, `strict_closure=false`; 285 closure violations as INFO |
| idempotence (original, relocated) | byte-identical, 55546 files, `5bd35870b58f9fe6…` |
| load commands (original, relocated) | 1445 Mach-O clean |
| packager | 755 MiB `x86_64` zip |
| imports (both orders, both trees) | PASS |
| versions | torch 2.9.1 / torchaudio 2.9.1 / torchcodec 0.8.1 / pyannote.audio 4.0.4 / numpy 2.4.6 |
| signatures | 244 verified, 0 invalid; `bin/python3` and `libpython3.11.dylib` INFO-unsigned |
| **entrypoint (original, relocated)** | **FAIL — stdout is not one JSON document** (9.3) |

So the gate ends `FAIL` for exactly one reason, twice: the diarization result is
correct but a native library writes three warning lines to stdout in front of the
JSON document. That is a product-side contract defect outside this task's
ownership, not a hydration, packaging, architecture, closure, signature or
idempotence failure.

### 9.2 ARM known-good payload, independent copy

The previously verified `2.0.39` ARM payload
(`local-release-repair/final-gate-2039/output/pyannote`) was copied to
`intel-conda-repair/arm-payload-independent-copy` and checked with the final
helper:

* `audit --expect-arch arm64`: **0 violations**, 447 Mach-O,
  151 warnings (`non_macho_container` 1, `unresolved_rpath_dependency` 141,
  `unsigned_macho` 9);
* `sanitize`: 0 Mach-O, 0 rpaths, 0 ids, 0 dependencies, 0 signatures,
  0 re-signed signatures; inventory `tree_sha256` unchanged
  (`e716d3efea807c8d…`, 44402 files);
* full gate `--verify-only` on that copy with the **final frozen sources**
  (`gate-arm-final`): PASS on both the original and the relocated tree — packager
  408 MiB, product entrypoint 1 speaker turn each, both import orders, 438
  verified signatures, 0 invalid, `byte-identical` idempotence
  (`e716d3efea807c8d…`, 44402 files) and `447 Mach-O clean of
  build-root/host load commands`.

So the relaxed dependency rule does not weaken the existing stale-`2.0.36`
asset audit: `test_guard_rejects_stale_runtime_and_accepts_repaired_one` still
rejects the host-`LC_RPATH` tree, and the known-good ARM payload is still
accepted byte for byte (re-checked with the final helper, section 9.4b).

### 9.3 Blocker 3 — product stdout contract (needs root approval, **not fixed here**)

Measured on the Intel payload:

```
$ …/bin/python3 /…/pyannote_diarize.py --audio-path probe.wav --model-path … --device cpu
Intel MKL WARNING: Support of Intel(R) Streaming SIMD Extensions 4.2 …
Intel MKL WARNING: Support of Intel(R) Streaming SIMD Extensions 4.2 …
Intel MKL WARNING: Support of Intel(R) Streaming SIMD Extensions 4.2 …
{"speakers": [{"speaker_id": "speaker_1", … "end_seconds": 7.16909375, …}]}
```

The diarization result is correct; the JSON is preceded by library output on
**stdout**. The app parses the whole stdout
(`crates/infrastructure/src/adapters/pyannote.rs:94`,
`serde_json::from_slice::<PyannoteOutput>(&output.stdout)`), and the hosted Intel
smoke does the same (`release_intel_pyannote_parakeet_smoke.sh:332`,
`json.loads(result_path.read_text())`). Any Intel host that prints that warning
(Rosetta on Apple silicon exposes SSE4.2 only; pre-Haswell Intel Macs lack AVX)
would fail diarization with a parse error. No environment variable suppresses
the message; measured `MKL_VERBOSE`, `MKL_DEPRECATION_WARNING` and
`MKL_ENABLE_INSTRUCTIONS` — all still print it.

`scripts/pyannote_diarize.py` is **outside this task's ownership**, so it was not
edited. Prepared, minimal, POSIX-only patch (Windows path unmeasured, so it stays
untouched):

```python
def claim_stdout_for_json():
    """Keep library output (MKL, NNPACK, logs) off the JSON channel.

    The app parses the whole stdout, so one warning line from a native
    dependency breaks diarization. Everything printed by the libraries is moved
    to stderr; the JSON document goes to the original stdout.
    """
    if os.name != "posix":
        return None
    handle = os.fdopen(os.dup(1), "w", encoding="utf-8")
    os.dup2(2, 1)
    sys.stdout = sys.stderr
    return handle

# in main(), before importing torch/pyannote:
_json_stdout = claim_stdout_for_json()
...
# instead of sys.stdout.write(json.dumps({"speakers": turns})):
if _json_stdout is not None:
    _json_stdout.write(json.dumps({"speakers": turns}))
    _json_stdout.flush()
else:
    sys.stdout.write(json.dumps({"speakers": turns}))
```

Ownership expansion request: `sbobino_desktop/scripts/pyannote_diarize.py`
(and, if root wants it, the same file is `include_str!`-embedded in the Rust
adapter, so applying it later requires the app rebuild root already owns).

### 9.4 Gate: signature classification, and the exact final-source runs

* `verify.signature.*` now mirrors the audit's own classification: an entirely
  unsigned third-party interpreter (`code object is not signed`) is INFO with
  an explicit note, while a present-but-broken signature is still FAIL. Conda's
  osx-64 `bin/python3` and `lib/libpython3.11.dylib` are unsigned by the
  distribution (the ARM python.org payload is signed and still PASSes).
* `verify.entrypoint.*` now distinguishes "stdout is not one JSON document"
  from "no speaker turn", so the evidence names the real defect (9.3) instead of
  a misleading turn count.

Frozen-source outcome: `evidence/source-freeze-final.json` equals the final
run's `source-before.json` and `source-after.json` (9.1b). The pre-hydration
freeze differed only in `local_packaging_gate.sh` — the two report-only gate
fixes above, applied after the hydration had started; no payload-affecting file
changed (`evidence/source-freeze-delta.md`).

## 10. Provenance

* Source freeze: `evidence/source-freeze-final.json` (sha256 of the nine gate
  inputs) equals the final run's `source-before.json` and `source-after.json`.
  The pre-hydration freeze and its one-file delta are in
  `evidence/source-freeze-before-final-gate.json` and
  `evidence/source-freeze-delta.md`; no payload-affecting file changed after the
  hydration.
* All artifacts, caches, prefixes, evidence and logs live under
  `/Volumes/UltraDisk/sbobino-companion-product-20261115/intel-conda-repair`;
  disk was checked before every expensive phase (28–43 GiB free, ≥ 20 GiB
  required by the Intel preflight).
* Host: macOS 27.2, Xcode 16.4 toolchain (`xcrun clang`), conda 24.9.2 (base,
  arm64, read-only), Rosetta `x86_64` processes.

## 11. Limits (honest)

* The Intel run is **Rosetta-translated on an arm64 Mac**, not native Intel
  hardware. It proves the x86_64 Mach-O payload, the real dyld loads under the
  x86_64 ABI, and the whole pipeline, but it is not a native-Intel
  certification. Rosetta exposes SSE4.2 only, which is exactly why MKL prints
  its deprecation warning here; most Intel Macs supported by macOS 13+ have
  AVX2 and would not print it, so the field probability of blocker 3 is lower
  than this run suggests — the contract violation it exposes is not.
* The gate ends `FAIL` while blocker 3 (9.3) is open. It cannot be closed
  truthfully from this task's ownership; a green gate now would require either
  editing the product script without approval or teaching the gate to tolerate a
  stdout contract the app itself does not tolerate.
* `strict_closure=false` everywhere: the 2 unresolved `@rpath` dependencies and
  the inactive torchcodec FFmpeg variants are reported, not fixed. The
  1486 audit warnings are dominated by `unsigned_macho`, which conda ships
  that way.
* The adhoc-signature repair is a policy change that needs root's decision (see
  section 5). The audit check itself is unchanged, so trees that are not passed
  through the sanitizer are still rejected.
* Inert, non-load-path residue: conda's prefix replacement leaves the local
  build prefix inside string data of the affected binaries. It is data, not a
  load command, so the load-command scans are clean, but it is visible in the
  payload and is disclosed here rather than hidden.
* No Rust/Tauri rebuild, no Cargo tests, no iOS work and no final integration /
  native app review: those stay with root, as instructed.
* No hosted run is proposed. The tag is immutable and the NO-GITHUB hold stands;
  the next patch version and any pinning decision belong to root after reviewing
  this result.
