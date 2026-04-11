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

RELEASE_JSON=$(gh release view "$TAG" --repo "$REPO_SLUG" --json isPrerelease,name,tagName,url)
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

gh release edit "$TAG" --repo "$REPO_SLUG" --prerelease=false

OLDER_STABLE_TAGS=$(gh release list --repo "$REPO_SLUG" --exclude-pre-releases --json tagName,isLatest | python3 - <<'PY'
import json, sys
releases = json.load(sys.stdin)
for release in releases:
    tag = release.get("tagName", "").strip()
    if tag and tag != sys.argv[1]:
        print(tag)
PY
"$TAG")

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
