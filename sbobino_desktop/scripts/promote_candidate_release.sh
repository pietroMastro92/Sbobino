#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: promote_candidate_release.sh <version> [repo-slug]

Promotes a previously validated GitHub prerelease candidate to stable and
removes older stable releases by default.

Environment:
  LOCAL_STABLE_RELEASES_TO_KEEP  Number of local dist/local-release versions to keep after
                                 a successful stable promotion. Default: 2.
EOF
}

if [[ $# -lt 1 || $# -gt 2 ]]; then
  usage
  exit 1
fi

VERSION=$1
REPO_SLUG=${2:-pietroMastro92/Sbobino}
TAG="v$VERSION"
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
LOCAL_RELEASE_ROOT=${LOCAL_RELEASE_ROOT:-"$SCRIPT_DIR/../dist/local-release"}
LOCAL_STABLE_RELEASES_TO_KEEP=${LOCAL_STABLE_RELEASES_TO_KEEP:-2}

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

need_cmd gh
need_cmd python3

if [[ ! "$LOCAL_STABLE_RELEASES_TO_KEEP" =~ ^[0-9]+$ ]] || [[ "$LOCAL_STABLE_RELEASES_TO_KEEP" -lt 1 ]]; then
  echo "LOCAL_STABLE_RELEASES_TO_KEEP must be a positive integer." >&2
  exit 1
fi

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
    "release-readiness-proof.json",
    "distribution-readiness-proof.json",
    "AS-PRIMARY.validation-report.json",
    "AS-THIRD.validation-report.json",
    "INTEL-PRIMARY.validation-report.json",
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
  --pattern "release-readiness-proof.json" \
  --pattern "distribution-readiness-proof.json" \
  --pattern "AS-PRIMARY.validation-report.json" \
  --pattern "AS-THIRD.validation-report.json" \
  --pattern "INTEL-PRIMARY.validation-report.json"

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
    "AS-PRIMARY.validation-report.json": {
        "machine_class": "AS-PRIMARY",
        "allowed_statuses": {"passed"},
        "runner_labels": {"self-hosted,macos,apple-silicon,as-primary"},
        "required_scenarios": [
            "native_upgrade_from_legacy_baseline",
            "warm_restart",
            "functional_diarization_smoke",
        ],
    },
    "AS-THIRD.validation-report.json": {
        "machine_class": "AS-THIRD",
        "allowed_statuses": {"passed"},
        "runner_labels": {
            "self-hosted,macos,apple-silicon,as-third",
            "github-hosted,macos-14",
            "github-hosted,macos-15",
        },
        "required_scenarios": [
            "clean_room_install",
            "warm_restart",
            "functional_diarization_smoke",
        ],
    },
    "INTEL-PRIMARY.validation-report.json": {
        "machine_class": "INTEL-PRIMARY",
        "allowed_statuses": {"passed", "soft_pass"},
        "runner_labels": {
            "self-hosted,macos,x64,intel-primary",
            "github-hosted,macos-13",
            "github-hosted,macos-15-intel",
        },
        "required_scenarios": [
            "release_metadata_validation",
            "bootstrap_layer_validation",
        ],
    },
}

def require_non_empty(value: object, label: str, report_name: str) -> None:
    if not str(value or "").strip():
        raise SystemExit(f"Stable promotion blocked: {report_name} missing {label}.")

def load_json(path: pathlib.Path, label: str) -> dict:
    if not path.is_file():
        raise SystemExit(f"Stable promotion blocked: could not download {label}.")
    return json.loads(path.read_text(encoding="utf-8"))

readiness = load_json(report_dir / "release-readiness-proof.json", "release-readiness-proof.json")
if readiness.get("version") != version:
    raise SystemExit("Stable promotion blocked: release-readiness-proof.json version mismatch.")
if str(readiness.get("status", "")).strip().lower() != "passed":
    raise SystemExit("Stable promotion blocked: release-readiness-proof.json is not marked passed.")
if str(readiness.get("gate", "")).strip() != "release_readiness.sh":
    raise SystemExit("Stable promotion blocked: release-readiness-proof.json gate mismatch.")

distribution = load_json(
    report_dir / "distribution-readiness-proof.json",
    "distribution-readiness-proof.json",
)
if int(distribution.get("schema_version", 0)) != 1:
    raise SystemExit(
        "Stable promotion blocked: distribution-readiness-proof.json has unsupported schema_version."
    )
if distribution.get("version") != version:
    raise SystemExit("Stable promotion blocked: distribution-readiness-proof.json version mismatch.")
if distribution.get("release_tag") != tag:
    raise SystemExit("Stable promotion blocked: distribution-readiness-proof.json release_tag mismatch.")
if str(distribution.get("status", "")).strip().lower() != "passed":
    raise SystemExit("Stable promotion blocked: distribution-readiness-proof.json is not marked passed.")
if str(distribution.get("gate", "")).strip() != "distribution_readiness.sh":
    raise SystemExit("Stable promotion blocked: distribution-readiness-proof.json gate mismatch.")

for report_name, expectation in expected_reports.items():
    report = load_json(report_dir / report_name, report_name)
    if int(report.get("schema_version", 0)) != 1:
        raise SystemExit(f"Stable promotion blocked: {report_name} has unsupported schema_version.")
    if report.get("version") != version:
        raise SystemExit(f"Stable promotion blocked: {report_name} version mismatch.")
    if report.get("release_tag") != tag:
        raise SystemExit(f"Stable promotion blocked: {report_name} release_tag mismatch.")
    if report.get("machine_class") != expectation["machine_class"]:
        raise SystemExit(f"Stable promotion blocked: {report_name} machine_class mismatch.")
    if str(report.get("status", "")).strip().lower() not in expectation["allowed_statuses"]:
        raise SystemExit(f"Stable promotion blocked: {report_name} is not in an allowed passed state.")
    require_non_empty(report.get("tester"), "tester", report_name)
    require_non_empty(report.get("os_name"), "os_name", report_name)
    require_non_empty(report.get("os_version"), "os_version", report_name)
    require_non_empty(report.get("tested_at_utc"), "tested_at_utc", report_name)
    require_non_empty(report.get("release_url"), "release_url", report_name)
    require_non_empty(report.get("commit_sha"), "commit_sha", report_name)
    if str(report.get("release_url", "")).strip() != release_url:
        raise SystemExit(
            f"Stable promotion blocked: {report_name} release_url does not match the public release URL."
        )
    if str(report.get("runner_label", "")).strip() not in expectation["runner_labels"]:
        raise SystemExit(f"Stable promotion blocked: {report_name} runner_label mismatch.")
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
    if report_name == "INTEL-PRIMARY.validation-report.json":
        arm64_execution = str(scenario_results.get("arm64_binary_execution", "")).strip().lower()
        if arm64_execution not in {"passed", "not_applicable"}:
            raise SystemExit(
                "Stable promotion blocked: INTEL-PRIMARY arm64_binary_execution must be passed or not_applicable."
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

LOCAL_RELEASE_DIRS_TO_DELETE=$(python3 - <<'PY' "$LOCAL_RELEASE_ROOT" "$TAG" "$LOCAL_STABLE_RELEASES_TO_KEEP"
import pathlib
import re
import sys

root = pathlib.Path(sys.argv[1])
current_tag = sys.argv[2]
keep_count = int(sys.argv[3])

if not root.is_dir():
    raise SystemExit(0)

version_pattern = re.compile(r"^v(\d+)\.(\d+)\.(\d+)$")

def version_key(path: pathlib.Path) -> tuple[int, int, int]:
    match = version_pattern.fullmatch(path.name)
    if not match:
        return (-1, -1, -1)
    return tuple(int(part) for part in match.groups())

release_dirs = [
    path
    for path in root.iterdir()
    if path.is_dir() and version_pattern.fullmatch(path.name)
]
release_dirs.sort(key=version_key, reverse=True)

keep_names = {path.name for path in release_dirs[:keep_count]}
keep_names.add(current_tag)

for path in release_dirs:
    if path.name not in keep_names:
        print(path)
PY
)

LOCAL_RELEASES_REMOVED=0
if [[ -n "${LOCAL_RELEASE_DIRS_TO_DELETE// }" ]]; then
  while IFS= read -r local_release_dir; do
    [[ -z "$local_release_dir" ]] && continue
    rm -rf "$local_release_dir"
    echo "Removed stale local release artifacts: $local_release_dir"
    LOCAL_RELEASES_REMOVED=$((LOCAL_RELEASES_REMOVED + 1))
  done <<<"$LOCAL_RELEASE_DIRS_TO_DELETE"
fi

cat <<EOF
Candidate promoted to stable:
  repo: $REPO_SLUG
  tag:  $TAG

Older stable releases were removed to keep the latest validated version as the only stable public release.
Local release retention:
  root:        $LOCAL_RELEASE_ROOT
  keep latest: $LOCAL_STABLE_RELEASES_TO_KEEP
  removed:     $LOCAL_RELEASES_REMOVED
EOF
