# Desktop 2.0.40 — final Intel Conda integration,2026-11-16

## Outcome and source boundary

All local release-repair checks pass. Candidate2.0.39 never published: ARM/Windows built but Intel hydration failed at ncurses. Its immutable tag remains on67b83f9. Stable2.0.36 and all prior tags remain unchanged. Version2.0.40 is prepared locally; no40 tag, push, dispatch or promotion has occurred at this checkpoint.

The exact final nine script hashes match the independent review, both architecture unit runs, original/relocated Intel and ARM verify gates, and a new complete raw Intel hydration. The fresh hydration reruns the actual Conda/PyTorch/TorchAudio/TorchCodec-source/pip/model path with the final planner, reusing task-owned download caches. This closes the review's earlier boundary that final source had not run on the raw full prefix. No fabricated uname or ARM-payload substitution was used: Intel executables run as x86_64 through Rosetta, not native Intel hardware.

## Product fixes

- An exact existing canonical in-payload MH_DYLIB target need not repeat its absolute load path in its dylib ID. All slices and identity checks remain; suffix reconstruction keeps strict provenance. Basename collisions, ambiguous matches, mixed bitcode, non-dylibs, symlink escapes and host-path guards retain their negative tests.
- Conda string-prefix rewriting can invalidate an existing ad-hoc signature without changing load commands. Only known invalid ad-hoc signatures are repaired. Unsigned remains unsigned and visible; invalid genuine/unknown signing identity refuses. Root strengthened the existing negative to snapshot every file's SHA/mode/mtime, reproduced mutation-before-refusal RED, then moved signature classification before every write and obtained GREEN. Ad-hoc integrity is not an authenticity claim.
- POSIX product CLI reserves the original stdout descriptor for its JSON result, redirects native/Python diagnostics to stderr before model import, and leaves the redirect through native finalizers. The whole-output JSON parser is unchanged; argparse help and the Windows output path are unchanged. Five subprocess regressions use real libc output, including delayed finalizers and error exit1 with empty JSON channel: baseline3 noisy cases RED, final5/5 GREEN. Actual Intel MKL warnings now appear in stderr with one valid speaker JSON on stdout.

## Local proof

| Gate | Result |
| --- | --- |
| Packaging/staging/loader+stdout tests |72+16+11=99 PASS nativeARM,99 PASS actualx86_64 process |
| Final-source fresh Intel hydration |PASS from raw Conda prefix; full audit0violations,244 valid present signatures; real product JSON1turn |
| Complete Intel original+relocated gate |PASS: audit0violations, byte-idempotence55546files,1445load-command scans clean,755MiB official zip, both import orders, real speaker inference, signatures |
| Independent ARM original+relocated gate |PASS with the same final script hashes |
| Cargo workspace |572 PASS on ARM (5 declared ignored),572 PASS on x86_64 (4 declared ignored); fmt and all-targets Clippy clean on both |
| Public ARM app40 |Rebuilt; canonicalcom.sbobino.desktop,arm64,ad-hoc deep/strict signature valid; exact final helper bytes found in executable |
| Coherence |Version/notes40, no-skipped guard and diff-check PASS; no dependency upgrades |

App executable SHA256:`af60f7a37227cce8d15db4afb9d02b71c1a4b8f14eab07f807a2afcfb9c450e6`. Local updater signature remains absent: the private release key exists only in the hosted release environment. No local GUI/LAN/iPhone or DMG-mount certification is claimed. Native Intel/Windows public distribution and all7 same-tag/SHA proof assets remain the hosted promotion gates.

Warnings remain explicit: Intel1486 warnings and non-strict closure, unsigned Conda core interpreter/libpython, unused unresolved-rpath variants and inert build-prefix data.0violations is not full strict-closure certification. The dependency solve is not completely hermetic. Review P3 bundle-seal ordering edge is fail-closed and not exercised by this standalone Intel payload; byte-tree idempotence does not attest all mode bits. No P1/P2 was found by independent review. No first-party assertion/skip/timeout was weakened.

## Resource containment and provenance

A task-local watcher now reads job-level failure while the run is still in progress; it binds run/head/workflow/tag before cancellation and can stop the same-head optional installer peer too.15 checks include reconstruction of the actual39 Intel-failed/ARM+Windows-active instant, through the production decision function. It never dispatches or retries a workflow. Its code is external task tooling, not a new persistent automation or product behavior.

Evidence:`/Volumes/UltraDisk/sbobino-companion-product-20261115/intel-conda-repair/root-final/`, machine receipt`evidence/final-2040-local-receipt.json`; independent review`evidence/independent-review.md`; fresh-source logs/receipts`fresh-source-hydration/`; all original implementation evidence preserved. Full source body of the new helper is verified inside `/Volumes/UltraDisk/sbobino-companion-product-20261115/native-local-2040/dist/bundle/Sbobino.app`.

All temp/build/cache files are task-owned on mounted UltraDisk, one Cargo job and incremental off, HOME unchanged. Personal app/archive, phone, simulator, Keychain, VPN and firewall were not changed. Next: selective product commit, CI/docs-only pin commit, then one official CI push; tag40/RC only after complete same-head CI success. No failed tag is moved or reused.
