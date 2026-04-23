# Distribution Validation Plan

## Goal

Ship a macOS release that installs and runs on a clean third-party Apple Silicon Mac without requiring:

- Homebrew
- host Python
- manually installed ffmpeg / whisper / pyannote dependencies
- terminal repair steps
- ad hoc human debugging during first launch

For now, this document defines the mandatory validation bar for `macOS Apple Silicon`.
It also defines the shape of the future matrix for `macOS Intel x86_64` and `Windows`.

## What mature software teams do

Teams that consistently ship reliable desktop software usually combine:

1. Deterministic build inputs.
2. Signed and notarized installers.
3. A release-candidate gate that validates the exact public artifacts, not just local builds.
4. Clean-room install testing on machines that do not share developer state.
5. Upgrade-path testing from at least one previous public version.
6. Clear exit criteria that block release if any mandatory scenario fails.
7. A small validation matrix that grows by platform instead of relying on one “works on my Mac” machine.

That is the direction this plan formalizes for Sbobino.

## Release Policy

A release is distributable only if all mandatory Apple Silicon scenarios pass on the exact GitHub release assets for that version.

Mandatory rule:

- Do not publish or promote a stable release until the Apple Silicon distribution matrix in this document is green.
- Publish the release as a prerelease candidate first, then promote only after `release-readiness-proof.json`, `distribution-readiness-proof.json`, and the machine validation report assets are uploaded with passed statuses.
- If the release fails on a third-party Mac, retire it and cut a new patch version.
- Never fix a broken stable release in place.

## Current Scope

### Required platform now

- `macOS Apple Silicon (arm64)`

### Required supporting platform now

- `macOS Intel x86_64` as bootstrap and distribution-layer validation for the arm64 release flow

### Required platform later

- `Windows x86_64`

## Test Environments

We should maintain these validation environments:

1. `AS-PRIMARY`
   Apple Silicon Mac with normal developer/team usage. This machine validates the real upgrade path from the previous stable public release.

2. `AS-THIRD`
   A physically separate third-party Apple Silicon Mac used as the clean-room install gate.

3. `INTEL-PRIMARY`
   Intel Mac used to validate release metadata, DMG integrity, manifest parsing, and bootstrap-layer compatibility for the arm64 candidate while Intel-native binaries are still out of scope.

Useful but optional:

4. `AS-STRESS`
   Apple Silicon Mac used for failure injection: flaky network, low disk, interrupted first launch, interrupted update.

Recommended GitHub runner labels:

- `AS-PRIMARY`: `self-hosted`, `macos`, `apple-silicon`, `as-primary`
- `AS-THIRD`: `self-hosted`, `macos`, `apple-silicon`, `as-third`
- `INTEL-PRIMARY`: `self-hosted`, `macos`, `x64`, `intel-primary`

## Apple Silicon Matrix

Every release must pass all of these scenarios.

### A. Artifact integrity

Purpose: prove the public GitHub release is internally coherent.

Must pass:

- `./scripts/release_readiness.sh <version>`
- `./scripts/distribution_readiness.sh <version>`
- manifest consistency for app/runtime/pyannote assets
- runtime smoke check
- pyannote asset smoke check

Release blocker if any of these fail.

### B. Clean-room install on third Mac

Machine: `AS-THIRD`

Preconditions:

- no `/Applications/Sbobino.app`
- no `~/Library/Application Support/com.sbobino.desktop`
- no reliance on Homebrew or developer tools

Steps:

1. Download the exact DMG from the GitHub release.
2. Install to `/Applications`.
3. Launch via normal user flow.
4. Complete first-launch setup.
5. Open `Settings > Local Models`.

Pass criteria:

- app opens successfully
- runtime installs without terminal actions
- whisper models install successfully
- pyannote runtime installs successfully
- pyannote model installs successfully
- `Settings > Local Models` reports pyannote `Ready`
- user is never forced into manual repair

### C. Warm restart

Machine: `AS-THIRD`

Steps:

1. Quit the app after setup completes.
2. Relaunch the app.
3. Open the main UI and `Settings > Local Models`.

Pass criteria:

- app reaches the main UI without repeating first-launch setup
- no heavy blocking inspection path on normal reopen
- runtime remains ready
- pyannote remains ready

### D. Functional diarization smoke

Machine: `AS-THIRD`

Steps:

1. Import a known short audio fixture with at least two speakers.
2. Run transcription with speaker diarization enabled.

Pass criteria:

- transcription completes
- diarization completes
- speaker segments are assigned in the timeline
- no pyannote runtime error is surfaced to the user

### E. Update-path validation

Machine: `AS-PRIMARY`

Steps:

1. Install the latest previous public version.
2. Ensure runtime, models, and pyannote are already working.
3. Update to the new release through the real shipped flow.
4. Launch after update.
5. Open `Settings > Local Models`.
6. Run one diarized transcription.

Pass criteria:

- update completes cleanly
- no manual repair is required
- pyannote is preserved or auto-migrated
- user can still use diarization the same way as before the update

### F. Intel bootstrap-layer validation

Machine: `INTEL-PRIMARY`

Steps:

1. Download the exact candidate DMG from the GitHub release.
2. Run `distribution_readiness.sh` against that public release.
3. Mount the DMG and inspect the shipped app bundle and updater/manifests.

Pass criteria:

- remote asset integrity passes
- bundle version matches the requested release version
- DMG contains a valid `Sbobino.app`
- executable, updater metadata, and manifests are coherent
- report may end as `soft_pass` if arm64 execution is intentionally `not_applicable`

### G. First-launch failure recovery

Machine: `AS-STRESS` or controlled Apple Silicon test host

Scenarios:

- network interruption during runtime download
- network interruption during pyannote download
- interrupted app launch during staged pyannote install
- low-disk rejection during install

Pass criteria:

- app does not get stranded in a permanently broken state
- staged pyannote install rolls back safely
- next launch can recover automatically or retry cleanly
- user is not left with a half-installed runtime that requires manual filesystem cleanup

## Exit Criteria For Stable Release

A stable Apple Silicon release is allowed only if:

1. `release_readiness.sh` passes.
2. `distribution_readiness.sh` passes.
3. `AS-PRIMARY` passes the native legacy-upgrade scenario from `0.1.16`, warm restart, and diarization smoke.
4. `AS-THIRD` passes clean-room install, warm restart, and diarization smoke.
5. `INTEL-PRIMARY` passes release metadata and bootstrap-layer validation, with `soft_pass` allowed when arm64 execution is `not_applicable`.
6. No mandatory scenario requires terminal repair or manual filesystem intervention.

If any item fails, the release is not distributable.

## Required Evidence Per Release

Each release should produce a short validation record with:

- version
- release URL
- machine class tested (`AS-PRIMARY`, `AS-THIRD`, `INTEL-PRIMARY`)
- OS version
- outcome of each scenario
- timestamp
- tester
- blocking issue links if failed

Minimum evidence for the release thread or release notes folder:

- full `release_readiness.sh` success
- full `distribution_readiness.sh` success
- `release-readiness-proof.json` uploaded on the GitHub release with `status=passed`
- `distribution-readiness-proof.json` uploaded on the GitHub release with `status=passed`
- `AS-PRIMARY.validation-report.json` uploaded on the GitHub release with `status=passed`
- `AS-THIRD.validation-report.json` uploaded on the GitHub release with `status=passed`
- `INTEL-PRIMARY.validation-report.json` uploaded on the GitHub release with `status=passed` or `status=soft_pass`

## Future Matrix Extension

### Windows x86_64

When Windows support is introduced, repeat the same structure with:

- `WIN-CLEAN-PRIMARY`
- `WIN-THIRD-PC`
- `WIN-UPGRADE-PC`

Additional checks:

- installer/uninstaller behavior
- Defender/SmartScreen friction
- path quoting
- runtime extraction permissions
- upgrade retention of runtime/model assets

## Immediate Next Step For Sbobino

For Apple Silicon, the release bar should now be:

1. local `release_readiness.sh`
2. uploaded release `distribution_readiness.sh`
3. `distribution-readiness-proof.json` uploaded to the prerelease
4. clean-room validation on `AS-THIRD`
5. upgrade validation from previous public version on `AS-PRIMARY`
6. Intel bootstrap-layer validation on `INTEL-PRIMARY`
7. only then manual stable promotion

That gives us a real distribution process instead of a developer-machine check.
