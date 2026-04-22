#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: preflight_self_hosted_runner.sh <machine-class> [repo-slug]

Checks whether the current macOS machine is ready to act as a Sbobino release
self-hosted runner.
EOF
}

if [[ $# -lt 1 || $# -gt 2 ]]; then
  usage
  exit 1
fi

MACHINE_CLASS=$1
REPO_SLUG=${2:-pietroMastro92/Sbobino}
ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
FAILURES=()

labels_for_machine() {
  case "$1" in
    AS-PRIMARY)
      echo "self-hosted,macos,apple-silicon,as-primary"
      ;;
    AS-THIRD)
      echo "self-hosted,macos,apple-silicon,as-third"
      ;;
    INTEL-PRIMARY)
      echo "self-hosted,macos,x64,intel-primary"
      ;;
    *)
      usage
      exit 1
      ;;
  esac
}

expected_arch_for_machine() {
  case "$1" in
    AS-PRIMARY|AS-THIRD)
      echo "arm64"
      ;;
    INTEL-PRIMARY)
      echo "x86_64"
      ;;
    *)
      usage
      exit 1
      ;;
  esac
}

record_failure() {
  FAILURES+=("$1")
}

check_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    record_failure "Missing required command: $1"
  fi
}

if [[ "$(uname -s)" != "Darwin" ]]; then
  record_failure "Host OS is not macOS."
fi

CURRENT_ARCH=$(uname -m)
EXPECTED_ARCH=$(expected_arch_for_machine "$MACHINE_CLASS")
if [[ "$CURRENT_ARCH" != "$EXPECTED_ARCH" ]]; then
  record_failure "Machine class $MACHINE_CLASS requires arch $EXPECTED_ARCH, current host is $CURRENT_ARCH."
fi

for cmd in gh cargo python3 curl hdiutil ditto sw_vers xcode-select; do
  check_cmd "$cmd"
done

if ! gh auth status >/dev/null 2>&1; then
  record_failure "gh auth status failed. Authenticate GitHub CLI before using this machine as a runner."
fi

if ! xcode-select -p >/dev/null 2>&1; then
  record_failure "Xcode command line tools are not configured."
fi

AVAILABLE_GB=$(python3 - <<'PY'
import shutil
free = shutil.disk_usage("/").free / (1024 ** 3)
print(f"{free:.1f}")
PY
)
if ! python3 - <<'PY' "$AVAILABLE_GB"
import sys
available = float(sys.argv[1])
if available < 30.0:
    raise SystemExit(1)
PY
then
  record_failure "Less than 30 GB free on the system volume."
fi

if [[ "$MACHINE_CLASS" != "INTEL-PRIMARY" ]]; then
  if [[ -z "${SBOBINO_VALIDATION_FIXTURE_AUDIO:-}" ]]; then
    record_failure "SBOBINO_VALIDATION_FIXTURE_AUDIO is not set."
  elif [[ ! -f "${SBOBINO_VALIDATION_FIXTURE_AUDIO}" ]]; then
    record_failure "SBOBINO_VALIDATION_FIXTURE_AUDIO does not point to an existing file."
  fi
fi

RUNNERS_JSON=$(gh api "repos/${REPO_SLUG}/actions/runners" 2>/dev/null || echo '{"runners":[]}')
LABELS_EXPECTED=$(labels_for_machine "$MACHINE_CLASS")
if ! python3 - <<'PY' "$RUNNERS_JSON" "$LABELS_EXPECTED"
import json
import sys

runners = json.loads(sys.argv[1]).get("runners", [])

def normalize_label(value: str) -> str:
    return str(value or "").strip().casefold()

expected = {normalize_label(value) for value in sys.argv[2].split(",")}

for runner in runners:
    labels = {
        normalize_label(label.get("name"))
        for label in runner.get("labels", [])
        if label.get("name")
    }
    if expected.issubset(labels) and runner.get("status") == "online":
        print(f"online:{runner.get('name','unknown')}")
        raise SystemExit(0)

raise SystemExit(1)
PY
then
  record_failure "No online GitHub runner is currently registered with labels: $LABELS_EXPECTED"
fi

if [[ ${#FAILURES[@]} -gt 0 ]]; then
  echo "Runner preflight FAILED for $MACHINE_CLASS" >&2
  for failure in "${FAILURES[@]}"; do
    echo "  - $failure" >&2
  done
  exit 1
fi

cat <<EOF
Runner preflight passed.
  repo:     $REPO_SLUG
  machine:  $MACHINE_CLASS
  arch:     $CURRENT_ARCH
  labels:   $(labels_for_machine "$MACHINE_CLASS")
  free GB:  $AVAILABLE_GB
EOF
