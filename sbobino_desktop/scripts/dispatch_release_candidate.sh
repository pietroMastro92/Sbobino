#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: dispatch_release_candidate.sh <tag-name> [repo-slug]

Dispatches the Release Candidate workflow. The workflow runs entirely on
GitHub-hosted runners (macos-14 + ubuntu-latest); no self-hosted Mac
infrastructure is required.
EOF
}

if [[ $# -lt 1 || $# -gt 2 ]]; then
  usage
  exit 1
fi

TAG_NAME=$1
REPO_SLUG=${2:-pietroMastro92/Sbobino}

if ! command -v gh >/dev/null 2>&1; then
  echo "Missing required command: gh" >&2
  exit 1
fi

gh workflow run "Release Candidate" \
  --repo "$REPO_SLUG" \
  -f "tag_name=$TAG_NAME"

cat <<EOF
Release candidate workflow dispatched successfully.
  repo: $REPO_SLUG
  tag:  $TAG_NAME

Next recommended command:
  gh run list --repo "$REPO_SLUG" --workflow "Release Candidate" --limit 5
EOF
