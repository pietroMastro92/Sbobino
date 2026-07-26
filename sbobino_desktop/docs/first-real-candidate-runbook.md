# Release Candidate Runbook

## Goal

Build, publish, validate, and promote a native multi-platform Sbobino release using GitHub Actions only.

## Required GitHub-hosted runners

- `macos-14` for `aarch64-apple-darwin`
- `macos-15-intel` for `x86_64-apple-darwin`
- `windows-2025` for `x86_64-pc-windows-msvc`

No self-hosted runner is required for stable promotion.

## Step 1: Prepare the tag

Verify version coherence and push the release tag:

```bash
cd sbobino_desktop
./scripts/check_release_versions.sh <version>
git push origin v<version>
```

## Step 2: Dispatch the candidate

Use the `Release Candidate` workflow with `tag_name=v<version>`. The workflow:

1. builds all three native targets;
2. signs and stages updater artifacts;
3. publishes one GitHub prerelease;
4. downloads the exact public assets on matching native runners;
5. uploads the validation proof assets.

## Step 3: Require the full hosted matrix

The following jobs must be `success`:

- native Windows build and installed NSIS smoke;
- native macOS ARM64 build;
- native macOS Intel build;
- candidate publication;
- ARM64 and Intel distribution readiness;
- Windows distribution readiness;
- ARM64 and Intel portability smoke;
- validation proof upload.

## Step 4: Verify public proof assets

The candidate must contain passed:

- `release-readiness-proof.json`
- `distribution-readiness-proof.json`
- `intel-distribution-readiness-proof.json`
- `windows-distribution-readiness-proof.json`
- `portability-smoke-report.json`
- `intel-portability-smoke-report.json`
- `windows-gui-smoke-report.json`

`latest.json` must expose `darwin-aarch64`, `darwin-x86_64`, and `windows-x86_64`.

## Step 5: Promote

Use GitHub Actions `Promote Release Candidate` or:

```bash
./scripts/promote_candidate_release.sh <version> pietroMastro92/Sbobino
```

Promotion marks the release stable and Latest, then keeps the newest two stable releases for rollback.

## Failure rule

If any mandatory hosted job or proof fails, do not promote. Fix the issue, publish a new patch candidate, and rerun the complete matrix.
