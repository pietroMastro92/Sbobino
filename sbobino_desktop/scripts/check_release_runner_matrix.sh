#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: check_release_runner_matrix.sh [repo-slug]

Verifies that the three required self-hosted runner classes for Sbobino release
validation are online on GitHub:
  - AS-PRIMARY
  - AS-THIRD
  - INTEL-PRIMARY
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

RUNNERS_JSON=$(gh api "repos/${REPO_SLUG}/actions/runners")

python3 - <<'PY' "$RUNNERS_JSON" "$REPO_SLUG"
import json
import sys

runners = json.loads(sys.argv[1]).get("runners", [])
repo_slug = sys.argv[2]

def normalize_label(value: str) -> str:
    return str(value or "").strip().casefold()

required = {
    "AS-PRIMARY": {"self-hosted", "macos", "apple-silicon", "as-primary"},
    "AS-THIRD": {"self-hosted", "macos", "apple-silicon", "as-third"},
    "INTEL-PRIMARY": {"self-hosted", "macos", "x64", "intel-primary"},
}

matched = {}
for machine_class, labels_expected in required.items():
    for runner in runners:
        labels = {
            normalize_label(label.get("name"))
            for label in runner.get("labels", [])
            if label.get("name")
        }
        if labels_expected.issubset(labels) and runner.get("status") == "online":
            matched[machine_class] = {
                "name": runner.get("name", "unknown"),
                "busy": bool(runner.get("busy")),
                "labels": sorted(
                    label.get("name")
                    for label in runner.get("labels", [])
                    if label.get("name")
                ),
            }
            break

missing = [machine_class for machine_class in required if machine_class not in matched]
if missing:
    print(f"Release runner matrix is NOT ready for {repo_slug}.", file=sys.stderr)
    for machine_class in missing:
        print(f"  - missing online runner for {machine_class}", file=sys.stderr)
    raise SystemExit(1)

print(f"Release runner matrix is ready for {repo_slug}.")
for machine_class in ("AS-PRIMARY", "AS-THIRD", "INTEL-PRIMARY"):
    runner = matched[machine_class]
    busy = "busy" if runner["busy"] else "idle"
    print(f"  - {machine_class}: {runner['name']} ({busy})")
PY
