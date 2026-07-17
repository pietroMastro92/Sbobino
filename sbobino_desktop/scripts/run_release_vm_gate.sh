#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: run_release_vm_gate.sh <version> [repo-slug] [machine-class]

Runs a hosted release gate for a published prerelease.
Supported machine classes: AS-THIRD (default), INTEL-PRIMARY, WINDOWS-PRIMARY.

Set SBOBINO_RELEASE_RUN_ID to reuse an existing GitHub Actions run.
Set SBOBINO_RELEASE_VM_WORKFLOW_REF to dispatch the validation workflow from a
branch/ref other than the release tag.
EOF
}

if [[ $# -lt 1 || $# -gt 3 ]]; then
  usage
  exit 1
fi

VERSION=$1
REPO_SLUG=${2:-pietroMastro92/Sbobino}
MACHINE_CLASS=${3:-AS-THIRD}
TAG="v$VERSION"
WORKFLOW_FILE="release-vm-validation.yml"

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

need_cmd gh
need_cmd python3

case "$MACHINE_CLASS" in
  AS-THIRD|INTEL-PRIMARY|WINDOWS-PRIMARY)
    ;;
  *)
    echo "Unsupported machine class: $MACHINE_CLASS" >&2
    usage
    exit 1
    ;;
esac

RELEASE_JSON=$(gh release view "$TAG" --repo "$REPO_SLUG" --json assets,isPrerelease,tagName,url)
python3 - <<'PY' "$RELEASE_JSON" "$VERSION" "$MACHINE_CLASS"
import json
import sys

release = json.loads(sys.argv[1])
version = sys.argv[2]
machine_class = sys.argv[3]
if release.get("tagName") != f"v{version}":
    raise SystemExit("Release tag does not match requested version.")
if release.get("isPrerelease") is not True:
    raise SystemExit(f"{machine_class} VM gate must run against a GitHub prerelease.")

assets = {asset.get("name", "").strip() for asset in release.get("assets", [])}
required_by_class = {
    "AS-THIRD": {
        f"Sbobino_{version}_aarch64.dmg",
        "setup-manifest.json",
        "speech-runtime-macos-aarch64.zip",
    },
    "INTEL-PRIMARY": {
        f"Sbobino_{version}_x86_64.dmg",
        "setup-manifest.json",
        "speech-runtime-macos-x86_64.zip",
        "pyannote-runtime-macos-x86_64.zip",
    },
    "WINDOWS-PRIMARY": {
        f"Sbobino_{version}_windows_x86_64-setup.exe",
        "setup-manifest.json",
        "speech-runtime-windows-x86_64.zip",
        "pyannote-runtime-windows-x86_64.zip",
    },
}
required = required_by_class[machine_class]
missing = sorted(required - assets)
if missing:
    raise SystemExit(f"{machine_class} VM gate blocked: missing release assets: " + ", ".join(missing))
PY

RUN_ID=${SBOBINO_RELEASE_RUN_ID:-}
if [[ -z "$RUN_ID" ]]; then
  WORKFLOW_REF=${SBOBINO_RELEASE_VM_WORKFLOW_REF:-$TAG}
  gh workflow run "$WORKFLOW_FILE" \
    --repo "$REPO_SLUG" \
    --ref "$WORKFLOW_REF" \
    -f version="$VERSION" \
    -f machine_class="$MACHINE_CLASS"

  sleep 10
  RUNS_JSON=$(gh run list \
    --repo "$REPO_SLUG" \
    --workflow "$WORKFLOW_FILE" \
    --json databaseId,displayTitle,status,createdAt \
    --limit 20)
  RUN_ID=$(python3 - <<'PY' "$RUNS_JSON" "$MACHINE_CLASS" "$VERSION"
import json
import sys

runs = json.loads(sys.argv[1])
needle = f"{sys.argv[2]} v{sys.argv[3]}"
for run in runs:
    if needle in str(run.get("displayTitle", "")):
        print(run["databaseId"])
        raise SystemExit(0)
raise SystemExit(f"Could not resolve workflow run id for {needle}. Set SBOBINO_RELEASE_RUN_ID manually.")
PY
)
fi

echo "Watching $MACHINE_CLASS validation run: $RUN_ID"
gh run watch "$RUN_ID" --repo "$REPO_SLUG" --exit-status

TMP_DIR=$(mktemp -d)
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

gh release download "$TAG" \
  --repo "$REPO_SLUG" \
  --dir "$TMP_DIR" \
  --pattern "$MACHINE_CLASS.validation-report.json"

python3 - <<'PY' "$TMP_DIR/$MACHINE_CLASS.validation-report.json" "$VERSION" "$TAG" "$MACHINE_CLASS"
import json
import sys
from pathlib import Path

report_path = Path(sys.argv[1])
version = sys.argv[2]
tag = sys.argv[3]
machine_class = sys.argv[4]

if not report_path.is_file():
    raise SystemExit(f"Missing {machine_class}.validation-report.json on prerelease.")

report = json.loads(report_path.read_text(encoding="utf-8"))
if int(report.get("schema_version", 0)) != 1:
    raise SystemExit(f"{machine_class} validation report has unsupported schema_version.")
if report.get("machine_class") != machine_class:
    raise SystemExit(f"{machine_class} validation report machine_class mismatch.")
if report.get("version") != version or report.get("release_tag") != tag:
    raise SystemExit(f"{machine_class} validation report version/tag mismatch.")
if str(report.get("status", "")).strip().lower() != "passed":
    raise SystemExit(f"{machine_class} validation report is not passed.")

results = report.get("scenario_results") or {}
required_by_class = {
    "AS-THIRD": {
        "clean_room_install",
        "warm_restart",
        "functional_parakeet_smoke",
        "functional_diarization_smoke",
    },
    "INTEL-PRIMARY": {
        "release_metadata_validation",
        "bootstrap_layer_validation",
        "clean_room_install",
        "warm_restart",
        "functional_diarization_smoke",
    },
    "WINDOWS-PRIMARY": {
        "clean_room_install",
        "first_setup",
        "functional_transcription_smoke",
        "functional_diarization_smoke",
        "warm_restart",
        "no_visible_console_windows",
        "opaque_main_window",
    },
}
required = required_by_class[machine_class]
failed = sorted(name for name in required if results.get(name) != "passed")
if failed:
    raise SystemExit(f"{machine_class} validation report missing passed scenarios: " + ", ".join(failed))
PY

echo "$MACHINE_CLASS release gate passed for $TAG."
