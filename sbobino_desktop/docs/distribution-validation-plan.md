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
- Publish the release as a prerelease candidate first, then promote only after the Apple Silicon validation report assets are uploaded with `status=passed`.
- If the release fails on a third-party Mac, retire it and cut a new patch version.
- Never fix a broken stable release in place.

## Current Scope

### Required platform now

- `macOS Apple Silicon (arm64)`

### Required platform later

- `macOS Intel x86_64`
- `Windows x86_64`

## Test Environments

We should maintain at least these Apple Silicon environments:

1. `AS-CLEAN-PRIMARY`
   A clean Apple Silicon Mac with no prior Sbobino app data.

2. `AS-CLEAN-THIRD-MAC`
   A physically separate third-party Apple Silicon Mac used as the final release confidence machine.

3. `AS-UPGRADE-MAC`
   An Apple Silicon Mac with the previous public Sbobino version installed and used normally, so update and migration paths are exercised.

Useful but optional:

4. `AS-STRESS-MAC`
   Apple Silicon Mac used for failure injection: flaky network, low disk, interrupted first launch, interrupted update.

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

Machine: `AS-CLEAN-THIRD-MAC`

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

Machine: `AS-CLEAN-THIRD-MAC`

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

Machine: `AS-CLEAN-THIRD-MAC`

Steps:

1. Import a known short audio fixture with at least two speakers.
2. Run transcription with speaker diarization enabled.

Pass criteria:

- transcription completes
- diarization completes
- speaker segments are assigned in the timeline
- no pyannote runtime error is surfaced to the user

### E. Update-path validation

Machine: `AS-UPGRADE-MAC`

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

### F. First-launch failure recovery

Machine: `AS-STRESS-MAC` or controlled Apple Silicon test host

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
3. Clean-room install passes on `AS-CLEAN-THIRD-MAC`.
4. Warm restart passes on `AS-CLEAN-THIRD-MAC`.
5. Functional diarization smoke passes on `AS-CLEAN-THIRD-MAC`.
6. Update-path validation passes on `AS-UPGRADE-MAC`.
7. No mandatory scenario requires terminal repair or manual filesystem intervention.

If any item fails, the release is not distributable.

## Required Evidence Per Release

Each release should produce a short validation record with:

- version
- release URL
- machine class tested (`AS-CLEAN-THIRD-MAC`, `AS-UPGRADE-MAC`)
- macOS version
- outcome of each scenario
- timestamp
- tester
- blocking issue links if failed

Minimum evidence for the release thread or release notes folder:

- full `release_readiness.sh` success
- full `distribution_readiness.sh` success
- `AS-CLEAN-THIRD-MAC.validation-report.json` uploaded on the GitHub release with `status=passed`
- `AS-UPGRADE-MAC.validation-report.json` uploaded on the GitHub release with `status=passed`

## Future Matrix Extension

### macOS Intel x86_64

When Intel support is introduced, repeat the same matrix with:

- `INTEL-CLEAN-PRIMARY`
- `INTEL-THIRD-MAC`
- `INTEL-UPGRADE-MAC`

Additional checks:

- no Apple-Silicon-only assumptions
- correct runtime asset selection
- no MPS-only pyannote behavior assumptions

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
3. clean-room validation on a third Apple Silicon Mac
4. upgrade validation from previous public version on Apple Silicon
5. only then stable release

That gives us a real distribution process instead of a developer-machine check.
