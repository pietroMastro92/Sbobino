# Release Validation Runners

## Goal

Use GitHub Actions as the orchestrator for distribution-critical clean-room validation.
Promotion-blocking clean-room gates run on **GitHub-hosted** runners (no local UTM/self-hosted VM required).
Self-hosted Macs remain optional for upgrade-path checks (`AS-PRIMARY`).

## Hosted clean-room matrix (required for stable promotion)

| Machine class | Hosted runner | Report asset |
| --- | --- | --- |
| `AS-THIRD` | `macos-14` (arm64) | `AS-THIRD.validation-report.json` |
| `INTEL-PRIMARY` | `macos-15-intel` (x86_64) | `INTEL-PRIMARY.validation-report.json` |
| `WINDOWS-PRIMARY` | `windows-2025` (x86_64) | `WINDOWS-PRIMARY.validation-report.json` |

Dispatch via:

```bash
./scripts/run_release_vm_gate.sh <version> pietroMastro92/Sbobino AS-THIRD
./scripts/run_release_vm_gate.sh <version> pietroMastro92/Sbobino INTEL-PRIMARY
./scripts/run_release_vm_gate.sh <version> pietroMastro92/Sbobino WINDOWS-PRIMARY
```

Set `SBOBINO_RELEASE_VM_WORKFLOW_REF=<branch>` when the workflow/scripts live on a branch newer than the release tag.

## Optional self-hosted runners

### `AS-PRIMARY`

- machine: primary Apple Silicon Mac used by the team
- labels:
  - `self-hosted`
  - `macos`
  - `apple-silicon`
  - `as-primary`
- purpose:
  - real upgrade-path validation from the latest stable public release
  - warm restart validation
  - diarization smoke after update

### Legacy local labels (superseded for clean-room)

`AS-THIRD` and `INTEL-PRIMARY` historically used self-hosted Macs / UTM VMs.
Clean-room promotion gates now run on the hosted matrix above; keep local labels only if you still want private offline experimentation.

## GitHub runner registration

Register each runner at repo or organization scope with the exact label sets above.

Recommended helper:

```bash
./scripts/install_self_hosted_runner_macos.sh <MACHINE_CLASS> pietroMastro92/Sbobino
```

Recommended service mode:

- run the GitHub runner as a persistent launch agent or service
- enable automatic start after reboot
- keep the runner online only on trusted machines

## Security boundaries

- use these runners only from trusted workflows and trusted tags
- do not expose secrets to workflows triggered from forks
- keep release publication and promote flows on `workflow_dispatch`
- prefer repository or organization variables for non-secret paths and fixtures

## Required local tooling

Install and keep available on every runner:

- Xcode command line tools
- Rust toolchain
- `cargo`
- `python3`
- `curl`
- `hdiutil`
- `ditto`

Apple Silicon runners also need enough free disk for the full first-launch runtime and pyannote installation.

After installation, run:

```bash
./scripts/preflight_self_hosted_runner.sh <MACHINE_CLASS> pietroMastro92/Sbobino
```

## Workspace hygiene

Each self-hosted job should start from a clean workspace.

Minimum practice:

- remove old checkout directories before the runner starts the next job
- keep validation output inside the checked-out repo or the runner temp directory
- do not reuse old validation JSON files between runs

## Validation fixture

Apple Silicon machine validation requires:

- environment or repository variable: `SBOBINO_VALIDATION_FIXTURE_AUDIO`

This must point to an absolute path on the runner host for a short audio file with at least two speakers.

The validation flow is fail-closed:

- if the fixture is missing, `AS-PRIMARY` and `AS-THIRD` fail
- the candidate must not be promoted to stable

## Clean-room guidance for `AS-THIRD`

Preferred setup:

- use a dedicated macOS user account only for release validation

Minimum acceptable setup:

- remove `/Applications/Sbobino.app`
- remove `~/Library/Application Support/com.sbobino.desktop`
- ensure the validation does not depend on Homebrew or developer-installed runtime state

## Expected workflow contract

1. Hosted GitHub Actions builds the candidate and publishes the prerelease.
2. Hosted GitHub Actions runs `distribution_readiness.sh` and uploads `distribution-readiness-proof.json`.
3. Self-hosted runners validate the exact public release assets and upload:
   - `AS-PRIMARY.validation-report.json`
   - `AS-THIRD.validation-report.json`
   - `INTEL-PRIMARY.validation-report.json`
4. Stable promotion remains manual and blocked unless all required reports are present and valid.

## First real run

Use [first-real-candidate-runbook.md](first-real-candidate-runbook.md) for the first end-to-end live candidate on GitHub.
