# Companion 2.0.39 — Intel lifecycle failures: local repair, 2026-11-15

Root integration note: this is the implementation lane's historical checkpoint
(three files, 224 library tests). The final integrated source also changes
`companion_ai.rs` and includes additional review regressions. See
`companion-lifecycle-final-integration-20261115.md` for the final source hashes,
counts, review dispositions and scope. The duplicated-descriptor regressions pin
the explicit-unlock contract; they do not identify the kernel-level cause of the
intermittent baseline failure.

Implementation lane for the four native `x86_64-apple-darwin` Rust failures of CI
[43709717704](https://github.com/pietroMastro92/Sbobino/actions/runs/43709717704)
at `1188c647e135ab6525707086ea6e67171038af45`.

No commit, stage, push, tag, dispatch, rerun, upload or promotion was performed; the
version stays `2.0.39` because no tag exists. Nothing was changed outside the three
files below. Root owns integration, review and every VCS/CI action from here.

Machine-readable record: `/Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/intel-lifecycle/evidence/intel-lifecycle-summary.json`.
Logs, probes and binaries: the sibling `logs/` and `probe/` directories.

## 1. Evidence read before touching code

- `docs/reviews/companion-2039-main-ci-20261115.md` (job table, four assertions, Intel history).
- `ci-pre-candidate-2039/logs/intel-job-logs.clean.log`: `219 passed; 4 failed`, exit 101, exact messages at `whisper_stream.rs:1547`, `companion.rs:2183`, `companion.rs:1884`, `companion.rs:1912`.
- `hist-132550258473.clean.log` (`5198720`), `hist-132560871831.clean.log` (`2aebad88`), `hist-31d832b.clean.log` (green): three of four Intel runs red on a **varying subset** of the same cluster, same two messages.
- Product-scope diff `7374914..1188c64` is empty, so CI compiled exactly the code in this checkout.

## 2. Local reproduction (the loop that goes red)

Environment (all caches/temp on UltraDisk, `CARGO_BUILD_JOBS=1`, `CARGO_INCREMENTAL=0`, offline):

```
TMPDIR=/Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/intel-lifecycle/tmp
CARGO_HOME=/Volumes/UltraDisk/sbobino-companion-product-20261111/desktop/cargo-home
CARGO_TARGET_DIR=/Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/rust/target
rustc/cargo/clippy 1.95.0
```

`cargo test -p sbobino-infrastructure --lib --target x86_64-apple-darwin --no-run` produces a
thin `Mach-O 64-bit executable x86_64` (`lipo -info` verified); it is executed with
`/usr/bin/arch -x86_64`, libtest default parallelism, no filtering tweaks. This is a
translated x86_64 process on ARM — **not** native-Intel certification.

Baseline at `1188c64`, unmodified source:

| Loop | Result |
| --- | --- |
| first full lib suite | **FAILED** `222 passed; 1 failed` — `stop_after_child_exit_…`, message identical to CI (`transcript: "partial preview must not be saved"`, `saved_audio_path: Some(...)`) |
| 14 sequential full lib suites | 9 green / **5 red**; failing instances: `cancelled_shutdown…` ×3, `dropped_server…` ×2, `concurrent_shutdowns…` ×1, `pairing_lease_blocks…` ×1, `companion::ai::tests::durable_replay…` ×2 |
| focused 4-test cluster ×20 | 18 green / 2 red (`cancelled_shutdown…`, `LEASE_BUSY`) |

So all four CI assertions except `occupied_persisted_port…` reproduce locally; that one was
reproduced as its mechanism (§3.4) rather than by luck.

## 3. Root causes: proven, not assumed

### 3.1 Whisper stop — the test's precondition was a 25 ms sleep (proven)

`stop_after_child_exit_…` spawned `sh -c "exit 7"`, slept 25 ms, installed the child and called
`stop()`. The reproduced failure returns `Ok` with the test's own line as transcript, which is only
possible if `stop()`'s `try_wait()` saw `None`: the loop has `active_readers = 0`, so nothing else
can mark the child failed. The sleep does not establish the precondition it assumes.

### 3.2 Whisper stop — a crash during the stop was saved as success (proven)

`stop()` sent SIGINT, then `timeout(900ms, child.wait())` and **discarded the returned
`ExitStatus`**. Any worker that exits with a failure status while the stop waits was therefore
reported as a successful session with the partial preview. This is a product hole, not only a
test defect; it is independent of the 25 ms race in §3.1.

### 3.3 Pairing lease — release relied on close-time semantics (proven by instrumentation)

Instrumented `PairingStore` (temporary `[DEBUG-…]` prints, removed afterwards) on the real failing
tests:

- `release_lease` took the `File` out of the store and dropped it; `/dev/fd/<fd>` immediately
  reports `EBADF`, i.e. the descriptor really was closed.
- The next `try_lock` on the same path still returned `TryLockError::WouldBlock`.
- `/usr/sbin/lsof -n -P -w <lock path>` at that exact moment printed **no holder at all** — no
  process, no descriptor — and the following attempt succeeded.
- The failing tests are exactly the ones that probe the lease (a failed `revoke`/`start`) while it
  is held, then release it and re-acquire immediately.

The observation does not prove asynchronous kernel release or rule out a descriptor not captured
by the instrumentation. `flock` specifies release when the last descriptor for the open file
description closes. The explicit-unlock repair defines the ownership boundary without depending
on that last close. Its causal kernel attribution remains unverified; the baseline failures and
the before/after stress results are measured evidence.

### 3.4 Occupied persisted port — TIME_WAIT ruled out, ephemeral reuse proven (test-only)

Probe (`probe/portprobe.rs`, both architectures):

- a port with a **server-side TIME_WAIT** connection rebinds fine (`Ok`), so TIME_WAIT is not the cause and no `SO_REUSEADDR` change is warranted;
- binding a port that a **live socket** owns fails with `AddrInUse` (errno 48) — the CI error;
- a client socket meanwhile holds an ephemeral local port of its own.

The production listener is provably closed before `shutdown()` returns (the restart tests rebind the
persisted port successfully, and Tokio drops the task future — listener included — inside
`Core::poll` before the join waker fires, `tokio-1.49.0/src/runtime/task/core.rs:361`). The old test
re-bound the *ephemeral* port of a stopped listener with a plain `std::net::TcpListener::bind`; on a
loaded runner another socket in the same process can already own that port, so the assertion was a
race against unrelated traffic. `net.inet.ip.portrange.first = 49152` locally.

### 3.5 Hypotheses that did not survive

- "duplicated/inherited open file descriptions": not observed in the instrumented companion-only
  cluster. The negative `lsof` sample is insufficient to exclude this causally (§3.3).
- "accept-task/destructor ordering": `drop(accept_store)` and the listener close happen before the
  `shutdown` future completes (Tokio drops the future on `Ready`), and drain-before-release already
  holds; the block measured here is only the lock-release ordering.
- "release the lease early / rotate the port / pick another port": rejected; every fix keeps
  fail-closed behaviour.

## 4. Fixes (three files)

`crates/infrastructure/src/adapters/whisper_stream.rs`

- `status_ends_an_intentional_stop()` + `status_signal_is_our_interrupt()`: a status observed by the
  interrupt wait is an intentional stop only for exit code 0 or SIGINT (the signal this stop sends);
  any other exit code, or a signal the stop never sent, is a crash and goes through the existing
  `mark_terminal_child_failure` path, so the recovery WAV path and the exact `status N` message are
  preserved. Escalated stops (SIGTERM/SIGKILL after the 900 ms budget) keep today's behaviour, so a
  forced stop is never reported as a crash on any platform.
- `await_child_exit()` replaces the 25 ms sleep: the precondition is the OS-observable exit of the
  fixture, the same primitive `stop()` uses.
- new `child_that_fails_while_stopping_is_not_saved_as_success()`: a fixture that ignores SIGINT and
  exits with status 7 when `stop()` closes the worker's stdin inside `Child::wait`. This exercises
  the exact race — alive when `stop` looks, failing while `stop` waits — with no elapsed-time
  assumption and no signal-handler race.

`crates/infrastructure/src/companion_identity.rs`

- `release_lease()` now calls `File::unlock()` (explicit `LOCK_UN`, processed before it returns) and
  only then drops the descriptor; a poisoned mutex no longer skips the release, and an unlock error
  is logged; closing the descriptor remains the OS fallback.
- `impl Drop for PairingStore` does the same, so a store dropped without `shutdown` (owner gone while
  the accept task drained) releases identically. This is the path the `dropped_server…` test
  exercises.
- No retries, no sleeps, no early grant. An unlock failure is logged and the descriptor is closed;
  a subsequent open still fails closed if the lease remains occupied.

`crates/infrastructure/src/companion.rs`

- corrected the accept-task comment that claimed the `Arc` drop releases the lease: the reference
  keeps the store — and the lease — alive through the drain; the release is the explicit unlock in an
  awaited `shutdown` or the store's `Drop`.
- `occupied_persisted_port_fails_without_rotating_identity` now owns its port and persists an
  identity for it, so the conflict is deterministic; the fail-closed assertion, the "no rotation"
  assertion and the byte-identical record assertion are unchanged.
- `start_persisting_server` persists a fresh profile's port below the ephemeral range (proven free
  inside the test) and `shutdown_drains_session_and_restart_reuses_the_persisted_pairing` uses that
  entry point, so restart coverage no longer races ephemeral reassignment.

Not touched: `companion_ai.rs`, `src-tauri` commands, versions, locks, dependencies, workflows,
scripts, other reports.

## 5. Behavioural RED/GREEN per corrected cause

| Cause | RED evidence | GREEN |
| --- | --- | --- |
| crash during stop saved as success | new test with the classification disabled: `FAILED … `WhisperStreamStopResult { transcript: "partial preview must not be saved", …, saved_audio_path: Some(...) }` (`logs/red/red-whisper-crash-test.log`) | passes with the classification, isolated and under load |
| 25 ms precondition | baseline full-suite run above failed with the identical message | `await_child_exit` version passes in every run |
| lease release ordering | baseline cluster 2/20 red, baseline full suite 5/14 red | cluster 20/20 + 10/10 (32 threads) green, full suite 10/10 green |
| ephemeral port reuse | baseline `occupied_persisted_port…` failed with `AddrInUse` on CI and the probe reproduces `AddrInUse` from a live socket | deterministic occupied-port test passes in every run |

## 6. Final local gates (frozen source, hashes in §7)

| Gate | Result |
| --- | --- |
| `x86_64-apple-darwin` lib, focused 9-test lifecycle cluster ×20, default parallelism | **20/20 green** |
| same cluster, `--test-threads=32` (oversubscription) ×10 | **10/10 green** |
| `x86_64-apple-darwin` full lib suite ×10 | **10/10 green** — `224 passed; 0 failed; 0 ignored` |
| full lib suite `--test-threads=32` ×8 | 6/8 green; the 2 red are `companion::ai::tests::durable_replay_conflict_cancel_restart_and_ack` only (§7) |
| `aarch64-apple-darwin` lib, focused cluster ×20 | **20/20 green** |
| `aarch64-apple-darwin` full lib suite ×10 | **10/10 green** — `224 passed; 0 failed; 0 ignored` |
| the exact four CI test names, 10 repetitions each | x86_64 **10/10**, aarch64 **10/10** green |
| `cargo test --workspace --target aarch64-apple-darwin` | **567 passed, 0 failed, 5 ignored** (20 suites; same five pre-existing `requires real …` ignores) |
| `cargo clippy --workspace --all-targets --target aarch64-apple-darwin -- -D warnings` | exit 0 |
| `cargo clippy -p sbobino-infrastructure --all-targets --target x86_64-apple-darwin -- -D warnings` | exit 0 |
| `cargo check --workspace --all-targets --target x86_64-apple-darwin` | exit 0 |

The suite count moved from 223 to 224 because of the added regression test; no test was skipped,
ignored or weakened, and no assertion was lowered.

## 7. Residual risk that needs root

`companion::ai::tests::durable_replay_conflict_cancel_restart_and_ack` fails 2/10–2/18 full-suite
runs on the fixed source, with `La sessione AI precedente si sta ancora chiudendo. Riprova.` The AI
store (`crates/infrastructure/src/companion_ai.rs`) keeps its lease in a plain `_lease: File` and
relies on the same close-time release; the error comes from its `try_lock`.

It is **outside this lane's file ownership**, so it is not patched here. Measured with a temporary
patch (reverted; `logs/exp-ai/`): `impl Drop for Store { fn drop(&mut self) { let _ = self._lease.unlock(); } }`
→ **14/14** oversubscribed full-suite runs green. Applying that (or authorising me to) is the last
step needed before an Intel run can be expected green; without it the Intel job can still flake on
this one test.

Limits of this evidence: no Intel hardware, local macOS 26 / Xcode 27.2 versus the hosted macOS 15 /
Xcode 16.4, and the Windows behaviour of the new status classification is reasoning + compile only.
The repaired cluster is proven deterministic locally in both process modes; native Intel remains
unproven until the hosted job runs.

## 8. Reproduction commands

```
cd /Users/pietromastro/Documents/sbobino_tauri/sbobino_desktop
set -a; . /Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/rust/env.sh; set +a
cargo test -p sbobino-infrastructure --lib --target x86_64-apple-darwin --no-run
/usr/bin/arch -x86_64 <printed test binary>                 # full lib suite
/usr/bin/arch -x86_64 <printed test binary> <the 9 filters>  # focused cluster
/Volumes/UltraDisk/sbobino-companion-product-20261115/local-release-repair/intel-lifecycle/battery.sh <binary> "/usr/bin/arch -x86_64" <label>
```
