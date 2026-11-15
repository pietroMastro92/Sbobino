# Companion — independent adversarial review of the local packaging repair (2.0.38)

Date: 2026-11-15 · Reviewer role: read-only review sub-agent (no sub-agents, no commits, no dispatch)
Repo: `/Users/pietromastro/Documents/sbobino_tauri` @ `31d832b7083ba86588774c0230a9bf7d5996d5a7` (`main`)
Hold: [`NO-GITHUB-DISPATCH.md`](../../../Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/NO-GITHUB-DISPATCH.md) — respected. Read-only `gh run view` log inspection used; nothing was dispatched, re-run, pushed, tagged, uploaded or bumped.

Scope of this review: the **actual working source** of the Python/shell packaging change and the claimed local proof. Rust (`companion.rs`, `fluid_diarization_runtime.rs`) was not re-reviewed per mandate; only the *consumer* interaction with `python.o` was checked.

---

## 1. Verdict

**No P1 release blocker found in the Python/shell lane.** The three defects the repair document claims to fix are real, correctly diagnosed, and fixed in the direction the evidence supports. The previously failing hosted step is reproduced locally with the identical message, and the fixed module reproduces the described behaviour on the real python.org payload.

Three P2 findings are proven with executable counterexamples; all are small, local, and none of them blocks the *next* ARM release attempt:

| # | Severity | Finding | File:line |
| --- | --- | --- | --- |
| P2-1 | latent correctness | Same-basename host dylib collision is **not** fail-closed inside one fixpoint pass; two different dylibs silently collapse onto one copy and the audit reports 0 violations | `scripts/lib/pyannote_portable_runtime.py:1540-1556` |
| P2-2 | destructive safety | `replace_runtime_dir` guard is lexical only: an intermediate `python` symlink lets `rm -rf` delete (and `rsync` write) **outside** the vetted resource root | `scripts/setup_bundled_pyannote.sh:184-199` (+ `:748`) |
| P2-3 | guard/claim | Local gate's "work dir must live outside the checkout" check is a no-op for a work dir that does not exist yet — i.e. the normal first run | `scripts/local_packaging_gate.sh:116` vs `:120` |

Plus one evidence-boundary note (§6) and four minors (§5). "Green 82 tests" is **not** treated as "green release": two of the destructive-path "tests" are source-text greps that cannot fail on P2-2 (§5).

---

## 2. Reviewed source (sha256, working tree)

```
d0bed6d7243ad873399d9f6fd7d7e54b98385f89806b626151dd88b7215702c8  sbobino_desktop/scripts/lib/pyannote_portable_runtime.py
77e9c9bb598ab15905b25cc43e4a9f7bf3ce1e38843b7fb6165f98be973ba585  sbobino_desktop/scripts/setup_bundled_pyannote.sh
d706df42533328c6fefaee612ad4a11062f9aecdbfbfedcd8329647752a3d1bd  sbobino_desktop/scripts/local_packaging_gate.sh
73d4e37d7147d001be02979d3e8cd21a43c70cf994d8155c8f4b0da9484fe61d  sbobino_desktop/scripts/release_readiness.sh   (root integration diff, §7)
86dec509c2441b39316a41bd076e8cd7b5bea8c7a0848fb7cbdf4bf98d3f2693  sbobino_desktop/scripts/test_pyannote_portable_runtime.py
6c32540dc6d43c4c1b80a32f4410beab8478edfb7ab8546e8041eacd5587337c  sbobino_desktop/scripts/test_setup_pyannote_staging.py
f58ae438061c8d06cc4e84f91684e6931251e05b87b91b64e05a206488527ce8  HEAD baseline module (git)
```

The first four hashes match `…/packaging/evidence/environment.txt` for the files it lists, so the evidence was produced from the source reviewed here (no stale-evidence mismatch). All scratch for this review lives in
`/Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/independent-review/` (mount verified: `/dev/disk5s1 … 61 GiB avail`). `HOME` untouched; no payload was cloned, re-hydrated or re-packaged.

---

## 3. P2-1 — the basename-collision guard does not fire within a fixpoint pass

**Claim under test** (repair doc §4, module docstring `_sanitize_pass`): "collision of basename between two files with **different realpaths** ⇒ fail-closed; the same file reached via `opt/…` and `Cellar/…` is the same realpath and passes."

**Mechanism.** `_sanitize_pass` (`pyannote_portable_runtime.py:1537`) embeds per client:

```python
1540   if target not in embedded_planned:
1541       embedded_planned.add(target)
1542       source_real = os.path.realpath(source)
1543       previous = embedded_sources.get(str(target))
1544       if previous is not None and previous != source_real:
1545           raise SystemExit("cannot embed ... different files with the same basename")
1547       embedded_sources[str(target)] = source_real
```

The realpath comparison is **inside** `if target not in embedded_planned`. The second client that wants the same basename in the *same pass* skips the whole block, so the collision is never compared; it is only detected when the second source appears in a **later** pass (where `embedded_planned` has been reset).

**Executable counterexample** (`independent-review/expB`, synthetic, task-owned):

```
hostrootA/lib/libcollide.dylib  ->  int collide_value(void){return 1;}
hostrootB/lib/libcollide.dylib  ->  int collide_value(void){return 2;}
payload/bin/client_a  links hostrootA/lib/libcollide.dylib
payload/bin/client_b  links hostrootB/lib/libcollide.dylib
```

with `HOST_PREFIXES` extended in memory with the two synthetic roots (the same white-box technique the shipped suite uses in `HostManagedEmbeddingTests.patched_prefixes`):

```
before:            a=1  b=2
SANITIZE RETURNED (no fail-closed): passes=1,
  embedded = ["lib/embedded-dylibs/libcollide.dylib"]        # one copy only
  client_a -> @loader_path/../lib/embedded-dylibs/libcollide.dylib
  client_b -> @loader_path/../lib/embedded-dylibs/libcollide.dylib   # wrong library
after:             a=1  b=1                                   # expected 1 / 2
audit after:       violations: []  warnings: []               # fully silent
```

The cross-pass path *does* fail closed (`independent-review/expB2`: `client → hostA/collide + hostA/wrap`, `wrap → hostB/collide` gives `SystemExit: cannot embed '…/hostrootB/lib/libcollide.dylib': '…' was already embedded from '…/hostrootA/…'`). So the invariant is real but pass-order dependent.

**Consequence.** A payload whose two clients reach two *different* host dylibs with the same basename in the same pass ships a runtime that loads the wrong library, and **neither the audit nor the signature checks can see it** (dependencies resolve inside the payload). Failure mode at user runtime: wrong version of a library / missing symbol — the class of bug that is expensive to diagnose after release.

**Does it threaten the next ARM release?** No. I measured the real payload (`gate-final/output/pyannote/…`): 71 files carry an `MH_DYLIB` slice, **0** universal dylibs have per-arch differing ids, and on the python.org + `/Library/Frameworks` shape the embed sources are `libssl.3` / `libcrypto.3` / `libsqlite3` / `liblzma.5` / `libmpdec.4` — all distinct basenames. It is a latent invariant breach, not an imminent one.

**Smallest correction.** Hoist the comparison out of the membership test:

```python
source_real = os.path.realpath(source)
previous = embedded_sources.get(str(target))
if previous is not None and previous != source_real:
    raise SystemExit(...)
if target not in embedded_planned:
    embedded_planned.add(target)
    embedded_sources[str(target)] = source_real
    ...
```

Regression test that fails today: two compiled dylibs, identical basename, different host dirs, two clients in one payload; assert `SystemExit` (or assert the two clients still print different values after a *successful* run).

---

## 4. P2-2 — `replace_runtime_dir` guard is lexical; an intermediate `python` symlink escapes it

**Claim under test** (repair doc §5): "`replace_runtime_dir()`: the runtime `rm -rf` goes through a fail-closed guard (absolute path, not `/`, inside `RESOURCE_ROOT`)."

**Mechanism.** `setup_bundled_pyannote.sh:191-199` validates the target **as a string** and then removes it:

```bash
195  if [[ -z "$root" || "$root" != /* || "$target" != "$root"/* ]]; then … return 1; fi
199  rm -rf "$target"
```

`$target` is `"$RESOURCE_ROOT/python/$TARGET_TRIPLE"` (`:748`). `rm -rf` resolves every component except the last one, so if `$RESOURCE_ROOT/python` is a symlink to some other directory, the lexical test passes and `rm -rf` deletes `…/aarch64-apple-darwin` **outside** `$RESOURCE_ROOT`. The following `mkdir -p "$(dirname "$RUNTIME_DIR")"` + `rsync -a "$STAGE_RUNTIME_DIR/" "$RUNTIME_DIR/"` (`:955-956`) then write the whole 400 MB payload through the same symlink.

**Executable counterexample** (`independent-review/expA`): the function was extracted verbatim (`sed -n '184,200p'`) and driven with a task-owned sentinel:

```
vetted_root/python -> ../outside/precious
target (lexical) = …/vetted_root/python/aarch64-apple-darwin     # inside vetted_root
guard exit status = 0
after: outside/precious/  -> KEEP.txt            # parent survives
       outside/precious/aarch64-apple-darwin/    -> deleted (SENTINEL.txt gone)
```

**Consequence.** With the default tracked root (`…/resources/pyannote/python` is a tracked directory, so CI is safe) this cannot fire. It fires for any operator who points `--resource-root`, `SBOBINO_PYANNOTE_RESOURCE_ROOT` or a local cache link at a tree whose `python` entry is a symlink — exactly the kind of disk-pressure workaround this repo's 873 MB asset invites. Loss is one triple directory of a *non-payload* tree, not user data, but it is silent and it precedes a write of the same shape.

**Smallest correction.** Canonicalise before deciding, and delete the canonical path:

```bash
local parent_real root_real
parent_real=$(cd "$(dirname "$target")" 2>/dev/null && pwd -P) || { echo "refusing …" >&2; return 1; }
root_real=$(cd "$root" 2>/dev/null && pwd -P) || { echo "refusing …" >&2; return 1; }
[[ "$parent_real" == "$root_real"/* ]] || { echo "refusing …" >&2; return 1; }
rm -rf "$parent_real/$(basename "$target")"
```

**Note on the existing tests.** `test_runtime_directory_removal_is_guarded` (`scripts/test_setup_pyannote_staging.py:229-234`) only asserts three source substrings; it cannot detect this. A behavioural test with the symlink sentinel above (or a `--dry-run`-style hook) is the only seam that catches it. Same for `test_cleanup_vets_the_stage_directory_before_removing_it` (`:264-269`), which is also a grep.

---

## 5. P2-3 and minors

**P2-3 — gate work-dir check is a no-op on first run.** `local_packaging_gate.sh:116`

```bash
116  if [[ "$(cd "$WORK_DIR" 2>/dev/null && pwd -P)" == "$REPO_ROOT"* ]]; then  # FAIL "must live outside the checkout"
120  mkdir -p "$WORK_DIR" || …
```

For a work dir that does not exist yet, `cd` fails, the substitution is empty, the test is false and the gate proceeds to create it — **inside the checkout** if that is what was passed. Reproduction with the exact expression (`independent-review/expD`):

```
ACCEPTED …/repo/sbobino_desktop/new-work-dir     # does not exist yet, inside the checkout
REFUSED  …/repo/sbobino_desktop/new-work-dir     # same path after mkdir -p
```

Consequence is contained (the destructive surface stays under `$WORK_DIR/output` and `$WORK_DIR/tmp`), but the "work dir outside the checkout" property that the evidence row relies on is not enforced on the normal first run. Smallest correction: resolve the deepest existing ancestor (the same helper shape as `resolve_resource_root`) and compare that, *before* `mkdir`. A second, smaller issue in the same line: the prefix comparison has no path-boundary, so `/…/repo-other` would be refused (safe direction, cosmetic).

Minors (no action required before the next attempt):

1. `local_packaging_gate.sh:388-390` — `--keep` has no effect: nothing is ever deleted, and the "Hydrated output kept for inspection" line prints only on failure. Either implement it or drop the flag/message.
2. `local_packaging_gate.sh:300-303` — the PASS detail `"0 violations"` in `verify.signatures.*` is a hardcoded label; the measured numbers are printed next to it, so no misleading evidence, but the label is not derived.
3. `local_packaging_gate.sh:187-192, 306-316` — `python3.11` and `aarch64-apple-darwin` are hardcoded, so `--python-version` other than 3.11 would fail the layout check. Consistent with preflight forcing `arm64`, worth a guard/message.
4. Repair doc §7 says "27 righe PASS"; `gate-summary.txt` has **26 PASS + 1 INFO**. Cosmetic, but the summary is the machine-readable artifact.

---

## 6. What I actually reproduced, and what stays unverified

### Reproduced here (independent of the author's logs)

| Check | Result |
| --- | --- |
| Hosted failure is the claimed one | `gh run view 43692826685` (log-failed, read-only): ARM job, `Hydrate native Pyannote assets`, `2026-11-15T08:15:22Z`: `command failed: /usr/bin/install_name_tool -id @rpath/python.o … (exit 1)` / `… is not a Mach-O file`. Nothing else failed before it (the step started at 08:14:09). |
| Local RED is the same failure | `run-p-red/logs/hydrate.log:298-299`: identical command, identical message. |
| `python.o` semantics on the real file | fat32 big-endian, 2 slices, both `dec0170b` LLVM-bitcode wrappers (`x86_64`, `arm64`); `is_macho False`; `read_install_names == []`; `otool -D` diagnostics captured by `otool_diagnostics` and never parsed as ids; payload copy byte-identical to the official pkg (`d9e9a239…2189ed`). |
| Per-slice classification on the real payload | `scan_macho_payload` on `gate-final/output/…/aarch64-apple-darwin`: `macho 447, non_macho 1, mixed 0, invalid 0` in 12 s; 100 fat32 Mach-O files, every slice filetype correct (`execute` / `dylib` / `bundle`); `lipo -archs` order matches container order for all of them. |
| Intel query mode | real reader under `arch -x86_64 /usr/bin/python3` on a real fat dylib reads both slices (`ids: ['@rpath/libpython3.11.dylib']`, deps resolved), and on the bitcode container `is_macho False`, `ids []`. Rosetta present on this host. |
| Hosted Intel failure shape confirmed | `gh run view 43692823687` (job `Native Rust tests (x86_64-apple-darwin)`, step `Verify native Pyannote portability tooling`): `AssertionError: Lists differ: ['/build/x86_64/leaf.dylib'] != [arm64…, x86_64…]` — the bare multi-arch dump did report one slice on `macos-15-intel`, so the diagnosis is not speculative. |
| No fail-open regression of the audit | pre-fix module vs working-tree module, `audit` on the **tracked 2.0.36 asset** (873 MB, 29 192 files): identical results — 434 binaries, 423 signed, **134 violations** (`host_rpath 2, non_portable_install_name 28, absolute_rpath 52, personal_path_reference 52`), 158 `unresolved_rpath_dependency` + 11 `unsigned_macho`. Violation **and** warning sets are bit-for-bit equal; nothing new, nothing lost. Matches repair doc §7. |
| Targeted unit tests on the reviewed hashes | `MachOContainerParsingTests NonMachOContainerTests HostArchitectureQueryTests HostManagedEmbeddingTests` → **22 tests OK** (3.5 s); `ResourceRootContractTests` → **5 tests OK**. (Not the full 82: no need to re-run the suites whose code I did not dispute.) `git status` unchanged afterwards; tracked resources untouched. |
| Read-only promises | Hydration wrote only to `--resource-root` on UltraDisk; `resources/pyannote` unchanged (`git status` clean for it); no cache/profile mutation; review scratch confined to `independent-review/`. |
| Rust consumer cannot be tripped by `python.o` | `pyannote_native_binary_paths` (`crates/infrastructure/src/lib.rs:3973-3978`) filters to `.so`/`.dylib`, so `validate_portable_pyannote_native_dependencies` never opens it. |
| Packaging guard is load-bearing | `package_pyannote_asset.sh:39` runs the shared `audit --expect-arch` on the source dir before zipping, so a residual violation aborts the release, not just the review. |

### Not verified (do not read these as PASS)

1. **Intel (x86_64) runtime payload.** No Intel host. Only the reader contract is proven. The `macos-15-intel` `otool` behaviour itself was confirmed from the CI log, not reproduced here.
2. **Windows.** Untouched by this lane.
3. **python.org payload + embed-from-`/Library/Frameworks` on the full payload.** The local fixture redirects the vendored framework dylibs to `@loader_path` paths, so the gate run reports `copied 0 dylib(s)`: the *production ARM* embedding branch was never exercised on a full python.org payload. That branch is covered by (a) `HostManagedEmbeddingTests`, which I re-ran green on the final module hash with real dylibs, a 2-pass fixpoint and a real relocation/`dlopen`; (b) `run-h-green` (Homebrew, `copied 5 dylib(s)`, `passes ≥ 2`, AUDIT OK) — but that log is **10:35**, the module mtime is **11:02**, and the claim "the only later change touched `inventory_macho_metadata`" cannot be verified from a dirty tree (no intermediate copy exists). The gate itself re-ran full hydration at 11:07 on the final hash, but on the no-embed branch.
4. **Hosted toolchain.** Local Xcode 27.2 / `cctools-1040` vs runner Xcode 16.4 / macOS 15. The fix no longer depends on the bare multi-arch `otool` dump, but `install_name_tool`/`codesign`/`lipo` behaviour on 16.4 is unchanged and unmeasured. The hosted ARM job has never executed anything past the hydration step.
5. **Closure strictness.** 141 `unresolved_rpath_dependency` remain warnings; the strict closure is not green and is not claimed. The functional entrypoint (1 turn, in place and after relocating to a path with spaces) is the only evidence those references resolve at runtime.
6. Root's native `aarch64` app build, signature result and readiness run are root's evidence, not mine.

---

## 7. Root's `release_readiness.sh` integration diff (reviewed on request)

Reviewed statically at `73d4e37d…` (no execution):

* `PYANNOTE_RESOURCE_ROOT=${SBOBINO_PYANNOTE_RESOURCE_ROOT:-"$DESKTOP_DIR/src-tauri/resources/pyannote"}` (`:48`) feeds **both** `setup_bundled_pyannote.sh --resource-root` (`:588`) and the two `package_pyannote_asset.sh` roots (`:592`, `:596`), so hydration and packaging cannot disagree. No other place in the script reads the tracked resource path (`:66` resolves the *built app bundle*, correctly independent).
* Because the flag is now always passed, the local readiness run exercises `resolve_resource_root` (absolute, non-`/`, existing-entry-must-be-dir, personal-path guard on requested **and** resolved form, canonicalisation) and — importantly for §4 — the guarded `replace_runtime_dir`. Default value is byte-identical to the old path, so hosted behaviour is untouched (`release.yml:96` still calls the script with no flag; that path is unaffected by this diff).
* The four `mktemp` calls are now template-based under `TMPDIR`/`TEMP_DIR`, consistent with `test_every_mktemp_uses_an_explicit_template`. `bash -n` passes; no contract test reads this file's text (only the `"gate": "release_readiness.sh"` metadata string), so nothing goes red from the rename.
* Two small notes: (a) `${TMPDIR:-/tmp}` is not required to be absolute, so a relative `TMPDIR` would produce a relative `TEMP_DIR` and a `cleanup` `rm -rf` evaluated from a different CWD — the same gap `setup_bundled_pyannote.sh:116-119` already rejects; (b) `release_readiness.sh` now requires the new `--resource-root` flag, so it must be committed **together** with `setup_bundled_pyannote.sh`.

---

## 8. Recommended smallest corrections (ordered, all local)

1. `pyannote_portable_runtime.py:1540` — hoist the realpath collision check out of `if target not in embedded_planned` (P2-1) + one regression test.
2. `setup_bundled_pyannote.sh:195` — compare canonicalised parent against canonicalised root before `rm -rf` (P2-2) + the symlink-sentinel test that replaces the two grep "tests".
3. `local_packaging_gate.sh:116` — canonicalise the deepest existing ancestor before the outside-checkout comparison, before `mkdir -p` (P2-3).
4. Optional, same edit: make `--keep` real or delete it; require an absolute `TMPDIR` in the readiness diff.

No new tests, gates or CI work are proposed here: these are the only items that change code, and the next hosted measurement should happen after root's native build + this review pass, not before.
