#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: check_release_runner_class.sh <machine-class> [repo-slug]

Verifies that one required self-hosted runner class for Sbobino release
validation is online on GitHub. Supported machine classes:
  - AS-PRIMARY
  - AS-THIRD
  - INTEL-PRIMARY
EOF
}

if [[ $# -lt 1 || $# -gt 2 ]]; then
  usage
  exit 1
fi

MACHINE_CLASS=$1
REPO_SLUG=${2:-pietroMastro92/Sbobino}

if ! command -v gh >/dev/null 2>&1; then
  echo "Missing required command: gh" >&2
  exit 1
fi

RUNNERS_JSON=$(gh api "repos/${REPO_SLUG}/actions/runners")

python3 - <<'PY' "$RUNNERS_JSON" "$REPO_SLUG" "$MACHINE_CLASS"
import json
import sys

runners = json.loads(sys.argv[1]).get("runners", [])
repo_slug = sys.argv[2]
machine_class = sys.argv[3]
required = {
    "AS-PRIMARY": {"self-hosted", "macos", "apple-silicon", "as-primary"},
    "AS-THIRD": {"self-hosted", "macos", "apple-silicon", "as-third"},
    "INTEL-PRIMARY": {"self-hosted", "macos", "x64", "intel-primary"},
}

if machine_class not in required:
    print(f"Unsupported machine class: {machine_class}", file=sys.stderr)
    raise SystemExit(1)

labels_expected = required[machine_class]
matches = []
for runner in runners:
    labels_display = {label.get("name") for label in runner.get("labels", []) if label.get("name")}
    labels = {label.lower() for label in labels_display}
    if labels_expected.issubset(labels):
        matches.append(
            {
                "name": runner.get("name", "unknown"),
                "status": runner.get("status", "unknown"),
                "busy": bool(runner.get("busy")),
                "labels": sorted(labels_display),
            }
        )

online = [runner for runner in matches if runner["status"] == "online"]
if not online:
    print(f"Release runner class {machine_class} is NOT ready for {repo_slug}.", file=sys.stderr)
    if matches:
        for runner in matches:
            print(
                f"  - {runner['name']}: {runner['status']}"
                f" ({'busy' if runner['busy'] else 'idle'})",
                file=sys.stderr,
            )
    else:
        print(f"  - no runner with labels: {', '.join(sorted(labels_expected))}", file=sys.stderr)
    raise SystemExit(1)

runner = online[0]
busy = "busy" if runner["busy"] else "idle"
print(f"Release runner class {machine_class} is ready for {repo_slug}.")
print(f"  - {runner['name']}: online ({busy})")
PY
