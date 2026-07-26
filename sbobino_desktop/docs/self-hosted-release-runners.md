# Optional Self-Hosted Release Diagnostics

Stable Sbobino releases are built and validated by the native GitHub-hosted runner matrix documented in [distribution-validation-plan.md](distribution-validation-plan.md).

Self-hosted Macs are optional diagnostic environments only. They are not required for candidate publication or stable promotion, and their reports are not mandatory release assets.

The remaining helper scripts can register and preflight `AS-PRIMARY` or `INTEL-PRIMARY` when a maintainer wants additional hardware-specific investigation:

```bash
./scripts/install_self_hosted_runner_macos.sh <MACHINE_CLASS> pietroMastro92/Sbobino
./scripts/preflight_self_hosted_runner.sh <MACHINE_CLASS> pietroMastro92/Sbobino
```

Do not expose release secrets to workflows triggered from forks. Keep optional runners offline when they are not being used for trusted diagnostics.
