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
    "release-readiness-proof.json",
    "distribution-readiness-proof.json",
    "portability-smoke-report.json",
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
  --pattern "portability-smoke-report.json"

python3 - <<'PY' "$TMP_DIR" "$VERSION" "$TAG"
import json
import pathlib
import sys

report_dir = pathlib.Path(sys.argv[1])
version = sys.argv[2]
tag = sys.argv[3]

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

portability = load_json(
    report_dir / "portability-smoke-report.json",
    "portability-smoke-report.json",
)
if int(portability.get("schema_version", 0)) != 1:
    raise SystemExit(
        "Stable promotion blocked: portability-smoke-report.json has unsupported schema_version."
    )
if portability.get("version") != version:
    raise SystemExit("Stable promotion blocked: portability-smoke-report.json version mismatch.")
if portability.get("release_tag") != tag:
    raise SystemExit("Stable promotion blocked: portability-smoke-report.json release_tag mismatch.")
if str(portability.get("status", "")).strip().lower() != "passed":
    raise SystemExit("Stable promotion blocked: portability-smoke-report.json is not marked passed.")
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
