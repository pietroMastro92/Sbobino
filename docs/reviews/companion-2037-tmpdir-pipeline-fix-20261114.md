# Temporary staging fix

Both native macOS jobs failed before packaging because the path guard rejects /private/ staging. Compared five remedies: runner-owned TMPDIR; split build-path and loader-path guards; normalize the tempfile alias; stage under the checkout; add macOS-default-temp regression coverage with a core fix.

Selected runner-owned TMPDIR for hydration and packaging. GitHub documents RUNNER_TEMP as runner-owned temporary storage (https://docs.github.com/en/actions/reference/workflows-and-actions/variables); Python documents TMPDIR as its first temporary-directory override (https://docs.python.org/3/library/tempfile.html). This changes only build staging, without weakening the guard or modifying reviewed product code. Source-contract tests run with the same external temporary-root policy. A general default-temp compatibility improvement remains a separate product change.

The existing v2.0.37 annotated tag remains immutable. Dispatch the official Release Candidate workflow from the fixed main workflow ref with tag_name=v2.0.37. All product checkouts and proof commit hashes remain tied to the existing tag, and the full native matrix runs again. No stable promotion or manual asset upload. Record dispatch workflow SHA separately from product tag SHA.

Validation: 51 tests successful, one existing native-pwsh skip on this Mac. Ruby/Psych YAML parse and bash -n on changed run blocks succeeded. Two safe staging roots accepted; /private, Application Support and pairing.enc remained rejected. Product-tree diff against f9055b289ede447a265e1cb51a177b304dbcf3bd is empty. No path-guard relaxation, no updater key change, no stable promotion. Initial run 43628701223 remains failed evidence (Windows build success, both macOS hydration failures).
