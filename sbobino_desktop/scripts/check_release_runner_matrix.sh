#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: check_release_runner_matrix.sh [repo-slug]

Verifies that the release clean-room gates can run fully on GitHub-hosted runners.
Self-hosted AS-PRIMARY remains optional for upgrade-path checks.

Hosted machine classes:
  - AS-THIRD        -> macos-14 (arm64)
  - INTEL-PRIMARY   -> macos-15-intel (x86_64)
  - WINDOWS-PRIMARY -> windows-2025 (x86_64)
EOF
}

if [[ $# -gt 1 ]]; then
  usage
  exit 1
fi

REPO_SLUG=${1:-pietroMastro92/Sbobino}

if ! command -v gh >/dev/null 2>&1; then
  echo "Missing required command: gh" >&2
  exit 1
fi

if ! command -v python3 >/dev/null 2>&1; then
  echo "Missing required command: python3" >&2
  exit 1
fi

WORKFLOW_JSON=$(
  gh api "repos/${REPO_SLUG}/contents/.github/workflows/release-vm-validation.yml" --jq '.content' \
    | python3 -c 'import base64,sys; print(base64.b64decode(sys.stdin.read().strip()).decode("utf-8"))'
)

python3 - <<'PY' "$WORKFLOW_JSON" "$REPO_SLUG"
import re
import sys

workflow = sys.argv[1]
repo_slug = sys.argv[2]

required = {
    "AS-THIRD": {
        "machine_class": "AS-THIRD",
        "runs_on": "macos-14",
        "report": "AS-THIRD.validation-report.json",
    },
    "INTEL-PRIMARY": {
        "machine_class": "INTEL-PRIMARY",
        "runs_on": "macos-15-intel",
        "report": "INTEL-PRIMARY.validation-report.json",
    },
    "WINDOWS-PRIMARY": {
        "machine_class": "WINDOWS-PRIMARY",
        "runs_on": "windows-2025",
        "report": "WINDOWS-PRIMARY.validation-report.json",
    },
}

missing = []
for machine_class, expectation in required.items():
    if f"- {expectation['machine_class']}" not in workflow and f"'{expectation['machine_class']}'" not in workflow:
        missing.append(f"{machine_class}: workflow choice missing")
        continue
    if expectation["runs_on"] not in workflow:
        missing.append(f"{machine_class}: runs-on '{expectation['runs_on']}' missing")
    if expectation["report"] not in workflow:
        missing.append(f"{machine_class}: report asset '{expectation['report']}' missing")

if "machine_class" not in workflow or "workflow_dispatch" not in workflow:
    missing.append("workflow_dispatch machine_class input missing")

if missing:
    print(f"Hosted release runner matrix is NOT ready for {repo_slug}.", file=sys.stderr)
    for item in missing:
        print(f"  - {item}", file=sys.stderr)
    raise SystemExit(1)

print(f"Hosted release runner matrix is ready for {repo_slug}.")
for machine_class, expectation in required.items():
    print(f"  - {machine_class}: github-hosted ({expectation['runs_on']})")
print("  - AS-PRIMARY: optional self-hosted upgrade-path runner (not required for stable promotion)")
PY
