#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: retire_failed_candidate.sh <version> [repo-slug]

Deletes a failed GitHub prerelease candidate and its tag so the next patch
version can be cut cleanly.
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
need_cmd git

RELEASE_JSON=$(gh release view "$TAG" --repo "$REPO_SLUG" --json isPrerelease,tagName 2>/dev/null || true)
if [[ -z "$RELEASE_JSON" ]]; then
  echo "Release $TAG does not exist in $REPO_SLUG." >&2
  exit 1
fi

IS_PRERELEASE=$(python3 - <<'PY' "$RELEASE_JSON"
import json, sys
print("1" if json.loads(sys.argv[1]).get("isPrerelease") else "0")
PY
)

if [[ "$IS_PRERELEASE" != "1" ]]; then
  echo "Release $TAG is not a prerelease candidate. Stable releases must not be retired with this script." >&2
  exit 1
fi

gh release delete "$TAG" --repo "$REPO_SLUG" --yes --cleanup-tag

if git rev-parse "$TAG" >/dev/null 2>&1; then
  git tag -d "$TAG" >/dev/null
fi

cat <<EOF
Failed candidate retired:
  repo: $REPO_SLUG
  tag:  $TAG

Cut a new patch version before publishing another candidate.
EOF
