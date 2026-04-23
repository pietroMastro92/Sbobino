#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: run_release_machine_validation.sh <machine-class> <version> [repo-slug] [report-path]

Runs release validation on a self-hosted machine and writes a machine-readable
JSON report. Supported machine classes:
  - AS-PRIMARY
  - AS-THIRD
  - INTEL-PRIMARY

Environment variables:
  SBOBINO_VALIDATION_DATA_DIR        Override app data dir
  SBOBINO_VALIDATION_APP_PATH        Override installed app path
  SBOBINO_VALIDATION_FIXTURE_AUDIO   Audio fixture used for diarization smoke on Apple Silicon
  SBOBINO_VALIDATION_LEGACY_UPGRADE_BASELINE_VERSION
                                    Baseline app version used for native in-app
                                    update validation on AS-PRIMARY
  SBOBINO_VALIDATION_LEGACY_UPGRADE_BASELINE_DMG
                                    Optional local DMG path for the legacy
                                    baseline used by AS-PRIMARY
  SBOBINO_VALIDATION_NATIVE_UPDATE_TIMEOUT_SECONDS
                                    Timeout for the native in-app update leg on
                                    AS-PRIMARY (default: 1800)
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
FIXTURE_AUDIO=${SBOBINO_VALIDATION_FIXTURE_AUDIO:-}
LEGACY_UPGRADE_BASELINE_VERSION=${SBOBINO_VALIDATION_LEGACY_UPGRADE_BASELINE_VERSION:-0.1.16}
LEGACY_UPGRADE_BASELINE_REVISION=${SBOBINO_VALIDATION_LEGACY_UPGRADE_BASELINE_REVISION:-}
LEGACY_UPGRADE_BASELINE_DMG=${SBOBINO_VALIDATION_LEGACY_UPGRADE_BASELINE_DMG:-}
NATIVE_UPDATE_TIMEOUT_SECONDS=${SBOBINO_VALIDATION_NATIVE_UPDATE_TIMEOUT_SECONDS:-1800}
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
RUNNER_LABEL_OVERRIDE=${SBOBINO_VALIDATION_RUNNER_LABEL:-}
REPO_ROOT=$(cd "$ROOT_DIR/.." && pwd)
GITHUB_API_TOKEN=${GH_TOKEN:-${GITHUB_TOKEN:-}}

FINAL_STATUS="failed"
REPORT_NOTES=""

SCENARIO_NATIVE_UPGRADE_FROM_LEGACY_BASELINE="pending"
SCENARIO_CLEAN_ROOM_INSTALL="pending"
SCENARIO_WARM_RESTART="pending"
SCENARIO_FUNCTIONAL_DIARIZATION_SMOKE="pending"
SCENARIO_RELEASE_METADATA_VALIDATION="pending"
SCENARIO_BOOTSTRAP_LAYER_VALIDATION="pending"
SCENARIO_ARM64_BINARY_EXECUTION="pending"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

unexpected_validation_error() {
  local exit_code=$?
  trap - ERR
  if [[ "$FINAL_STATUS" == "failed" && -f "$REPORT_PATH" ]]; then
    exit "$exit_code"
  fi
  REPORT_NOTES="Unexpected validation error while running ${MACHINE_CLASS} release validation."
  FINAL_STATUS="failed"
  write_report || true
  exit "$exit_code"
}
trap unexpected_validation_error ERR

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

need_cmd cargo
need_cmd curl
need_cmd ditto
need_cmd hdiutil
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
    "$SCENARIO_NATIVE_UPGRADE_FROM_LEGACY_BASELINE" \
    "$SCENARIO_CLEAN_ROOM_INSTALL" \
    "$SCENARIO_WARM_RESTART" \
    "$SCENARIO_FUNCTIONAL_DIARIZATION_SMOKE" \
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
    native_upgrade_from_legacy_baseline,
    clean_room_install,
    warm_restart,
    functional_diarization_smoke,
    release_metadata_validation,
    bootstrap_layer_validation,
    arm64_binary_execution,
) = sys.argv[1:19]

definitions = {
    "AS-PRIMARY": {
        "required": [
            "native_upgrade_from_legacy_baseline",
            "warm_restart",
            "functional_diarization_smoke",
        ],
        "results": {
            "native_upgrade_from_legacy_baseline": native_upgrade_from_legacy_baseline,
            "warm_restart": warm_restart,
            "functional_diarization_smoke": functional_diarization_smoke,
        },
        "runner_label": "self-hosted,macos,apple-silicon,as-primary",
    },
    "AS-THIRD": {
        "required": [
            "clean_room_install",
            "warm_restart",
            "functional_diarization_smoke",
        ],
        "results": {
            "clean_room_install": clean_room_install,
            "warm_restart": warm_restart,
            "functional_diarization_smoke": functional_diarization_smoke,
        },
        "runner_label": "self-hosted,macos,apple-silicon,as-third",
    },
    "INTEL-PRIMARY": {
        "required": [
            "release_metadata_validation",
            "bootstrap_layer_validation",
        ],
        "results": {
            "release_metadata_validation": release_metadata_validation,
            "bootstrap_layer_validation": bootstrap_layer_validation,
            "arm64_binary_execution": arm64_binary_execution,
        },
        "runner_label": "self-hosted,macos,x64,intel-primary",
    },
}

definition = definitions[machine_class]
runner_label = os.environ.get("RUNNER_LABEL_OVERRIDE", "").strip() or definition["runner_label"]
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
  trap - ERR
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
  local bundle_executable=""
  if [[ -f "$APP_PATH/Contents/Info.plist" ]]; then
    bundle_executable=$(/usr/libexec/PlistBuddy -c "Print :CFBundleExecutable" "$APP_PATH/Contents/Info.plist" 2>/dev/null || true)
  fi
  if [[ -n "${bundle_executable// }" ]]; then
    pkill -f "$APP_PATH/Contents/MacOS/$bundle_executable" >/dev/null 2>&1 || true
  fi
  sleep 2
}

download_asset() {
  local version_arg=$1
  local asset_name=$2
  local destination=$3
  local url="https://github.com/$REPO_SLUG/releases/download/v${version_arg}/${asset_name}"
  local curl_args=()
  if [[ -n "${GITHUB_API_TOKEN// }" ]]; then
    curl_args+=(-H "Authorization: Bearer $GITHUB_API_TOKEN")
  fi
  curl \
    --fail \
    --location \
    --retry 3 \
    --retry-delay 2 \
    --silent \
    --show-error \
    "${curl_args[@]}" \
    --output "$destination" \
    "$url"
}

try_download_asset() {
  local version_arg=$1
  local asset_name=$2
  local destination=$3
  local url="https://github.com/$REPO_SLUG/releases/download/v${version_arg}/${asset_name}"
  local curl_args=()
  if [[ -n "${GITHUB_API_TOKEN// }" ]]; then
    curl_args+=(-H "Authorization: Bearer $GITHUB_API_TOKEN")
  fi
  curl \
    --fail \
    --location \
    --retry 1 \
    --retry-delay 1 \
    --silent \
    --show-error \
    "${curl_args[@]}" \
    --output "$destination" \
    "$url" >/dev/null 2>&1
}

resolve_legacy_upgrade_baseline_git_ref() {
  local baseline_tag="v${LEGACY_UPGRADE_BASELINE_VERSION}"
  if git -C "$REPO_ROOT" rev-parse "$baseline_tag" >/dev/null 2>&1; then
    echo "$baseline_tag"
    return 0
  fi

  local baseline_revision="${LEGACY_UPGRADE_BASELINE_REVISION:-}"
  if [[ -z "${baseline_revision// }" && "$LEGACY_UPGRADE_BASELINE_VERSION" == "0.1.16" ]]; then
    baseline_revision="c8a0992576055ec8c99fce8cfb383845c60be0da"
  fi

  if [[ -z "${baseline_revision// }" ]]; then
    return 1
  fi

  git -C "$REPO_ROOT" fetch --no-tags origin "$baseline_revision" >/dev/null 2>&1 || true
  if git -C "$REPO_ROOT" rev-parse "$baseline_revision" >/dev/null 2>&1; then
    echo "$baseline_revision"
    return 0
  fi

  return 1
}

read_bundle_executable() {
  if [[ ! -f "$APP_PATH/Contents/Info.plist" ]]; then
    return 1
  fi
  /usr/libexec/PlistBuddy -c "Print :CFBundleExecutable" "$APP_PATH/Contents/Info.plist" 2>/dev/null
}

read_installed_app_version() {
  if [[ ! -f "$APP_PATH/Contents/Info.plist" ]]; then
    return 1
  fi
  /usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$APP_PATH/Contents/Info.plist" 2>/dev/null
}

capture_app_pid() {
  local bundle_executable
  bundle_executable=$(read_bundle_executable 2>/dev/null || true)
  if [[ -z "${bundle_executable// }" ]]; then
    return 0
  fi
  pgrep -f "$APP_PATH/Contents/MacOS/$bundle_executable" | head -n 1 || true
}

extract_archive_single_root() {
  local archive_path=$1
  local destination=$2
  local stage_dir
  stage_dir=$(mktemp -d "$TMP_DIR/extract.XXXXXX")

  if ! /usr/bin/ditto -x -k "$archive_path" "$stage_dir"; then
    rm -rf "$stage_dir"
    fail_validation "Failed to extract archive '$archive_path'."
  fi

  local extracted_root
  extracted_root=$(python3 - <<'PY' "$stage_dir"
from pathlib import Path
import sys

stage_dir = Path(sys.argv[1])
roots = [path for path in stage_dir.iterdir() if path.is_dir() and path.name != "__MACOSX"]
if len(roots) != 1:
    raise SystemExit(1)
print(roots[0])
PY
)
  if [[ -z "${extracted_root// }" ]]; then
    rm -rf "$stage_dir"
    fail_validation "Archive '$archive_path' did not expand to a single root directory."
  fi

  rm -rf "$destination"
  mkdir -p "$(dirname "$destination")"
  mv "$extracted_root" "$destination"
  rm -rf "$stage_dir"
}

install_release_pyannote_baseline() {
  local asset_version=$1
  local manifest_version=$2
  local runtime_root="$DATA_DIR/runtime/pyannote"
  local manifest_download="$TMP_DIR/pyannote-manifest-${asset_version}.json"
  local runtime_download="$TMP_DIR/pyannote-runtime-${asset_version}.zip"
  local model_download="$TMP_DIR/pyannote-model-${asset_version}.zip"

  download_asset "$asset_version" "pyannote-manifest.json" "$manifest_download"
  download_asset "$asset_version" "pyannote-runtime-macos-aarch64.zip" "$runtime_download"
  download_asset "$asset_version" "pyannote-model-community-1.zip" "$model_download"

  rm -rf "$runtime_root"
  mkdir -p "$runtime_root"
  extract_archive_single_root "$runtime_download" "$runtime_root/python"
  extract_archive_single_root "$model_download" "$runtime_root/model"

  python3 - <<'PY' "$manifest_download" "$runtime_root/manifest.json" "$runtime_root/status.json" "$manifest_version"
import json
import sys
from datetime import datetime, timezone
from pathlib import Path

release_manifest_path = Path(sys.argv[1])
managed_manifest_path = Path(sys.argv[2])
status_path = Path(sys.argv[3])
manifest_version = str(sys.argv[4]).strip()

release_manifest = json.loads(release_manifest_path.read_text(encoding="utf-8"))
assets = {asset.get("kind"): asset for asset in release_manifest.get("assets", [])}
runtime_asset = assets.get("pyannote_runtime_macos_aarch64")
model_asset = assets.get("pyannote_model")
if not isinstance(runtime_asset, dict) or not isinstance(model_asset, dict):
    raise SystemExit("Pyannote release manifest is missing required assets.")

installed_at = datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
managed_manifest = {
    "source": "release_asset",
    "app_version": manifest_version,
    "compat_level": int(release_manifest.get("compat_level", 1) or 1),
    "runtime_asset": str(runtime_asset.get("name", "")).strip(),
    "runtime_sha256": str(runtime_asset.get("sha256", "")).strip(),
    "model_asset": str(model_asset.get("name", "")).strip(),
    "model_sha256": str(model_asset.get("sha256", "")).strip(),
    "runtime_arch": "aarch64-apple-darwin",
    "installed_at": installed_at,
}
managed_manifest_path.write_text(json.dumps(managed_manifest, indent=2) + "\n", encoding="utf-8")

status_payload = {
    "reason_code": "ok",
    "message": "Pyannote diarization runtime is ready.",
    "updated_at": installed_at,
}
status_path.write_text(json.dumps(status_payload, indent=2) + "\n", encoding="utf-8")
PY
}

capture_runtime_health_reason_code() {
  local snapshot_path=$1
  python3 - <<'PY' "$snapshot_path"
import json
import sys

snapshot = json.load(open(sys.argv[1], encoding="utf-8"))
print(str(snapshot.get("health", {}).get("pyannote", {}).get("reason_code", "")).strip())
PY
}

capture_runtime_health_message() {
  local snapshot_path=$1
  python3 - <<'PY' "$snapshot_path"
import json
import sys

snapshot = json.load(open(sys.argv[1], encoding="utf-8"))
print(str(snapshot.get("health", {}).get("pyannote", {}).get("message", "")).strip())
PY
}

capture_runtime_health_ready_flag() {
  local snapshot_path=$1
  python3 - <<'PY' "$snapshot_path"
import json
import sys

snapshot = json.load(open(sys.argv[1], encoding="utf-8"))
print("1" if snapshot.get("health", {}).get("pyannote", {}).get("ready") else "0")
PY
}

install_app_from_dmg() {
  local version_arg=$1
  local dmg_path="$TMP_DIR/Sbobino_${version_arg}_aarch64.dmg"
  download_asset "$version_arg" "Sbobino_${version_arg}_aarch64.dmg" "$dmg_path"
  install_app_from_dmg_path "$dmg_path"
}

install_app_from_dmg_path() {
  local dmg_path=$1
  local mount_dir="$TMP_DIR/mount-$(basename "$dmg_path" .dmg)"
  mkdir -p "$mount_dir"
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

prepare_legacy_upgrade_baseline_dmg() {
  if [[ -n "${LEGACY_UPGRADE_BASELINE_DMG// }" ]]; then
    if [[ ! -f "$LEGACY_UPGRADE_BASELINE_DMG" ]]; then
      fail_validation "Configured legacy baseline DMG was not found at '$LEGACY_UPGRADE_BASELINE_DMG'."
    fi
    echo "$LEGACY_UPGRADE_BASELINE_DMG"
    return 0
  fi

  local downloaded_dmg="$TMP_DIR/Sbobino_${LEGACY_UPGRADE_BASELINE_VERSION}_legacy-baseline.dmg"
  if try_download_asset \
    "$LEGACY_UPGRADE_BASELINE_VERSION" \
    "Sbobino_${LEGACY_UPGRADE_BASELINE_VERSION}_aarch64.dmg" \
    "$downloaded_dmg"; then
    echo "$downloaded_dmg"
    return 0
  fi

  local baseline_ref
  local worktree_dir="$TMP_DIR/legacy-upgrade-baseline"
  local release_dir="$worktree_dir/sbobino_desktop"
  local candidate_feed_url="https://github.com/$REPO_SLUG/releases/download/$TAG/latest.json"
  local baseline_app_dir="$release_dir/target/aarch64-apple-darwin/release/bundle/macos"
  local baseline_app_path="$baseline_app_dir/Sbobino.app"
  local baseline_dmg_dir="$release_dir/target/aarch64-apple-darwin/release/bundle/dmg"
  local baseline_dmg_path="$baseline_dmg_dir/Sbobino_${LEGACY_UPGRADE_BASELINE_VERSION}_aarch64.dmg"
  local staging_dir

  if ! baseline_ref=$(resolve_legacy_upgrade_baseline_git_ref); then
    fail_validation "Legacy baseline v${LEGACY_UPGRADE_BASELINE_VERSION} is unavailable: no public DMG, no local tag, and no fallback git revision were found."
  fi

  git -C "$REPO_ROOT" worktree add --detach "$worktree_dir" "$baseline_ref" >/dev/null
  python3 - <<'PY' "$release_dir/apps/desktop/src-tauri/tauri.conf.json" "$candidate_feed_url"
import json
import sys
from pathlib import Path

config_path = Path(sys.argv[1])
candidate_feed_url = sys.argv[2]
payload = json.loads(config_path.read_text(encoding="utf-8"))
payload.setdefault("plugins", {}).setdefault("updater", {})["endpoints"] = [candidate_feed_url]
config_path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
PY

  if ! (
    cd "$release_dir"
    pushd apps/desktop >/dev/null
    npm ci
    SBOBINO_RELEASE_PROFILE=public npm run tauri:build -- --target aarch64-apple-darwin --bundles app
    popd >/dev/null

    if [[ ! -d "$baseline_app_path" ]]; then
      echo "Expected baseline app at '$baseline_app_path', but it was not created." >&2
      exit 1
    fi

    codesign --force --deep --sign - "$baseline_app_path"

    staging_dir=$(mktemp -d "$TMP_DIR/legacy-dmg-stage.XXXXXX")
    cp -R "$baseline_app_path" "$staging_dir/"
    ln -s /Applications "$staging_dir/Applications"
    mkdir -p "$baseline_dmg_dir"
    rm -f "$baseline_dmg_path"
    hdiutil create \
      -volname "Sbobino" \
      -srcfolder "$staging_dir" \
      -ov \
      -format UDZO \
      "$baseline_dmg_path" >/dev/null
    rm -rf "$staging_dir"
  ); then
    git -C "$REPO_ROOT" worktree remove --force "$worktree_dir" >/dev/null 2>&1 || true
    fail_validation "Failed to build the validation baseline app from git ref '$baseline_ref'."
  fi

  if [[ ! -f "$baseline_dmg_path" ]]; then
    git -C "$REPO_ROOT" worktree remove --force "$worktree_dir" >/dev/null 2>&1 || true
    fail_validation "Legacy baseline build did not produce '$baseline_dmg_path'."
  fi

  cp "$baseline_dmg_path" "$downloaded_dmg"
  git -C "$REPO_ROOT" worktree remove --force "$worktree_dir" >/dev/null 2>&1 || true
  echo "$downloaded_dmg"
}

trigger_update_menu_refresh() {
  osascript <<'APPLESCRIPT' >/dev/null 2>&1
tell application "Sbobino" to activate
tell application "System Events"
  tell process "Sbobino"
    try
      click menu item "Verifica disponibilita aggiornamenti..." of menu 1 of menu bar item 1 of menu bar 1
      return
    end try
  end tell
end tell
APPLESCRIPT
}

click_native_update_button() {
  local result
  result=$(osascript <<'APPLESCRIPT'
set buttonNames to {"Download & Install", "Scarica e installa", "Descargar e instalar", "Herunterladen und installieren"}
tell application "Sbobino" to activate
tell application "System Events"
  tell process "Sbobino"
    if not (exists window 1) then
      return "window-missing"
    end if
    repeat with wantedName in buttonNames
      try
        click first button of (entire contents of window 1) whose name is wantedName
        return "clicked"
      end try
    end repeat
  end tell
end tell
return "not-found"
APPLESCRIPT
)
  [[ "$result" == "clicked" ]]
}

read_update_banner_hint() {
  osascript <<'APPLESCRIPT' 2>/dev/null || true
set interestingSnippets to {"update", "install", "aggiorn", "scarica", "download", "actualizacion", "aktual"}
tell application "System Events"
  tell process "Sbobino"
    if not (exists window 1) then
      return ""
    end if
    set textItems to {}
    repeat with uiItem in (entire contents of window 1)
      try
        set candidateText to value of uiItem as text
        set end of textItems to candidateText
      end try
      try
        set candidateTitle to name of uiItem as text
        set end of textItems to candidateTitle
      end try
    end repeat
  end tell
end tell
repeat with candidateText in textItems
  repeat with snippet in interestingSnippets
    if (candidateText as text) contains (snippet as text) then
      return candidateText as text
    end if
  end repeat
end repeat
return ""
APPLESCRIPT
}

trigger_native_update_and_wait() {
  local target_version=$1
  local started_at
  local click_deadline
  local old_pid
  local observed_click=0
  local observed_version_switch=0
  local observed_relaunch=0
  local old_version=""

  old_pid=$(capture_app_pid)
  old_version=$(read_installed_app_version 2>/dev/null || true)
  started_at=$(date +%s)
  click_deadline=$((started_at + 180))

  while true; do
    local now
    now=$(date +%s)
    if (( observed_click == 0 )); then
      if click_native_update_button; then
        observed_click=1
      elif (( now > click_deadline )); then
        trigger_update_menu_refresh || true
        click_deadline=$((now + 60))
      fi
    fi

    local current_version=""
    local current_pid=""
    current_version=$(read_installed_app_version 2>/dev/null || true)
    current_pid=$(capture_app_pid)

    if [[ "$current_version" == "$target_version" ]]; then
      observed_version_switch=1
    fi
    if (( observed_version_switch == 1 )) && [[ -n "${current_pid// }" ]]; then
      if [[ -z "${old_pid// }" || "$current_pid" != "$old_pid" ]]; then
        observed_relaunch=1
      fi
    fi
    if (( observed_click == 1 && observed_version_switch == 1 && observed_relaunch == 1 )); then
      return 0
    fi

    if (( now - started_at > NATIVE_UPDATE_TIMEOUT_SECONDS )); then
      local banner_hint=""
      banner_hint=$(read_update_banner_hint)
      fail_validation "Timed out waiting for native in-app update from v${old_version:-unknown} to v${target_version}. clicked=${observed_click} version_switched=${observed_version_switch} relaunch=${observed_relaunch} current_bundle_version=${current_version:-missing} banner_hint=${banner_hint:-none}"
    fi
    sleep 5
  done
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

ensure_fixture_audio() {
  if [[ -n "${FIXTURE_AUDIO// }" && -f "$FIXTURE_AUDIO" ]]; then
    return 0
  fi

  if ! command -v say >/dev/null 2>&1 || ! command -v afconvert >/dev/null 2>&1; then
    fail_validation "SBOBINO_VALIDATION_FIXTURE_AUDIO is not set and this runner cannot synthesize the diarization fixture."
  fi

  local part1_aiff="$TMP_DIR/fixture-speaker-1.aiff"
  local part2_aiff="$TMP_DIR/fixture-speaker-2.aiff"
  local part1_wav="$TMP_DIR/fixture-speaker-1.wav"
  local part2_wav="$TMP_DIR/fixture-speaker-2.wav"
  local output_wav="$TMP_DIR/fixture-two-speakers.wav"

  say -v Samantha -o "$part1_aiff" "Hello, this is the first speaker. We are validating pyannote on a clean macOS machine."
  say -v Daniel -o "$part2_aiff" "Hi, this is the second speaker. The release should keep diarization working without manual repair."
  afconvert -f WAVE -d LEI16@16000 -c 1 "$part1_aiff" "$part1_wav"
  afconvert -f WAVE -d LEI16@16000 -c 1 "$part2_aiff" "$part2_wav"

  if ! python3 - <<'PY' "$part1_wav" "$part2_wav" "$output_wav"
import sys
import wave

first, second, output = sys.argv[1:4]

with wave.open(first, "rb") as source_a, wave.open(second, "rb") as source_b:
    format_a = (
        source_a.getnchannels(),
        source_a.getsampwidth(),
        source_a.getframerate(),
        source_a.getcomptype(),
        source_a.getcompname(),
    )
    format_b = (
        source_b.getnchannels(),
        source_b.getsampwidth(),
        source_b.getframerate(),
        source_b.getcomptype(),
        source_b.getcompname(),
    )
    if format_a != format_b:
        raise SystemExit(
            "Generated fixture clips do not share the same WAV format: "
            f"{format_a!r} != {format_b!r}"
        )

    frames = source_a.readframes(source_a.getnframes()) + source_b.readframes(source_b.getnframes())

with wave.open(output, "wb") as destination:
    destination.setnchannels(format_a[0])
    destination.setsampwidth(format_a[1])
    destination.setframerate(format_a[2])
    destination.setcomptype(format_a[3], format_a[4])
    destination.writeframes(frames)
PY
  then
    fail_validation "Failed to build a normalized two-speaker WAV fixture for pyannote validation."
  fi

  FIXTURE_AUDIO="$output_wav"
}

run_diarization_smoke() {
  ensure_fixture_audio
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
if len(labels) < 2:
    raise SystemExit(1)
PY
  then
    fail_validation "Pyannote smoke test did not produce at least two speaker labels."
  fi
}

find_previous_stable_version() {
  python3 - <<'PY' "$REPO_SLUG" "$TAG" "${GITHUB_API_TOKEN:-}"
import json
import sys
import urllib.request

repo_slug, current_tag, api_token = sys.argv[1:4]
url = f"https://api.github.com/repos/{repo_slug}/releases"
headers = {"User-Agent": "sbobino-machine-validation"}
if api_token.strip():
    headers["Authorization"] = f"Bearer {api_token.strip()}"
request = urllib.request.Request(url, headers=headers)
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

  if ! "$ROOT_DIR/scripts/distribution_readiness.sh" "$VERSION" "$REPO_SLUG"; then
    fail_validation "Intel runner failed distribution readiness for v${VERSION}."
  fi
  SCENARIO_RELEASE_METADATA_VALIDATION="passed"

  local dmg_path="$TMP_DIR/Sbobino_${VERSION}_aarch64.dmg"
  local mount_dir="$TMP_DIR/mount-intel"
  mkdir -p "$mount_dir"
  download_asset "$VERSION" "Sbobino_${VERSION}_aarch64.dmg" "$dmg_path"
  hdiutil attach "$dmg_path" -nobrowse -mountpoint "$mount_dir" -quiet

  if [[ ! -d "$mount_dir/Sbobino.app" ]]; then
    hdiutil detach "$mount_dir" -quiet || true
    fail_validation "Mounted DMG does not contain Sbobino.app."
  fi

  local plist_version
  local bundle_executable
  plist_version=$(/usr/libexec/PlistBuddy -c "Print :CFBundleShortVersionString" "$mount_dir/Sbobino.app/Contents/Info.plist")
  if [[ "$plist_version" != "$VERSION" ]]; then
    hdiutil detach "$mount_dir" -quiet || true
    fail_validation "Mounted app bundle version '$plist_version' does not match expected version '$VERSION'."
  fi

  bundle_executable=$(/usr/libexec/PlistBuddy -c "Print :CFBundleExecutable" "$mount_dir/Sbobino.app/Contents/Info.plist")
  if [[ -z "${bundle_executable// }" ]]; then
    hdiutil detach "$mount_dir" -quiet || true
    fail_validation "Mounted app bundle does not declare CFBundleExecutable."
  fi

  if [[ ! -x "$mount_dir/Sbobino.app/Contents/MacOS/$bundle_executable" ]]; then
    hdiutil detach "$mount_dir" -quiet || true
    fail_validation "Mounted app bundle is missing the '$bundle_executable' executable."
  fi

  hdiutil detach "$mount_dir" -quiet || true

  SCENARIO_BOOTSTRAP_LAYER_VALIDATION="passed"
  SCENARIO_ARM64_BINARY_EXECUTION="not_applicable"
  record_success "soft_pass" "Intel runner validated release metadata and bootstrap artifacts for the arm64 candidate."
}

validate_as_third() {
  if [[ "$(uname -m)" != "arm64" ]]; then
    fail_validation "AS-THIRD validation must run on an Apple Silicon Mac."
  fi

  clear_install_state
  install_app_from_dmg "$VERSION"
  seed_privacy_acceptance
  launch_app
  wait_for_setup_report_success "$TIMEOUT_SECONDS"
  wait_for_runtime_health_ready "$TIMEOUT_SECONDS" 0
  SCENARIO_CLEAN_ROOM_INSTALL="passed"

  quit_app
  set_speaker_diarization_enabled 1
  launch_app
  wait_for_runtime_health_ready 900 1
  SCENARIO_WARM_RESTART="passed"

  run_diarization_smoke
  SCENARIO_FUNCTIONAL_DIARIZATION_SMOKE="passed"

  record_success "passed" "Third Apple Silicon machine completed clean-room install, enabled diarization without blocking the app, and passed the first pyannote smoke after background setup."
}

validate_as_primary() {
  if [[ "$(uname -m)" != "arm64" ]]; then
    fail_validation "AS-PRIMARY validation must run on an Apple Silicon Mac."
  fi

  local baseline_dmg
  baseline_dmg=$(prepare_legacy_upgrade_baseline_dmg)

  clear_install_state
  install_app_from_dmg_path "$baseline_dmg"
  seed_privacy_acceptance
  set_speaker_diarization_enabled 1
  install_release_pyannote_baseline "$VERSION" "$LEGACY_UPGRADE_BASELINE_VERSION"
  launch_app
  wait_for_setup_report_success "$TIMEOUT_SECONDS"
  local baseline_snapshot="$TMP_DIR/as-primary-baseline-health.json"
  capture_runtime_health "$baseline_snapshot"
  if [[ "$(capture_runtime_health_ready_flag "$baseline_snapshot")" != "1" ]]; then
    local baseline_reason
    baseline_reason=$(capture_runtime_health_reason_code "$baseline_snapshot")
    if [[ "$baseline_reason" == "pyannote_repair_required" || "$baseline_reason" == "pyannote_runtime_missing" || "$baseline_reason" == "pyannote_model_missing" || "$baseline_reason" == "pyannote_checksum_invalid" || "$baseline_reason" == "pyannote_version_mismatch" ]]; then
      quit_app
      rm -f "$SETUP_REPORT_PATH"
      install_release_pyannote_baseline "$VERSION" "$LEGACY_UPGRADE_BASELINE_VERSION"
      local repaired_baseline_snapshot="$TMP_DIR/as-primary-repaired-baseline-health.json"
      capture_runtime_health "$repaired_baseline_snapshot"
      if [[ "$(capture_runtime_health_ready_flag "$repaired_baseline_snapshot")" != "1" ]]; then
        fail_validation "Primary Apple Silicon fallback baseline remained pyannote-unready before upgrade: $(capture_runtime_health_message "$repaired_baseline_snapshot")"
      fi
    else
      fail_validation "Primary Apple Silicon baseline on v${LEGACY_UPGRADE_BASELINE_VERSION} is not pyannote-ready before upgrade: $(capture_runtime_health_message "$baseline_snapshot")"
    fi
  else
    wait_for_runtime_health_ready "$TIMEOUT_SECONDS" 1
  fi
  run_diarization_smoke

  trigger_native_update_and_wait "$VERSION"
  wait_for_runtime_health_ready 900 1
  SCENARIO_NATIVE_UPGRADE_FROM_LEGACY_BASELINE="passed"

  quit_app
  launch_app
  wait_for_runtime_health_ready 180 1
  SCENARIO_WARM_RESTART="passed"

  run_diarization_smoke
  SCENARIO_FUNCTIONAL_DIARIZATION_SMOKE="passed"

  record_success "passed" "Primary Apple Silicon machine completed a native in-app upgrade from v${LEGACY_UPGRADE_BASELINE_VERSION} to the public candidate and preserved runtime + pyannote usability."
}

case "$MACHINE_CLASS" in
  AS-PRIMARY)
    validate_as_primary
    ;;
  AS-THIRD)
    validate_as_third
    ;;
  INTEL-PRIMARY)
    validate_intel_primary
    ;;
  *)
    usage
    exit 1
    ;;
esac
