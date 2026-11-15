# 2.0.38 root review resolutions

Product source before review: implementation reviewed in companion-2038-packaging-independent-review-20261115.md. This follow-up resolves the concrete review findings without relaxing root guards or portability checks.

- Version wiring: release/promote/verification version locks now use 2.0.38; reviewed-product SHA is set after the product commit, in a separate workflow commit. The v2.0.37 tag remains immutable.
- Staging: guard-root checks the requested path and its canonical path with existing symlink parents resolved before the shell can mkdir. A synthetic symlink into a protected Application Support path is rejected without creating its child.
- Audit closure: remove the global basename fallback. Only declared, expanded runpaths establish a match. Bare @loader_path is supported; @executable_path is resolved only with an explicit executable context, never inferred from a dylib directory. Unknown executable context remains a closure finding.
- Dyld identities: repaired libraries whose basename occurs more than once receive a deterministic layout-scoped identity. Direct runpath dependencies on those repaired libraries become verified loader-relative dependencies, avoiding import-order coalescing.
- Compatibility wording: removed the unconditional no-end-user-behavior claim; recorded the preserved wheel versions and actual loader checks instead.

## Validation

Five new native counterexamples fail against the implementation snapshot preceding root fixes (5 failures); the same five pass after correction. Full relevant packaging/staging tests: 55/55 passed. Workflow/quality contract tests: 34 run, one existing native-PowerShell skip. Version and notes gates pass. CI explicitly runs staging and loader-review tests on both native macOS runners.

A fresh APFS copy of the R2 payload was sanitized with current production tooling and moved to a new path containing spaces. Audit: 449 Mach-O files, zero violations, 439 valid signatures, 186 conservative warnings (176 unresolved static runpath references, 10 unsigned files covered by bundle signing). Strict closure is not claimed. Independent loader processes with a clean environment successfully imported sklearn then torch, and torch then sklearn. All 15 import/use checks passed. The production diarization entrypoint on the relocated payload processed the existing 8.83-second synthetic fixture offline and returned two speakers, exit zero.

Evidence: /Volumes/UltraDisk/sbobino-companion-product-20261115/packaging-2038/root-review/{red.log,green.log,real/audit.json,real/loader-results.json,real/diarization.stdout}. Earlier operator harness failures from the wrong test working directory and an absent TMPDIR were corrected before these results; they are not product-test passes.

No hosted Intel/Windows result or public-release pass is claimed here. No stable promotion, personal archive access, Keychain change, phone operation or VPN/firewall change. The next gate is the complete official candidate pipeline.
