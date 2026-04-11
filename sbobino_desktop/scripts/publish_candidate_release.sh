#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: publish_candidate_release.sh <version> [repo-slug] [asset-dir]

Creates a fresh GitHub prerelease candidate and uploads the full Sbobino asset set.
This command refuses to reuse an existing release for the same version.
EOF
}

if [[ $# -lt 1 || $# -gt 3 ]]; then
  usage
  exit 1
fi

VERSION=$1
REPO_SLUG=${2:-pietroMastro92/Sbobino}
ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
ASSET_DIR=${3:-"$ROOT_DIR/dist/local-release/v$VERSION"}
TAG="v$VERSION"

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

need_cmd gh
need_cmd git

if [[ ! -d "$ASSET_DIR" ]]; then
  echo "Candidate asset directory not found: $ASSET_DIR" >&2
  exit 1
fi

required_assets=(
  "Sbobino_${VERSION}_aarch64.dmg"
  "Sbobino.app.tar.gz"
  "Sbobino.app.tar.gz.sig"
  "latest.json"
  "setup-manifest.json"
  "runtime-manifest.json"
  "speech-runtime-macos-aarch64.zip"
  "pyannote-manifest.json"
  "pyannote-runtime-macos-aarch64.zip"
  "pyannote-model-community-1.zip"
  "release-notes.md"
)

for asset in "${required_assets[@]}"; do
  if [[ ! -f "$ASSET_DIR/$asset" ]]; then
    echo "Missing required candidate asset: $ASSET_DIR/$asset" >&2
    exit 1
  fi
done

if gh release view "$TAG" --repo "$REPO_SLUG" >/dev/null 2>&1; then
  echo "Release $TAG already exists in $REPO_SLUG. Candidate versions must be fresh patch releases." >&2
  exit 1
fi

if ! git rev-parse "$TAG" >/dev/null 2>&1; then
  echo "Local git tag $TAG does not exist. Create it before publishing the candidate." >&2
  exit 1
fi

gh release create "$TAG" \
  --repo "$REPO_SLUG" \
  --prerelease \
  --title "$TAG" \
  --notes-file "$ASSET_DIR/release-notes.md"

gh release upload "$TAG" \
  "$ASSET_DIR/Sbobino_${VERSION}_aarch64.dmg" \
  "$ASSET_DIR/Sbobino.app.tar.gz" \
  "$ASSET_DIR/Sbobino.app.tar.gz.sig" \
  "$ASSET_DIR/latest.json" \
  "$ASSET_DIR/setup-manifest.json" \
  "$ASSET_DIR/speech-runtime-macos-aarch64.zip" \
  "$ASSET_DIR/runtime-manifest.json" \
  "$ASSET_DIR/pyannote-runtime-macos-aarch64.zip" \
  "$ASSET_DIR/pyannote-model-community-1.zip" \
  "$ASSET_DIR/pyannote-manifest.json" \
  --repo "$REPO_SLUG"

cat <<EOF
Candidate prerelease published successfully:
  repo: $REPO_SLUG
  tag:  $TAG

Next required steps:
  1. ./scripts/distribution_readiness.sh "$VERSION" "$REPO_SLUG"
  2. Validate the prerelease on a second Apple Silicon Mac
  3. Promote it only if the clean-room validation passes
EOF
