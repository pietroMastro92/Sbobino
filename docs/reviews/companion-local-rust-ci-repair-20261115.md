# Companion local Rust/CI repair — 2026-11-15

Local-only repair of the Rust regressions that failed CI run
[43692823687](https://github.com/pietroMastro92/Sbobino/actions/runs/43692823687) and blocked the
2.0.38 candidate. No commit, stage, push, tag, workflow dispatch, upload or promotion was performed,
and the `NO-GITHUB-DISPATCH.md` hold stays in force.

| Field | Value |
| --- | --- |
| Repository | `/Users/pietromastro/Documents/sbobino_tauri` |
| Base commit | `31d832b7083ba86588774c0230a9bf7d5996d5a7` (2026-11-15 09:04:16 +0100, `ci: pin Desktop 2.0.38 candidate to reviewed packaging fixes`) |
| Branch | `main` (tracked tree clean at start; unrelated untracked files preserved) |
| Toolchain | `rustc 1.95.0 (59807616e 2026-04-14)`, `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`, `clippy 0.1.95 (59807616e1 2026-04-14)`, `rustfmt 1.9.0-stable` |
| Lane root | `/Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/rust` |
| `CARGO_HOME` | `/Volumes/UltraDisk/sbobino-companion-product-20261111/desktop/cargo-home` (existing, offline) |
| `CARGO_TARGET_DIR` | `/Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/rust/target` (8.7 GB, lane-local) |
| `TMPDIR` | `/Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/rust/tmp` |
| Cargo settings | `CARGO_BUILD_JOBS=1`, `CARGO_INCREMENTAL=0`, `CARGO_NET_OFFLINE=true`; `HOME` unchanged |

Frozen environment (`/Volumes/UltraDisk/.../local-release-repair/rust/env.sh`):

```sh
export TMPDIR=/Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/rust/tmp
export CARGO_HOME=/Volumes/UltraDisk/sbobino-companion-product-20261111/desktop/cargo-home
export CARGO_TARGET_DIR=/Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/rust/target
export CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 CARGO_NET_OFFLINE=true CARGO_TERM_COLOR=never
```

A lane-local `CARGO_TARGET_DIR` was chosen deliberately: the shared
`/Volumes/UltraDisk/sbobino-companion-product-20261111/desktop/target` only holds host-tree
`debug/` and `release/` output and has no `aarch64-apple-darwin/` subtree, so a `--target` build
could not reuse any of it, and sharing it would have put a build lock in front of the packaging
worker's concurrent Tauri build. The registry cache (`CARGO_HOME`) *is* shared, so nothing was
downloaded.

Files owned and changed by this lane (nothing else was touched):

| File | SHA-256 before | SHA-256 after |
| --- | --- | --- |
| `sbobino_desktop/crates/infrastructure/src/companion.rs` | `5602b14e7d72327be5bf8504582d6c85c3b4adaa6e72fd2c4c6f2d1193cf5420` | `c701747159ef174b2d2e272574a7295c952d79d3ef9dfdc321bab0e70a93c265` |
| `sbobino_desktop/crates/infrastructure/src/fluid_diarization_runtime.rs` | `ed8dcad62ffb9ae85441a7da817cadc18dd24516b0b9f340a861c3da2f2ff52e` | `ace61dfb9ebe6a21b03a9a4a403f6bd05b091c81a0346c3bf8348f613fb3776f` |

`git status --porcelain` (tracked) after the change shows only these two files plus the other
worker's Python/script edits, which were left untouched:

```
 M sbobino_desktop/crates/infrastructure/src/companion.rs
 M sbobino_desktop/crates/infrastructure/src/fluid_diarization_runtime.rs
 M sbobino_desktop/scripts/lib/pyannote_portable_runtime.py      (other worker)
 M sbobino_desktop/scripts/setup_bundled_pyannote.sh             (other worker)
 M sbobino_desktop/scripts/test_pyannote_portable_runtime.py     (other worker)
 M sbobino_desktop/scripts/test_setup_pyannote_staging.py        (other worker)
```

## 1. Failures reproduced

### 1.1 `clippy::redundant_pattern_matching` (macOS aarch64 + Linux lint gates)

CI (`Native Rust tests (aarch64-apple-darwin) / Lint native workspace`, 08:12:19Z, and `rust /
Clippy`, 08:06:24Z) failed on `crates/infrastructure/src/fluid_diarization_runtime.rs:532`:

```
532 |             !matches!(native_compute_for_device(device, host), Err(_))
    |              ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ help: try: `native_compute_for_device(device, host).is_err()`
    = note: `-D clippy::redundant-pattern-matching` implied by `-D warnings`
```

Applied locally at the same revision (exit `101`, one error, `lib` target):

```
error: redundant pattern matching, consider using `is_err()`
   --> crates/infrastructure/src/fluid_diarization_runtime.rs:532:14
error: could not compile `sbobino-infrastructure` (lib) due to 1 previous error
EXIT=101
```

The machine-applicable suggestion in the diagnostic is **not** semantics-preserving when read
literally: the highlighted span excludes the leading `!`, so `.is_err()` alone would invert the
check. Rendered by hand into the surrounding negation it would be `!x.is_err()`, which is
`is_ok()`. The fix spells out `is_ok()`.

### 1.2 `clippy::err_expect` (Linux lint gate, `lib test` target)

CI (`rust / Clippy`, 08:06:25Z) failed three times in `companion.rs`, at `2097:18`, `2135:14`,
`2173:14`, each `.err().expect(...)` on a `Result<(), String>`. The Linux `lib` failure (1.1) and the
`lib test` failure (1.2) are independent: fixing only the first leaves `-D warnings` red.

Because a `lib` failure stops the `lib test` target, the unweakened gate cannot reach 1.2 by itself.
It was reproduced locally with the same command plus a single explicitly-labelled diagnostic
exception, and the exact sites match CI:

```
$ rustup run 1.95.0 cargo clippy -p sbobino-infrastructure --all-targets \
    --target aarch64-apple-darwin -- -D warnings -A clippy::redundant_pattern_matching
error: called `.err().expect()` on a `Result` value
   --> crates/infrastructure/src/companion.rs:2097:18
error: called `.err().expect()` on a `Result` value
   --> crates/infrastructure/src/companion.rs:2135:14
error: called `.err().expect()` on a `Result` value
   --> crates/infrastructure/src/companion.rs:2173:14
error: could not compile `sbobino-infrastructure` (lib test) due to 3 previous errors
EXIT=101
```

That `-A` flag was diagnostic only. Every gate reported below as green ran without it.

Four other `.err().expect(...)` calls exist in the same test module (`companion.rs` 1799, 1823, 1930,
and the second-start assert) but sit on `Result<CompanionServer, String>`. Clippy's `err_expect`
lint is intentionally silent there because `expect_err` would require `Debug`, which
`CompanionServer` does not implement; adding `Debug` to a production server type for a test message
would be a worse change, so those calls are left alone and the lint stays quiet.

### 1.3 `companion::tests::persisted_pairing_state_is_read_only_and_distinguishes_unreadable_profiles` (Windows x86_64)

CI (`Native Rust tests (Windows x86_64) / Verify Windows companion protocol and durable AI jobs`,
08:17:19Z) reported `50` tests: `49 passed; 1 failed`.

```
---- companion::tests::persisted_pairing_state_is_read_only_and_distinguishes_unreadable_profiles stdout ----
thread '...' panicked at crates\infrastructure\src\companion.rs:2222:9:
assertion `left == right` failed
  left: Absent
 right: Unknown
```

Root cause (production bug, not a test portability problem). The old implementation classified a
single child lookup:

```rust
let path = data_dir.join(identity::DIRECTORY).join("pairing.enc");
match std::fs::metadata(&path) {
    Ok(metadata) if metadata.is_file() => Present,
    Ok(_) => Unknown,
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Absent,
    Err(_) => Unknown,
}
```

The failing case is the real layout where `data_dir/companion-pairing` is a regular file, so the
child lookup has to cross a non-directory component. The platforms disagree on that error, and the
disagreement is documented in the toolchain's own source (`rust-lang/rust` tag `1.95.0`, the exact
version CI pins):

| Platform | Mapping file | Relevant arms |
| --- | --- | --- |
| Unix | `library/std/src/sys/io/error/unix.rs:120,129` | `libc::ENOENT => NotFound`, `libc::ENOTDIR => NotADirectory` |
| Windows | `library/std/src/sys/io/error/windows.rs:21-25,49` | `ERROR_FILE_NOT_FOUND \| ERROR_PATH_NOT_FOUND \| ERROR_BAD_NETPATH \| ERROR_BAD_NET_NAME => NotFound`, `ERROR_DIRECTORY => NotADirectory` |

So on Unix the crossed lookup is `NotADirectory` → `Unknown` (test passes), while on Windows it is
`ERROR_PATH_NOT_FOUND` → `NotFound` → the old code answered `Absent`. The CI symptom (`left:
Absent`) is the empirical confirmation that Windows reported `NotFound` for that lookup.

Measured on macOS with the same toolchain (probe compiled directly with `rustc`, no cargo):

```
missing-record        kind=Some(NotFound)        # data_dir/companion-pairing/pairing.enc absent
parent-is-file        kind=Some(NotADirectory)   # companion-pairing is a regular file
parent-metadata       ok=true is_dir=false is_file=true
data-dir-is-file      kind=Some(NotADirectory)
missing-data-dir      kind=Some(NotFound)
dir-as-record         is_file=false is_dir=true
```

The third line is the load-bearing one: the *parent* observation is a plain successful `metadata`
on every platform, so a contract that decides on the parent is portable, while a contract that
decides on the child error kind is not.

## 2. Patch

### 2.1 Contract

`persisted_pairing_state` keeps its signature, its read-only guarantee (no directory, lease, record
or Keychain entry is created, the record is never opened or decrypted, the listener never starts)
and its three states, and now decides from a layout it can actually inspect:

1. `data_dir` exists but is not a directory (or cannot be inspected) → `Unknown`.
2. `data_dir/companion-pairing`:
   - `Directory` → decide on the record;
   - `Missing` → `Absent` (normal first start, nothing was ever saved);
   - regular file or uninspectable → `Unknown`. **This is the Windows case that used to be `Absent`.**
3. Record, only meaningful inside a confirmed profile directory:
   - regular file → `Present` (presence only, never a validity claim);
   - `Missing` → `Absent`;
   - directory / other / uninspectable → `Unknown`.

### 2.2 Code

- `PairingEntryKind { Directory, RegularFile, Missing, Uninspectable }` plus a single
  `pairing_entry_kind(path)` wrapper around `metadata`.
- `classify_persisted_pairing_state(profile_directory, record) -> PersistedPairingState`: the pure,
  private decision table. It is the same code the public entry point runs; splitting it out only
  removes the I/O so the Windows evidence pair can be asserted without a Windows host.
- `persisted_pairing_state` = `data_dir` guard + two `pairing_entry_kind` probes + the table.
  The record probe below a non-directory is a harmless failed `stat`; the parent already decides.
- `fluid_diarization_runtime.rs`: `!matches!(native_compute_for_device(device, host), Err(_))`
  → `native_compute_for_device(device, host).is_ok()`. Negation preserved: `Err` = rejected device
  = unsupported; `Ok(Some(_))` and `Ok(None)` (Pyannote-only preferences such as `mps`/`cuda`)
  stay supported on the `auto` path, exactly as before. Covered by the pre-existing
  `auto_mps_preserves_pyannote_choice`, `auto_ane_does_not_fallback_to_pyannote_when_native_is_missing`
  and `intel_auto_preserves_pyannote_despite_an_installed_fluid_bundle` tests, which still pass.
- Three `.err().expect(...)` → `.expect_err(...)` on `Result<(), String>` only. Panic messages are
  unchanged, so the assertions read identically.

No `#[allow]`, no lint downgrade, no skipped/ignored test, no platform-conditional expectation, no
`cfg(windows)` special case, no dependency or manifest change.

### 2.3 Regression tests

- `companion.rs`, existing real-filesystem test
  `persisted_pairing_state_is_read_only_and_distinguishes_unreadable_profiles` (unchanged in shape,
  this is the test CI failed): `Absent` for an empty profile and no directory creation, `Present`
  for synthetic bytes, `Unknown` for a directory in place of the record, `Unknown` for a
  `companion-pairing` regular file. One case added: `data_dir` itself replaced by a regular file →
  `Unknown`.
- New `persisted_pairing_classification_never_reads_an_uninspectable_profile_as_absent`: an
  eight-row table over `(profile_directory, record)` including the Windows evidence pair
  `(RegularFile, Missing) → Unknown`. Under the old rule that pair answered `Absent`, which is the
  exact `left: Absent / right: Unknown` CI diff; the table therefore encodes the regression, not a
  platform conditional. Real filesystem tests are kept alongside it.

No test mutates process-global state: every case uses its own `tempfile::tempdir()` and no
environment variable, Keychain, network listener or profile is touched.

## 3. Green evidence (local, aarch64-apple-darwin, toolchain 1.95.0)

Exact `ci.yml` `native-macos` gate commands, `matrix.target = aarch64-apple-darwin`:

| Gate | Command (run in `sbobino_desktop`) | Result |
| --- | --- | --- |
| Format | `cargo fmt --all -- --check` | `PASS` (exit 0, no diff) |
| Lint | `cargo clippy --workspace --all-targets --target aarch64-apple-darwin -- -D warnings` | `PASS` (exit 0, `Finished` in 5m 24s, no warnings) |
| Tests | `cargo test --workspace --target aarch64-apple-darwin` | `PASS` (exit 0, 20 binaries, 566 passed, 0 failed, 5 ignored) |
| Companion module (CI Windows step shape, native target) | `cargo test -p sbobino-infrastructure --target aarch64-apple-darwin companion:: -- --test-threads=1` | `PASS` (53 passed, 0 failed) |
| Tight loop | `cargo test -p sbobino-infrastructure --target aarch64-apple-darwin companion::tests::persisted_pairing` | `PASS` (3 passed, 0.12 s) |

The 53 count is consistent with CI's Windows 50: the macOS `cfg(unix)` build adds the two
`#[cfg(unix)]` directory-sync tests and the new classification test.

Reproduction → fix → re-verification used the same gate command, so the RED and GREEN signals are
the same signal. Logs (all under the lane root):

```
logs/red-clippy-arm64-prefix.log                pre-fix unweakened gate, EXIT=101
logs/red-clippy-err-expect-diagnostic.log       pre-fix err_expect sites (diagnostic -A, EXIT=101)
logs/green-clippy-arm64.log                     post-fix unweakened gate, EXIT=0
logs/green-test-workspace-arm64.log             post-fix full workspace suite, EXIT=0
logs/green-companion-module-arm64.log           post-fix companion module + tight loop, EXIT=0
logs/windows-target-check.log                   Windows cross-target attempt (blocked, see below)
evidence/rust-1.95-windows-fs.rs                std fs source, tag 1.95.0 (primary source)
evidence/rust-1.95-unix-errorkind.rs            std unix error mapping, tag 1.95.0
evidence/rust-1.95-std-io-errorkind.rs          std Windows error mapping, tag 1.95.0
tmp/fs_error_kind_probe.rs                      local error-kind probe source
```

## 4. What is *not* verified here

This is an Apple Silicon Mac with an offline registry cache; the statements below are limits, not
passes.

| Item | Status | Evidence / blocker |
| --- | --- | --- |
| Linux lint + Linux tests (`ci.yml` `rust` job) | `UNVERIFIED` | No Linux target is installed (`rustup target list --installed` → `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-pc-windows-msvc` only) and installing a toolchain/target or its system libraries is out of scope for this hold. The same crate set lints clean on macOS, but `cfg(target_os = "linux")` blocks are therefore not compiled. |
| Windows compile (`x86_64-pc-windows-msvc`) | `UNVERIFIED` | Target is installed, but offline resolution fails before any of our code is reached: `cargo check -p sbobino-domain --target x86_64-pc-windows-msvc` → `error: failed to download 'windows-core v0.62.2'`; `cargo check -p sbobino-infrastructure --target x86_64-pc-windows-msvc` → `error: failed to download 'keyring v3.6.3'`. Those target-only crates are not in the shared cache and new downloads are forbidden. No MSVC C toolchain is present either, which would additionally block the bundled `libsqlite3-sys` build. |
| Windows runtime execution | `UNVERIFIED` | No Windows host or VM is available, and this hold forbids CI. The Windows behavioural claim rests on the CI log (`left: Absent`) plus the 1.95.0 std error mapping and a `(RegularFile, Missing) → Unknown` assertion over the production decision table — not on a Windows run. |
| `x86_64-apple-darwin` (Intel matrix entry) | `NOT RUN` | Out of this lane's required scope; would be a second full cold build. The Intel release path is the packaging worker's / root's call. |
| ARM release hydration (`arm-hydrate-2038.log`) | `NOT MINE` | That failure is `install_name_tool ... python.o is not Mach-O` in the portable-Python step (log line 882), i.e. the packaging lane. No Cargo stage in that log failed. |
| Full Tauri release build | `DELEGATED` | Root/packaging worker runs it; Cargo inputs for this crate are green (see section 1 of the reuse note below). |

Because of the three `UNVERIFIED` rows, this report does **not** claim all-platform green. It claims:
the two Rust lint gates and the full workspace test suite are green on aarch64-apple-darwin with the
pinned toolchain, and the Windows decision path is fixed by construction with the Windows evidence
pair encoded in a test.

## 5. Reuse for the full release build

Same shell, same lane target (already warm for `aarch64-apple-darwin` debug/test artifacts):

```sh
cd /Users/pietromastro/Documents/sbobino_tauri/sbobino_desktop
source /Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/rust/env.sh
rustup run 1.95.0 cargo test   --workspace --target aarch64-apple-darwin
rustup run 1.95.0 cargo clippy --workspace --all-targets --target aarch64-apple-darwin -- -D warnings
```

For a release build the target dir can be kept or pointed at a release-specific lane directory; only
the registry cache (`CARGO_HOME`) needs to stay shared to remain offline. Nothing in this lane is
committed, so the root should review the diff and commit it in the lane of its choosing:

```
git -C /Users/pietromastro/Documents/sbobino_tauri diff -- \
  sbobino_desktop/crates/infrastructure/src/companion.rs \
  sbobino_desktop/crates/infrastructure/src/fluid_diarization_runtime.rs
```

Do not revert the four modified `sbobino_desktop/scripts/*` files: they belong to the packaging
worker.

## 6. Lane status

Finished for Cargo. Remaining known failures in the failed run are outside this lane: the
portable-Python/`install_name_tool` hydration step, `test_pyannote_portable_runtime.py`
(`test_install_names_are_reported_per_architecture` on `x86_64-apple-darwin`), and the Intel
supported-inference smoke.
