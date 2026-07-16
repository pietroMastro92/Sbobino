# Bug Reproducer

## ✅ FIX_PROVEN — Bug reproduced and fix proven

> Both approved regression tests failed for the predicted causes, passed after the approved causal fixes, and the broader Rust and TypeScript gates passed.

**Project:** Sbobino Desktop  
**Bug:** Concurrent chat context corruption and AI quota misclassification  
**Environment:** macOS Apple Silicon, Rust workspace and Vitest frontend, local test environment  
**Generated:** 2026-07-16

## Discovery scope

- Tauri artifact chat orchestration and adaptive context construction
- AI service verification error classification
- Provider configuration and model discovery paths

## Ranked and tested candidates

| # | Candidate | Contract evidence | Trigger | Location | Confidence | Outcome |
|---:|---|---|---|---|---|---|
| 1 | Concurrent questions can be paired with the wrong answers or omitted from model context | Only complete question-answer pairs become context, and concurrent conversations must preserve every turn. | Persist user1, user2, assistant1, assistant2 before building the next prompt. | /Users/pietromastro/Documents/sbobino_tauri/sbobino_desktop/apps/desktop/src-tauri/src/commands/artifacts.rs:851 | high | REPRODUCED |
| 2 | HTTP 429 responses mentioning an API key are classified as authentication failures | Authentication and quota failures have distinct localized UI states. | Classify AI provider returned 429: API key quota exceeded. | /Users/pietromastro/Documents/sbobino_tauri/sbobino_desktop/apps/desktop/src-tauri/src/commands/settings.rs:546 | high | REPRODUCED |

## Original report

Inspect the provider and persistent-chat implementation for concrete correctness defects before preparing a stable candidate release.

| Contract | Expected | Actual |
|---|---|---|
| Observed behavior | Concurrent completed turns remain correctly paired and HTTP 429 is reported as a quota failure. | The first concurrent question disappeared from context, while the quota error was labeled as authentication. |

## Minimal reproduction

Two focused Rust unit tests exercise the production context builder and service-error classifier with deterministic inputs.

**Confirming signal:** The context omitted Question one; the classifier returned ai_authentication instead of ai_quota.

### Reproduction files approved at Gate 1

- [artifacts.rs](/Users/pietromastro/Documents/sbobino_tauri/sbobino_desktop/apps/desktop/src-tauri/src/commands/artifacts.rs:3488) — Approved concurrent-turn regression test.
- [settings.rs](/Users/pietromastro/Documents/sbobino_tauri/sbobino_desktop/apps/desktop/src-tauri/src/commands/settings.rs:703) — Approved quota-classification regression test.

## Red to green evidence

| Evidence | Before fix | After fix |
|---|---:|---:|
| Exit code | 101 | 0 |
| Timed out | False | False |
| Duration | 15,730 ms | 15,970 ms |
| Same command | — | True |
| Broader suite | — | passed |

### Before — failing evidence

```text
concurrent_completed_chat_turns_keep_correct_question_answer_pairs failed: assertion failed: enriched[0].contains("user: Question one")
quota_errors_are_not_misclassified_as_authentication failed: left ai_authentication, right ai_quota
```

### After — fixed evidence

```text
concurrent_completed_chat_turns_keep_correct_question_answer_pairs ... ok
quota_errors_are_not_misclassified_as_authentication ... ok
```

## Root cause

Chat context retained only one pending user and backend chat calls were not serialized per artifact. Error classification searched for API key before checking the authoritative 429/quota signal.

## Approved fix

Added per-artifact backend chat serialization, FIFO recovery for interleaved historical turns, and quota-first error classification.

**Why this is causal:** Serialization prevents new interleaving, FIFO preserves already interleaved complete pairs, and prioritizing 429 directly corrects the erroneous branch selection.

### Production files approved at Gate 2

- [state.rs](/Users/pietromastro/Documents/sbobino_tauri/sbobino_desktop/apps/desktop/src-tauri/src/state.rs:39) — Per-artifact weak chat-lock registry.
- [lib.rs](/Users/pietromastro/Documents/sbobino_tauri/sbobino_desktop/apps/desktop/src-tauri/src/lib.rs:177) — Initializes the chat-lock registry.
- [artifacts.rs](/Users/pietromastro/Documents/sbobino_tauri/sbobino_desktop/apps/desktop/src-tauri/src/commands/artifacts.rs:667) — Serializes each artifact conversation and pairs queued turns FIFO.
- [settings.rs](/Users/pietromastro/Documents/sbobino_tauri/sbobino_desktop/apps/desktop/src-tauri/src/commands/settings.rs:546) — Prioritizes quota signals over authentication wording.

## Verification

| Check | Status | Evidence |
|---|---|---|
| Focused regressions | ✅ passed | Both tests changed from deterministic failure to pass. |
| Rust workspace | ✅ passed | cargo test --workspace passed; two pre-existing real-runtime smoke tests remain intentionally ignored. |
| Frontend tests | ✅ passed | 30 files and 139 tests passed. |
| Frontend production build | ✅ passed | TypeScript and Vite production build completed. |

## Reproduce

```bash
cargo test -p sbobino-desktop concurrent_completed_chat_turns_keep_correct_question_answer_pairs -- --nocapture
```
```bash
cargo test -p sbobino-desktop quota_errors_are_not_misclassified_as_authentication -- --nocapture
```

## Limitations

- The deterministic concurrency regression exercises the production pairing logic; the backend lock is covered by compilation and the broader suite rather than a timed multi-request integration test.
- Live provider credentials and a Windows host were not part of this local proof.

## Residual risks

- Provider fallback attribution and draft-aware model discovery remain untested candidates from the read-only audit.
- Stable release readiness still requires native artifact and real-runtime validation on Apple Silicon, Intel macOS, and Windows.

## Notes

- Gate 1 approved only the two focused regression tests.
- Gate 2 approved the four production files and the exact causal transformation.
- No dependencies or public command signatures changed.

---

Generated by `$bug-reproducer`. A fix is proven only by the same red-to-green reproducer plus relevant broader checks.
