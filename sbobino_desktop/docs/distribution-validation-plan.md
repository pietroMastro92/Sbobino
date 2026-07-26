# Distribution Validation Plan

## Authority

GitHub Actions is the release-validation authority. Stable promotion depends on native hosted runners validating the exact public prerelease assets.

## Native matrix

| Platform | Runner | Target | Required validation |
| --- | --- | --- | --- |
| macOS Apple Silicon | `macos-14` | `aarch64-apple-darwin` | public DMG install, bundle architecture, portability launch, updater wiring |
| macOS Intel | `macos-15-intel` | `x86_64-apple-darwin` | public DMG install, bundle architecture, portability launch, updater wiring |
| Windows | `windows-2025` | `x86_64-pc-windows-msvc` | public NSIS install, runtime/DLL audit, opaque GUI, invisible helper processes |

## Mandatory candidate proof assets

- `release-readiness-proof.json`
- `distribution-readiness-proof.json`
- `intel-distribution-readiness-proof.json`
- `windows-distribution-readiness-proof.json`
- `portability-smoke-report.json`
- `intel-portability-smoke-report.json`
- `windows-gui-smoke-report.json`

Every proof must have `status=passed`, match the requested version, and refer to the same public release tag where that field is defined.

## macOS gates

Both macOS runners must:

1. download the matching public DMG;
2. install the app from that DMG;
3. verify the app executable and packaged runtime are native for the runner architecture;
4. reject Homebrew or developer-machine library paths;
5. launch the installed app and confirm it remains alive;
6. validate the matching updater artifact and signature.

## Windows gates

The Windows runner must:

1. download and silently install the public NSIS package;
2. validate all runtime executables and app-local DLL dependencies;
3. launch the installed application;
4. exercise FFmpeg, Whisper, Parakeet, and Python helper probes;
5. report zero visible console windows;
6. report exactly one main Sbobino window;
7. confirm the main window is opaque;
8. validate the Windows updater artifact and signature.

## Updater contract

`latest.json` must contain exactly the supported native updater targets:

- `darwin-aarch64`
- `darwin-x86_64`
- `windows-x86_64`

Each entry must reference the corresponding signed updater payload from the same release.

## Promotion rule

`promote_candidate_release.sh` fails closed if a mandatory proof is missing, malformed, mismatched, or not passed. Once all hosted proofs pass, promotion:

1. marks the candidate stable;
2. marks it as GitHub Latest;
3. retains the current and immediately previous stable releases for rollback.

Physical-machine validation may still be run as an optional diagnostic, but it is not part of the stable promotion contract.
