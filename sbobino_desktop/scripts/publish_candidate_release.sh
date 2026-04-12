#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: publish_candidate_release.sh <version> [repo-slug] [asset-dir] [--prerelease]

Creates a fresh GitHub release and uploads the full Sbobino asset set.
Use --prerelease only when you explicitly want a candidate release.
This command refuses to reuse an existing release for the same version.
EOF
}

if [[ $# -lt 1 || $# -gt 4 ]]; then
  usage
  exit 1
fi

VERSION=$1
REPO_SLUG=${2:-pietroMastro92/Sbobino}
ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
ASSET_DIR=${3:-"$ROOT_DIR/dist/local-release/v$VERSION"}
RELEASE_KIND=${4:-}
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

if [[ -n "$RELEASE_KIND" && "$RELEASE_KIND" != "--prerelease" ]]; then
  echo "Unsupported option: $RELEASE_KIND" >&2
  usage
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
  --title "$TAG" \
  --notes-file "$ASSET_DIR/release-notes.md"

if [[ "$RELEASE_KIND" == "--prerelease" ]]; then
  gh release edit "$TAG" --repo "$REPO_SLUG" --prerelease
fi

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
Release published successfully:
  repo: $REPO_SLUG
  tag:  $TAG

Next required steps:
  1. ./scripts/distribution_readiness.sh "$VERSION" "$REPO_SLUG"
  2. Validate the release on a second Apple Silicon Mac
  3. Use --prerelease only when you intentionally want a candidate first
EOF
