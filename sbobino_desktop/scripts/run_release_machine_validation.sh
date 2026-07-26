#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: run_release_machine_validation.sh <machine-class> <version> [repo-slug] [report-path]

Runs release validation on a self-hosted machine and writes a machine-readable
JSON report. Supported machine classes:
  - AS-PRIMARY
  - INTEL-PRIMARY

Environment variables:
  SBOBINO_VALIDATION_DATA_DIR        Override app data dir
  SBOBINO_VALIDATION_APP_PATH        Override installed app path
  SBOBINO_VALIDATION_FIXTURE_AUDIO   Audio fixture used for diarization smoke
  SBOBINO_VALIDATION_PARAKEET_MODEL  Parakeet GGUF used for optional app smoke
  SBOBINO_VALIDATION_TIMEOUT_SECONDS Timeout for setup/runtime readiness waits (default: 2400)
  SBOBINO_VALIDATION_PRIVACY_VERSION Privacy policy version to seed into settings.json
EOF
}

if [[ $# -lt 2 || $# -gt 4 ]]; then
  usage
  exit 1
fi

MACHINE_CLASS=$1
VERSION=$2
REPO_SLUG=${3:-pietroMastro92/Sbobino}
REPORT_PATH=${4:-"$(pwd)/${MACHINE_CLASS}.validation-report.json"}

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
DATA_DIR=${SBOBINO_VALIDATION_DATA_DIR:-"$HOME/Library/Application Support/com.sbobino.desktop"}
APP_PATH=${SBOBINO_VALIDATION_APP_PATH:-"/Applications/Sbobino.app"}
default_fixture_audio() {
  case "${MACHINE_CLASS:-}" in
    INTEL-PRIMARY)
      printf '%s\n' "$HOME/Fixtures/intel-primary-diarization.wav"
      ;;
    *)
      printf '%s\n' ""
      ;;
  esac
}

FIXTURE_AUDIO=${SBOBINO_VALIDATION_FIXTURE_AUDIO:-$(default_fixture_audio)}
PARAKEET_MODEL=${SBOBINO_VALIDATION_PARAKEET_MODEL:-tdt-0.6b-v3-q4_k.gguf}
TIMEOUT_SECONDS=${SBOBINO_VALIDATION_TIMEOUT_SECONDS:-2400}
PRIVACY_POLICY_VERSION=${SBOBINO_VALIDATION_PRIVACY_VERSION:-2026-04-03}
TAG="v$VERSION"
RELEASE_URL="https://github.com/$REPO_SLUG/releases/tag/$TAG"
BASE_DOWNLOAD_URL="https://github.com/$REPO_SLUG/releases/download/$TAG"
SETUP_REPORT_PATH="$DATA_DIR/setup-report.json"
SETTINGS_PATH="$DATA_DIR/settings.json"
TMP_DIR=$(mktemp -d)
COMMIT_SHA=${GITHUB_SHA:-$(git -C "$ROOT_DIR/.." rev-parse HEAD 2>/dev/null || true)}
OS_NAME="macOS"
OS_VERSION="$(sw_vers -productVersion) ($(uname -m))"
TESTER=${GITHUB_ACTOR:-$(whoami)}

FINAL_STATUS="failed"
REPORT_NOTES=""

SCENARIO_UPDATE_PATH_VALIDATION="pending"
SCENARIO_CLEAN_ROOM_INSTALL="pending"
SCENARIO_WARM_RESTART="pending"
SCENARIO_FUNCTIONAL_DIARIZATION_SMOKE="pending"
SCENARIO_FUNCTIONAL_PARAKEET_SMOKE="pending"
SCENARIO_RELEASE_METADATA_VALIDATION="pending"
SCENARIO_BOOTSTRAP_LAYER_VALIDATION="pending"
SCENARIO_ARM64_BINARY_EXECUTION="pending"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

truthy_env_value() {
  case "${1:-}" in
    1|true|TRUE|yes|YES|on|ON) return 0 ;;
    *) return 1 ;;
  esac
}

is_base_apple_m1_host() {
  local brand
  brand=$(/usr/sbin/sysctl -n machdep.cpu.brand_string 2>/dev/null || true)
  [[ "$brand" == *"Apple M1"* && "$brand" != *"M1 Pro"* && "$brand" != *"M1 Max"* && "$brand" != *"M1 Ultra"* ]]
}

parakeet_release_smoke_requires_gpu() {
  if [[ -n "${SBOBINO_REQUIRE_PARAKEET_GPU:-}" ]]; then
    printf '%s\n' "$SBOBINO_REQUIRE_PARAKEET_GPU"
  elif is_base_apple_m1_host && ! truthy_env_value "${SBOBINO_PARAKEET_FORCE_METAL:-}"; then
    printf '%s\n' "0"
  else
    printf '%s\n' "1"
  fi
}

need_cmd cargo
need_cmd curl
need_cmd ditto
need_cmd hdiutil
need_cmd lipo
need_cmd open
need_cmd python3
need_cmd sw_vers
need_cmd uname

write_report() {
  python3 - <<'PY' \
    "$REPORT_PATH" \
    "$MACHINE_CLASS" \
    "$VERSION" \
    "$TAG" \
    "$RELEASE_URL" \
    "$COMMIT_SHA" \
    "$FINAL_STATUS" \
    "$TESTER" \
    "$OS_NAME" \
    "$OS_VERSION" \
    "$REPORT_NOTES" \
    "$SCENARIO_UPDATE_PATH_VALIDATION" \
    "$SCENARIO_CLEAN_ROOM_INSTALL" \
    "$SCENARIO_WARM_RESTART" \
    "$SCENARIO_FUNCTIONAL_DIARIZATION_SMOKE" \
    "$SCENARIO_FUNCTIONAL_PARAKEET_SMOKE" \
    "$SCENARIO_RELEASE_METADATA_VALIDATION" \
    "$SCENARIO_BOOTSTRAP_LAYER_VALIDATION" \
    "$SCENARIO_ARM64_BINARY_EXECUTION"
import json
import os
import sys
from datetime import datetime, timezone
from pathlib import Path

(
    report_path,
    machine_class,
    version,
    tag,
    release_url,
    commit_sha,
    status,
    tester,
    os_name,
    os_version,
    notes,
    update_path_validation,
    clean_room_install,
    warm_restart,
    functional_diarization_smoke,
    functional_parakeet_smoke,
    release_metadata_validation,
    bootstrap_layer_validation,
    arm64_binary_execution,
) = sys.argv[1:20]

definitions = {
    "AS-PRIMARY": {
        "required": [
            "update_path_validation",
            "warm_restart",
            "functional_diarization_smoke",
        ],
        "results": {
            "update_path_validation": update_path_validation,
            "warm_restart": warm_restart,
            "functional_diarization_smoke": functional_diarization_smoke,
        },
        "runner_label": "self-hosted,macOS,ARM64,apple-silicon,as-primary",
    },
    "INTEL-PRIMARY": {
        "required": [
            "release_metadata_validation",
            "bootstrap_layer_validation",
            "clean_room_install",
            "warm_restart",
            "functional_diarization_smoke",
        ],
        "results": {
            "release_metadata_validation": release_metadata_validation,
            "bootstrap_layer_validation": bootstrap_layer_validation,
            "clean_room_install": clean_room_install,
            "warm_restart": warm_restart,
            "functional_diarization_smoke": functional_diarization_smoke,
        },
        "runner_label": "self-hosted,macOS,X64,intel-primary",
    },
}

definition = definitions[machine_class]
runner_label = os.environ.get("SBOBINO_VALIDATION_RUNNER_LABEL") or definition["runner_label"]
payload = {
    "schema_version": 1,
    "version": version,
    "release_tag": tag,
    "release_url": release_url,
    "commit_sha": commit_sha,
    "machine_class": machine_class,
    "status": status,
    "tester": tester,
    "os_name": os_name,
    "os_version": os_version,
    "runner_label": runner_label,
    "tested_at_utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "notes": notes,
    "required_scenarios": definition["required"],
    "scenario_results": definition["results"],
}

Path(report_path).write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
PY
}

fail_validation() {
  REPORT_NOTES=$1
  FINAL_STATUS="failed"
  write_report
  echo "$REPORT_NOTES" >&2
  exit 1
}

record_success() {
  FINAL_STATUS=$1
  REPORT_NOTES=$2
  write_report
}

quit_app() {
  osascript -e 'tell application "Sbobino" to quit' >/dev/null 2>&1 || true
  pkill -f "/Applications/Sbobino.app/Contents/MacOS/Sbobino" >/dev/null 2>&1 || true
  sleep 2
}

download_asset() {
  local version_arg=$1
  local asset_name=$2
  local destination=$3
  local url="https://github.com/$REPO_SLUG/releases/download/v${version_arg}/${asset_name}"
  curl \
    --fail \
    --location \
    --retry 3 \
    --retry-delay 2 \
    --silent \
    --show-error \
    --output "$destination" \
    "$url"
}

install_app_from_dmg() {
  local version_arg=$1
  local arch=${2:-aarch64}
  local dmg_path="$TMP_DIR/Sbobino_${version_arg}_${arch}.dmg"
  local mount_dir="$TMP_DIR/mount-${version_arg}-${arch}"
  mkdir -p "$mount_dir"
  download_asset "$version_arg" "Sbobino_${version_arg}_${arch}.dmg" "$dmg_path"
  hdiutil attach "$dmg_path" -nobrowse -mountpoint "$mount_dir" -quiet
  rm -rf "$APP_PATH"
  /usr/bin/ditto "$mount_dir/Sbobino.app" "$APP_PATH"
  hdiutil detach "$mount_dir" -quiet || true
  xattr -dr com.apple.quarantine "$APP_PATH" >/dev/null 2>&1 || true
}

clear_install_state() {
  quit_app
  rm -rf "$APP_PATH"
  rm -rf "$DATA_DIR"
}

seed_privacy_acceptance() {
  mkdir -p "$DATA_DIR"
  python3 - <<'PY' "$SETTINGS_PATH" "$PRIVACY_POLICY_VERSION"
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

settings_path = Path(sys.argv[1])
privacy_version = sys.argv[2]

payload = {}
if settings_path.exists():
    try:
        payload = json.loads(settings_path.read_text(encoding="utf-8"))
    except Exception:
        payload = {}

general = payload.setdefault("general", {})
general["privacy_policy_version_accepted"] = privacy_version
general["privacy_policy_accepted_at"] = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")

settings_path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
PY
}

set_speaker_diarization_enabled() {
  local enabled=${1:-0}
  mkdir -p "$DATA_DIR"
  python3 - <<'PY' "$SETTINGS_PATH" "$enabled"
import json
import sys
from pathlib import Path

settings_path = Path(sys.argv[1])
enabled = sys.argv[2] == "1"

payload = {}
if settings_path.exists():
    try:
        payload = json.loads(settings_path.read_text(encoding="utf-8"))
    except Exception:
        payload = {}

transcription = payload.setdefault("transcription", {})
speaker = transcription.setdefault("speaker_diarization", {})
speaker["enabled"] = enabled
speaker.setdefault("device", "cpu")
speaker.setdefault("speaker_colors", {})

settings_path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
PY
}

launch_app() {
  open "$APP_PATH"
}

wait_for_setup_report_success() {
  local timeout=$1
  local started_at
  started_at=$(date +%s)

  while true; do
    if python3 - <<'PY' "$SETUP_REPORT_PATH"
import json
import sys
from pathlib import Path

report_path = Path(sys.argv[1])
if not report_path.exists():
    raise SystemExit(2)

report = json.loads(report_path.read_text(encoding="utf-8"))
if (
    report.get("setup_complete") is True
    and str(report.get("final_reason_code", "")).strip() == "setup_complete"
    and not str(report.get("final_error", "") or "").strip()
):
    raise SystemExit(0)

steps = report.get("steps") or []
if any(str(step.get("status", "")).strip() == "failed" for step in steps):
    raise SystemExit(3)

if str(report.get("final_error", "") or "").strip():
    raise SystemExit(3)

raise SystemExit(2)
PY
    then
      return 0
    else
      case $? in
        2) ;;
        3)
          local report_body=""
          if [[ -f "$SETUP_REPORT_PATH" ]]; then
            report_body=$(cat "$SETUP_REPORT_PATH")
          fi
          fail_validation "setup-report.json indicates a failed first-launch setup: ${report_body}"
          ;;
        *)
          fail_validation "Unexpected error while reading setup-report.json."
          ;;
      esac
    fi

    if (( $(date +%s) - started_at > timeout )); then
      fail_validation "Timed out waiting for setup-report.json to report setup_complete."
    fi
    sleep 10
  done
}

capture_runtime_health() {
  local output_path=$1
  cargo run --quiet \
    --manifest-path "$ROOT_DIR/Cargo.toml" \
    -p sbobino-infrastructure \
    --bin runtime_health_snapshot \
    -- \
    --data-dir "$DATA_DIR" \
    --resources-dir "$APP_PATH/Contents/Resources" \
    --pretty >"$output_path"
}

wait_for_runtime_health_ready() {
  local timeout=$1
  local require_pyannote=$2
  local started_at
  started_at=$(date +%s)

  while true; do
    local snapshot_path="$TMP_DIR/runtime-health.json"
    if capture_runtime_health "$snapshot_path"; then
      if python3 - <<'PY' "$snapshot_path" "$require_pyannote"
import json
import sys

snapshot = json.load(open(sys.argv[1], encoding="utf-8"))
require_pyannote = sys.argv[2] == "1"
health = snapshot["health"]
if not health.get("setup_complete"):
    raise SystemExit(2)
if require_pyannote and not health.get("pyannote", {}).get("ready"):
    raise SystemExit(2)
raise SystemExit(0)
PY
      then
        return 0
      fi
    fi

    if (( $(date +%s) - started_at > timeout )); then
      local snapshot_body=""
      if [[ -f "$TMP_DIR/runtime-health.json" ]]; then
        snapshot_body=$(cat "$TMP_DIR/runtime-health.json")
      fi
      fail_validation "Timed out waiting for runtime health readiness. Last snapshot: ${snapshot_body}"
    fi
    sleep 10
  done
}

run_diarization_smoke() {
  if [[ -z "${FIXTURE_AUDIO// }" ]]; then
    fail_validation "SBOBINO_VALIDATION_FIXTURE_AUDIO is required for Apple Silicon diarization smoke."
  fi
  if [[ ! -f "$FIXTURE_AUDIO" ]]; then
    fail_validation "Diarization fixture not found at '$FIXTURE_AUDIO'."
  fi

  local python_path="$DATA_DIR/runtime/pyannote/python/bin/python3"
  local model_dir="$DATA_DIR/runtime/pyannote/model"
  local output_path="$TMP_DIR/pyannote-smoke.json"

  if [[ ! -x "$python_path" ]]; then
    fail_validation "Managed pyannote python was not installed at '$python_path'."
  fi
  if [[ ! -d "$model_dir" ]]; then
    fail_validation "Managed pyannote model dir was not installed at '$model_dir'."
  fi

  PATH="$DATA_DIR/bin:$PATH" \
    "$python_path" \
    "$ROOT_DIR/scripts/pyannote_diarize.py" \
    --audio-path "$FIXTURE_AUDIO" \
    --model-path "$model_dir" \
    --device cpu >"$output_path"

  if ! python3 - <<'PY' "$output_path"
import json
import sys

payload = json.load(open(sys.argv[1], encoding="utf-8"))
speakers = payload.get("speakers") or []
labels = {speaker.get("speaker_label") for speaker in speakers if speaker.get("speaker_label")}
if len(labels) < 1:
    raise SystemExit(1)
PY
  then
    fail_validation "Pyannote smoke test did not produce speaker labels."
  fi
}

run_parakeet_transcription_smoke() {
  if [[ -z "${FIXTURE_AUDIO// }" ]]; then
    fail_validation "SBOBINO_VALIDATION_FIXTURE_AUDIO is required for the Parakeet smoke."
  fi
  if [[ ! -f "$FIXTURE_AUDIO" ]]; then
    fail_validation "Parakeet smoke audio fixture not found at '$FIXTURE_AUDIO'."
  fi

  local parakeet_cli="$DATA_DIR/bin/parakeet-cli"
  local parakeet_models_dir="$DATA_DIR/parakeet-models"
  local parakeet_model_path="$parakeet_models_dir/$PARAKEET_MODEL"
  local smoke_log="$TMP_DIR/parakeet-real-smoke.log"
  local compare_log="$TMP_DIR/parakeet-compare.log"
  local require_parakeet_gpu
  local skip_parakeet_cpu
  require_parakeet_gpu=$(parakeet_release_smoke_requires_gpu)
  if [[ "$require_parakeet_gpu" == "1" ]]; then
    skip_parakeet_cpu=1
  else
    skip_parakeet_cpu=0
  fi

  if [[ ! -x "$parakeet_cli" ]]; then
    fail_validation "Managed Parakeet CLI was not installed at '$parakeet_cli'."
  fi
  if [[ ! -f "$parakeet_model_path" ]]; then
    fail_validation "Managed Parakeet model was not installed at '$parakeet_model_path'."
  fi

  if ! PATH="$DATA_DIR/bin:$PATH" \
    SBOBINO_PARAKEET_CLI="$parakeet_cli" \
    SBOBINO_PARAKEET_MODELS_DIR="$parakeet_models_dir" \
    SBOBINO_PARAKEET_MODEL="$PARAKEET_MODEL" \
    SBOBINO_PARAKEET_AUDIO="$FIXTURE_AUDIO" \
    SBOBINO_ASR_SAMPLE=artemis \
    "$ROOT_DIR/scripts/smoke_parakeet_real.sh" >"$smoke_log" 2>&1; then
    fail_validation "Parakeet real smoke failed. Last log lines: $(tail -80 "$smoke_log")"
  fi

  if ! PATH="$DATA_DIR/bin:$PATH" \
    SBOBINO_APP_SUPPORT_DIR="$DATA_DIR" \
    SBOBINO_PARAKEET_CLI="$parakeet_cli" \
    SBOBINO_PARAKEET_MODELS_DIR="$parakeet_models_dir" \
    SBOBINO_PARAKEET_MODEL="$PARAKEET_MODEL" \
    SBOBINO_ARTEMIS_AUDIO="$FIXTURE_AUDIO" \
    SBOBINO_ASR_SAMPLE=artemis \
    SBOBINO_ASR_COMPARE_MODE=transcribe \
    SBOBINO_REQUIRE_PARAKEET_GPU="$require_parakeet_gpu" \
    SBOBINO_COMPARE_SKIP_PARAKEET_CPU="$skip_parakeet_cpu" \
    SBOBINO_COMPARE_SKIP_WHISPER_GPU=1 \
    "$ROOT_DIR/scripts/compare_asr_engines_real.sh" >"$compare_log" 2>&1; then
    fail_validation "Parakeet transcription smoke failed. Last log lines: $(tail -120 "$compare_log")"
  fi
}

run_parakeet_live_smoke() {
  if [[ -z "${FIXTURE_AUDIO// }" ]]; then
    fail_validation "SBOBINO_VALIDATION_FIXTURE_AUDIO is required for the Parakeet live smoke."
  fi
  if [[ ! -f "$FIXTURE_AUDIO" ]]; then
    fail_validation "Parakeet live smoke audio fixture not found at '$FIXTURE_AUDIO'."
  fi

  local parakeet_lib="$DATA_DIR/lib/libparakeet.dylib"
  local parakeet_models_dir="$DATA_DIR/parakeet-models"
  local realtime_model=""
  local live_log="$TMP_DIR/parakeet-live-smoke.log"

  if [[ ! -f "$parakeet_lib" ]]; then
    fail_validation "Managed Parakeet live library was not installed at '$parakeet_lib'."
  fi
  for candidate in nemotron-3.5-asr-streaming-0.6b-q4_k.gguf; do
    if [[ -f "$parakeet_models_dir/$candidate" ]]; then
      realtime_model=$candidate
      break
    fi
  done
  if [[ -z "$realtime_model" ]]; then
    fail_validation "Managed Parakeet realtime EOU model was not installed in '$parakeet_models_dir'."
  fi

  if ! PATH="$DATA_DIR/bin:$PATH" \
    DYLD_LIBRARY_PATH="$DATA_DIR/lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" \
    DYLD_FALLBACK_LIBRARY_PATH="$DATA_DIR/lib${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}" \
    SBOBINO_PARAKEET_LIB="$parakeet_lib" \
    SBOBINO_PARAKEET_MODELS_DIR="$parakeet_models_dir" \
    SBOBINO_PARAKEET_REALTIME_MODEL="$realtime_model" \
    SBOBINO_PARAKEET_AUDIO="$FIXTURE_AUDIO" \
    cargo test --manifest-path "$ROOT_DIR/Cargo.toml" -p sbobino-desktop \
      parakeet_realtime_c_api_streams_real_wav \
      -- --ignored --nocapture >"$live_log" 2>&1; then
    fail_validation "Parakeet live C API smoke failed. Last log lines: $(tail -120 "$live_log")"
  fi
}

find_previous_stable_version() {
  python3 - <<'PY' "$REPO_SLUG" "$TAG"
import json
import sys
import urllib.request

repo_slug, current_tag = sys.argv[1:3]
url = f"https://api.github.com/repos/{repo_slug}/releases"
request = urllib.request.Request(url, headers={"User-Agent": "sbobino-machine-validation"})
with urllib.request.urlopen(request) as response:
    releases = json.load(response)

for release in releases:
    tag = str(release.get("tag_name", "")).strip()
    if not tag or tag == current_tag:
        continue
    if release.get("draft") or release.get("prerelease"):
        continue
    if tag.startswith("v"):
        print(tag[1:])
        raise SystemExit(0)

raise SystemExit("Could not determine the previous stable GitHub release.")
PY
}

validate_intel_primary() {
  if [[ "$(uname -m)" != "x86_64" ]]; then
    fail_validation "INTEL-PRIMARY validation must run on an x86_64 Mac."
  fi

  "$ROOT_DIR/scripts/distribution_readiness.sh" "$VERSION" "$REPO_SLUG"
  SCENARIO_RELEASE_METADATA_VALIDATION="passed"

  local dmg_path="$TMP_DIR/Sbobino_${VERSION}_x86_64.dmg"
  local mount_dir="$TMP_DIR/mount-intel"
  mkdir -p "$mount_dir"
  download_asset "$VERSION" "Sbobino_${VERSION}_x86_64.dmg" "$dmg_path"
  hdiutil attach "$dmg_path" -nobrowse -mountpoint "$mount_dir" -quiet

  if [[ ! -d "$mount_dir/Sbobino.app" ]]; then
    hdiutil detach "$mount_dir" -quiet || true
    fail_validation "Mounted DMG does not contain Sbobino.app."
  fi

  local plist_version
  plist_version=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$mount_dir/Sbobino.app/Contents/Info.plist")
  if [[ "$plist_version" != "$VERSION" ]]; then
    hdiutil detach "$mount_dir" -quiet || true
    fail_validation "Mounted app bundle version '$plist_version' does not match expected version '$VERSION'."
  fi

  local executable_name
  executable_name=$(/usr/libexec/PlistBuddy -c "Print :CFBundleExecutable" "$mount_dir/Sbobino.app/Contents/Info.plist")
  local app_executable="$mount_dir/Sbobino.app/Contents/MacOS/$executable_name"
  if [[ ! -x "$app_executable" ]]; then
    hdiutil detach "$mount_dir" -quiet || true
    fail_validation "Mounted app bundle is missing the Sbobino executable."
  fi
  if [[ " $(lipo -archs "$app_executable") " != *" x86_64 "* ]]; then
    hdiutil detach "$mount_dir" -quiet || true
    fail_validation "Mounted app bundle is not an x86_64-native executable."
  fi

  hdiutil detach "$mount_dir" -quiet || true

  SCENARIO_BOOTSTRAP_LAYER_VALIDATION="passed"
  clear_install_state
  install_app_from_dmg "$VERSION" x86_64
  seed_privacy_acceptance
  set_speaker_diarization_enabled 1
  launch_app
  wait_for_setup_report_success "$TIMEOUT_SECONDS"
  wait_for_runtime_health_ready "$TIMEOUT_SECONDS" 1
  SCENARIO_CLEAN_ROOM_INSTALL="passed"

  quit_app
  launch_app
  wait_for_runtime_health_ready 900 1
  SCENARIO_WARM_RESTART="passed"

  run_diarization_smoke
  SCENARIO_FUNCTIONAL_DIARIZATION_SMOKE="passed"
  SCENARIO_ARM64_BINARY_EXECUTION="not_applicable"
  record_success "passed" "Intel runner completed native clean-room setup, warm restart, and diarization validation."
}

validate_as_primary() {
  if [[ "$(uname -m)" != "arm64" ]]; then
    fail_validation "AS-PRIMARY validation must run on an Apple Silicon Mac."
  fi

  local previous_version
  previous_version=$(find_previous_stable_version)

  clear_install_state
  install_app_from_dmg "$previous_version"
  seed_privacy_acceptance
  set_speaker_diarization_enabled 1
  launch_app
  wait_for_setup_report_success "$TIMEOUT_SECONDS"
  wait_for_runtime_health_ready "$TIMEOUT_SECONDS" 1
  run_diarization_smoke
  quit_app

  install_app_from_dmg "$VERSION"
  seed_privacy_acceptance
  set_speaker_diarization_enabled 1
  launch_app
  wait_for_runtime_health_ready 900 1
  SCENARIO_UPDATE_PATH_VALIDATION="passed"

  quit_app
  launch_app
  wait_for_runtime_health_ready 180 1
  SCENARIO_WARM_RESTART="passed"

  run_diarization_smoke
  SCENARIO_FUNCTIONAL_DIARIZATION_SMOKE="passed"

  record_success "passed" "Primary Apple Silicon machine upgraded from v${previous_version} to the public candidate and preserved runtime + pyannote usability."
}

case "$MACHINE_CLASS" in
  AS-PRIMARY)
    validate_as_primary
    ;;
  INTEL-PRIMARY)
    validate_intel_primary
    ;;
  *)
    usage
    exit 1
    ;;
esac
