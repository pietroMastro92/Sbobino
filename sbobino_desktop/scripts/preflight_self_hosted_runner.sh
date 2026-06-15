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
MIN_FREE_GB=${SBOBINO_RUNNER_MIN_FREE_GB:-12.0}

validation_fixture_audio() {
  if [[ -n "${SBOBINO_VALIDATION_FIXTURE_AUDIO:-}" ]]; then
    printf '%s\n' "$SBOBINO_VALIDATION_FIXTURE_AUDIO"
    return 0
  fi
  case "$MACHINE_CLASS" in
    AS-THIRD)
      printf '%s\n' "$HOME/Fixtures/as-third-diarization.wav"
      ;;
    *)
      printf '%s\n' ""
      ;;
  esac
}

validation_fixture_audio() {
  if [[ -n "${SBOBINO_VALIDATION_FIXTURE_AUDIO:-}" ]]; then
    printf '%s\n' "$SBOBINO_VALIDATION_FIXTURE_AUDIO"
    return 0
  fi
  case "$MACHINE_CLASS" in
    AS-THIRD)
      printf '%s\n' "$HOME/Fixtures/as-third-diarization.wav"
      ;;
    *)
      printf '%s\n' ""
      ;;
  esac
}

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
if ! python3 - <<'PY' "$AVAILABLE_GB" "$MIN_FREE_GB"
import sys
available = float(sys.argv[1])
minimum = float(sys.argv[2])
if available < minimum:
    raise SystemExit(1)
PY
then
  record_failure "Less than ${MIN_FREE_GB} GB free on the system volume."
fi

if [[ "$MACHINE_CLASS" != "INTEL-PRIMARY" ]]; then
  FIXTURE_AUDIO=$(validation_fixture_audio)
  if [[ -z "${FIXTURE_AUDIO:-}" ]]; then
    record_failure "SBOBINO_VALIDATION_FIXTURE_AUDIO is not set."
  elif [[ ! -f "${FIXTURE_AUDIO}" ]]; then
    record_failure "SBOBINO_VALIDATION_FIXTURE_AUDIO does not point to an existing file: $FIXTURE_AUDIO"
  fi
fi

LABELS_EXPECTED=$(labels_for_machine "$MACHINE_CLASS")
if [[ "${GITHUB_ACTIONS:-}" == "true" ]]; then
  :
else
  RUNNER_MATCH_FOUND=0
  RUNNER_LINES=$(gh api "repos/${REPO_SLUG}/actions/runners" \
    --jq '.runners[] | select(.status == "online") | [.name, ([.labels[].name | ascii_downcase] | join(","))] | @tsv' \
    2>/dev/null || true)
  while IFS=$'\t' read -r runner_name runner_labels; do
    [[ -z "${runner_name:-}" ]] && continue
    IFS=',' read -ra expected_labels <<<"$LABELS_EXPECTED"
    runner_label_set=",$runner_labels,"
    missing_label=0
    for expected_label in "${expected_labels[@]}"; do
      if [[ "$runner_label_set" != *",$expected_label,"* ]]; then
        missing_label=1
        break
      fi
    done
    if [[ "$missing_label" == "0" ]]; then
      echo "online:$runner_name"
      RUNNER_MATCH_FOUND=1
      break
    fi
  done <<<"$RUNNER_LINES"
  if [[ "$RUNNER_MATCH_FOUND" != "1" ]]; then
    record_failure "No online GitHub runner is currently registered with labels: $LABELS_EXPECTED"
  fi
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
  min GB:   $MIN_FREE_GB
EOF
