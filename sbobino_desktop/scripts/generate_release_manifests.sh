#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "Usage: $0 <version> <asset-dir>" >&2
  exit 1
fi

VERSION=$1
ASSET_DIR=$2

RUNTIME_ZIP="$ASSET_DIR/speech-runtime-macos-aarch64.zip"
PYANNOTE_RUNTIME_ZIP="$ASSET_DIR/pyannote-runtime-macos-aarch64.zip"
PYANNOTE_MODEL_ZIP="$ASSET_DIR/pyannote-model-community-1.zip"
RUNTIME_MANIFEST="$ASSET_DIR/runtime-manifest.json"
PYANNOTE_MANIFEST="$ASSET_DIR/pyannote-manifest.json"
SETUP_MANIFEST="$ASSET_DIR/setup-manifest.json"

for path in "$RUNTIME_ZIP" "$PYANNOTE_RUNTIME_ZIP" "$PYANNOTE_MODEL_ZIP"; do
  if [[ ! -f "$path" ]]; then
    echo "Missing required release asset: $path" >&2
    exit 1
  fi
done

mkdir -p "$ASSET_DIR"

sha256() {
  shasum -a 256 "$1" | awk '{print $1}'
}

RUNTIME_SHA=$(sha256 "$RUNTIME_ZIP")
PYANNOTE_RUNTIME_SHA=$(sha256 "$PYANNOTE_RUNTIME_ZIP")
PYANNOTE_MODEL_SHA=$(sha256 "$PYANNOTE_MODEL_ZIP")

cat >"$RUNTIME_MANIFEST" <<JSON
{
  "app_version": "$VERSION",
  "assets": [
    {
      "kind": "speech_runtime_macos_aarch64",
      "name": "$(basename "$RUNTIME_ZIP")",
      "sha256": "$RUNTIME_SHA"
    }
  ]
}
JSON

cat >"$PYANNOTE_MANIFEST" <<JSON
{
  "app_version": "$VERSION",
  "assets": [
    {
      "kind": "pyannote_runtime_macos_aarch64",
      "name": "$(basename "$PYANNOTE_RUNTIME_ZIP")",
      "sha256": "$PYANNOTE_RUNTIME_SHA"
    },
    {
      "kind": "pyannote_model",
      "name": "$(basename "$PYANNOTE_MODEL_ZIP")",
      "sha256": "$PYANNOTE_MODEL_SHA"
    }
  ]
}
JSON

RUNTIME_MANIFEST_SHA=$(sha256 "$RUNTIME_MANIFEST")
PYANNOTE_MANIFEST_SHA=$(sha256 "$PYANNOTE_MANIFEST")

cat >"$SETUP_MANIFEST" <<JSON
{
  "app_version": "$VERSION",
  "release_tag": "v$VERSION",
  "runtime_manifest": {
    "name": "$(basename "$RUNTIME_MANIFEST")",
    "sha256": "$RUNTIME_MANIFEST_SHA"
  },
  "runtime_asset": {
    "name": "$(basename "$RUNTIME_ZIP")",
    "sha256": "$RUNTIME_SHA"
  },
  "pyannote_manifest": {
    "name": "$(basename "$PYANNOTE_MANIFEST")",
    "sha256": "$PYANNOTE_MANIFEST_SHA"
  },
  "pyannote_runtime_asset": {
    "name": "$(basename "$PYANNOTE_RUNTIME_ZIP")",
    "sha256": "$PYANNOTE_RUNTIME_SHA"
  },
  "pyannote_model_asset": {
    "name": "$(basename "$PYANNOTE_MODEL_ZIP")",
    "sha256": "$PYANNOTE_MODEL_SHA"
  }
}
JSON

echo "Created release manifests in $ASSET_DIR"
