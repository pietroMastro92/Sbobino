#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: promote_candidate_release.sh <version> [repo-slug]

Promotes a previously validated GitHub prerelease candidate to stable and
removes older stable releases by default.
EOF
}

if [[ $# -lt 1 || $# -gt 2 ]]; then
  usage
  exit 1
fi

VERSION=$1
REPO_SLUG=${2:-pietroMastro92/Sbobino}
TAG="v$VERSION"

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

need_cmd gh
need_cmd python3

RELEASE_JSON=$(gh release view "$TAG" --repo "$REPO_SLUG" --json assets,isPrerelease,name,tagName,url)
if [[ -z "$RELEASE_JSON" ]]; then
  echo "Release $TAG was not found in $REPO_SLUG." >&2
  exit 1
fi

IS_PRERELEASE=$(python3 - <<'PY' "$RELEASE_JSON"
import json, sys
print("1" if json.loads(sys.argv[1]).get("isPrerelease") else "0")
PY
)

if [[ "$IS_PRERELEASE" != "1" ]]; then
  echo "Release $TAG is already stable. Only validated prereleases can be promoted." >&2
  exit 1
fi

python3 - <<'PY' "$RELEASE_JSON" "$VERSION"
import json
import sys

release = json.loads(sys.argv[1])
version = sys.argv[2]
expected_assets = {
    "AS-CLEAN-THIRD-MAC.validation-report.json",
    "AS-UPGRADE-MAC.validation-report.json",
}
present_assets = {
    asset.get("name", "").strip()
    for asset in release.get("assets", [])
    if isinstance(asset, dict)
}
missing = sorted(expected_assets - present_assets)
if missing:
    raise SystemExit(
        "Stable promotion blocked: missing validation report assets: "
        + ", ".join(missing)
    )
if release.get("tagName") != f"v{version}":
    raise SystemExit("Release tag does not match the requested version.")
PY

TMP_DIR=$(mktemp -d)
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

gh release download "$TAG" \
  --repo "$REPO_SLUG" \
  --dir "$TMP_DIR" \
  --pattern "AS-CLEAN-THIRD-MAC.validation-report.json" \
  --pattern "AS-UPGRADE-MAC.validation-report.json"

python3 - <<'PY' "$TMP_DIR" "$VERSION" "$TAG" "$RELEASE_JSON"
import json
import pathlib
import sys

report_dir = pathlib.Path(sys.argv[1])
version = sys.argv[2]
tag = sys.argv[3]
release = json.loads(sys.argv[4])
release_url = str(release.get("url", "")).strip()

expected_reports = {
    "AS-CLEAN-THIRD-MAC.validation-report.json": {
        "machine_class": "AS-CLEAN-THIRD-MAC",
        "required_scenarios": [
            "clean_room_install",
            "warm_restart",
            "functional_diarization_smoke",
        ],
    },
    "AS-UPGRADE-MAC.validation-report.json": {
        "machine_class": "AS-UPGRADE-MAC",
        "required_scenarios": [
            "update_path_validation",
        ],
    },
}

def require_non_empty(value: object, label: str, report_name: str) -> None:
    if not str(value or "").strip():
        raise SystemExit(f"Stable promotion blocked: {report_name} missing {label}.")

for report_name, expectation in expected_reports.items():
    report_path = report_dir / report_name
    if not report_path.is_file():
        raise SystemExit(f"Stable promotion blocked: could not download {report_name}.")
    report = json.loads(report_path.read_text(encoding="utf-8"))
    if int(report.get("schema_version", 0)) != 1:
        raise SystemExit(f"Stable promotion blocked: {report_name} has unsupported schema_version.")
    if report.get("version") != version:
        raise SystemExit(f"Stable promotion blocked: {report_name} version mismatch.")
    if report.get("release_tag") != tag:
        raise SystemExit(f"Stable promotion blocked: {report_name} release_tag mismatch.")
    if report.get("machine_class") != expectation["machine_class"]:
        raise SystemExit(f"Stable promotion blocked: {report_name} machine_class mismatch.")
    if str(report.get("status", "")).strip().lower() != "passed":
        raise SystemExit(f"Stable promotion blocked: {report_name} is not marked passed.")
    require_non_empty(report.get("tester"), "tester", report_name)
    require_non_empty(report.get("macos_version"), "macos_version", report_name)
    require_non_empty(report.get("tested_at_utc"), "tested_at_utc", report_name)
    require_non_empty(report.get("release_url"), "release_url", report_name)
    if str(report.get("release_url", "")).strip() != release_url:
        raise SystemExit(
            f"Stable promotion blocked: {report_name} release_url does not match the public release URL."
        )
    required_scenarios = report.get("required_scenarios")
    if required_scenarios != expectation["required_scenarios"]:
        raise SystemExit(
            f"Stable promotion blocked: {report_name} required_scenarios do not match the expected matrix."
        )
    scenario_results = report.get("scenario_results")
    if not isinstance(scenario_results, dict):
        raise SystemExit(f"Stable promotion blocked: {report_name} is missing scenario_results.")
    for scenario in expectation["required_scenarios"]:
        if str(scenario_results.get(scenario, "")).strip().lower() != "passed":
            raise SystemExit(
                f"Stable promotion blocked: {report_name} scenario {scenario} is not passed."
            )
PY

gh release edit "$TAG" --repo "$REPO_SLUG" --prerelease=false

RELEASE_LIST_JSON=$(gh release list --repo "$REPO_SLUG" --exclude-pre-releases --json tagName,isLatest)

OLDER_STABLE_TAGS=$(python3 - <<'PY' "$RELEASE_LIST_JSON" "$TAG"
import json, sys
releases = json.loads(sys.argv[1])
for release in releases:
    tag = release.get("tagName", "").strip()
    if tag and tag != sys.argv[2]:
        print(tag)
PY
)

if [[ -n "${OLDER_STABLE_TAGS// }" ]]; then
  while IFS= read -r stable_tag; do
    [[ -z "$stable_tag" ]] && continue
    gh release delete "$stable_tag" --repo "$REPO_SLUG" --yes --cleanup-tag
  done <<<"$OLDER_STABLE_TAGS"
fi

cat <<EOF
Candidate promoted to stable:
  repo: $REPO_SLUG
  tag:  $TAG

Older stable releases were removed to keep the latest validated version as the only stable public release.
EOF
