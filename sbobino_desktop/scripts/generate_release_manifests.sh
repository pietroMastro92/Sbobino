#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "Usage: $0 <version> <asset-dir>" >&2
  exit 1
fi

VERSION=$1
ASSET_DIR=$2

RUNTIME_ZIP="$ASSET_DIR/speech-runtime-macos-aarch64.zip"
RUNTIME_X86_64_ZIP="$ASSET_DIR/speech-runtime-macos-x86_64.zip"
PYANNOTE_RUNTIME_ZIP="$ASSET_DIR/pyannote-runtime-macos-aarch64.zip"
PYANNOTE_RUNTIME_X86_64_ZIP="$ASSET_DIR/pyannote-runtime-macos-x86_64.zip"
PYANNOTE_MODEL_ZIP="$ASSET_DIR/pyannote-model-community-1.zip"
RUNTIME_MANIFEST="$ASSET_DIR/runtime-manifest.json"
PYANNOTE_MANIFEST="$ASSET_DIR/pyannote-manifest.json"
SETUP_MANIFEST="$ASSET_DIR/setup-manifest.json"
PYANNOTE_COMPAT_LEVEL=${PYANNOTE_COMPAT_LEVEL:-1}

for path in \
  "$RUNTIME_ZIP" \
  "$RUNTIME_X86_64_ZIP" \
  "$PYANNOTE_RUNTIME_ZIP" \
  "$PYANNOTE_RUNTIME_X86_64_ZIP" \
  "$PYANNOTE_MODEL_ZIP"
do
  if [[ ! -f "$path" ]]; then
    echo "Missing required release asset: $path" >&2
    exit 1
  fi
done

mkdir -p "$ASSET_DIR"

sha256() {
  shasum -a 256 "$1" | awk '{print $1}'
}

file_size_bytes() {
  python3 - "$1" <<'PY'
import pathlib
import sys

print(pathlib.Path(sys.argv[1]).stat().st_size)
PY
}

zip_expanded_size_bytes() {
  python3 - "$1" <<'PY'
import pathlib
import sys
import zipfile

with zipfile.ZipFile(pathlib.Path(sys.argv[1])) as archive:
    print(sum(entry.file_size for entry in archive.infolist()))
PY
}

RUNTIME_SHA=$(sha256 "$RUNTIME_ZIP")
RUNTIME_X86_64_SHA=$(sha256 "$RUNTIME_X86_64_ZIP")
PYANNOTE_RUNTIME_SHA=$(sha256 "$PYANNOTE_RUNTIME_ZIP")
PYANNOTE_RUNTIME_X86_64_SHA=$(sha256 "$PYANNOTE_RUNTIME_X86_64_ZIP")
PYANNOTE_MODEL_SHA=$(sha256 "$PYANNOTE_MODEL_ZIP")
RUNTIME_SIZE=$(file_size_bytes "$RUNTIME_ZIP")
RUNTIME_EXPANDED_SIZE=$(zip_expanded_size_bytes "$RUNTIME_ZIP")
RUNTIME_X86_64_SIZE=$(file_size_bytes "$RUNTIME_X86_64_ZIP")
RUNTIME_X86_64_EXPANDED_SIZE=$(zip_expanded_size_bytes "$RUNTIME_X86_64_ZIP")
PYANNOTE_RUNTIME_SIZE=$(file_size_bytes "$PYANNOTE_RUNTIME_ZIP")
PYANNOTE_RUNTIME_EXPANDED_SIZE=$(zip_expanded_size_bytes "$PYANNOTE_RUNTIME_ZIP")
PYANNOTE_RUNTIME_X86_64_SIZE=$(file_size_bytes "$PYANNOTE_RUNTIME_X86_64_ZIP")
PYANNOTE_RUNTIME_X86_64_EXPANDED_SIZE=$(zip_expanded_size_bytes "$PYANNOTE_RUNTIME_X86_64_ZIP")
PYANNOTE_MODEL_SIZE=$(file_size_bytes "$PYANNOTE_MODEL_ZIP")
PYANNOTE_MODEL_EXPANDED_SIZE=$(zip_expanded_size_bytes "$PYANNOTE_MODEL_ZIP")

cat >"$RUNTIME_MANIFEST" <<JSON
{
  "app_version": "$VERSION",
  "assets": [
    {
      "kind": "speech_runtime_macos_aarch64",
      "name": "$(basename "$RUNTIME_ZIP")",
      "sha256": "$RUNTIME_SHA",
      "size_bytes": $RUNTIME_SIZE,
      "expanded_size_bytes": $RUNTIME_EXPANDED_SIZE
    },
    {
      "kind": "speech_runtime_macos_x86_64",
      "name": "$(basename "$RUNTIME_X86_64_ZIP")",
      "sha256": "$RUNTIME_X86_64_SHA",
      "size_bytes": $RUNTIME_X86_64_SIZE,
      "expanded_size_bytes": $RUNTIME_X86_64_EXPANDED_SIZE
    }
  ]
}
JSON

cat >"$PYANNOTE_MANIFEST" <<JSON
{
  "app_version": "$VERSION",
  "compat_level": $PYANNOTE_COMPAT_LEVEL,
  "assets": [
    {
      "kind": "pyannote_runtime_macos_aarch64",
      "name": "$(basename "$PYANNOTE_RUNTIME_ZIP")",
      "sha256": "$PYANNOTE_RUNTIME_SHA",
      "size_bytes": $PYANNOTE_RUNTIME_SIZE,
      "expanded_size_bytes": $PYANNOTE_RUNTIME_EXPANDED_SIZE
    },
    {
      "kind": "pyannote_runtime_macos_x86_64",
      "name": "$(basename "$PYANNOTE_RUNTIME_X86_64_ZIP")",
      "sha256": "$PYANNOTE_RUNTIME_X86_64_SHA",
      "size_bytes": $PYANNOTE_RUNTIME_X86_64_SIZE,
      "expanded_size_bytes": $PYANNOTE_RUNTIME_X86_64_EXPANDED_SIZE
    },
    {
      "kind": "pyannote_model",
      "name": "$(basename "$PYANNOTE_MODEL_ZIP")",
      "sha256": "$PYANNOTE_MODEL_SHA",
      "size_bytes": $PYANNOTE_MODEL_SIZE,
      "expanded_size_bytes": $PYANNOTE_MODEL_EXPANDED_SIZE
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
  "pyannote_compat_level": $PYANNOTE_COMPAT_LEVEL,
  "runtime_manifest": {
    "name": "$(basename "$RUNTIME_MANIFEST")",
    "sha256": "$RUNTIME_MANIFEST_SHA",
    "size_bytes": $(file_size_bytes "$RUNTIME_MANIFEST"),
    "expanded_size_bytes": $(file_size_bytes "$RUNTIME_MANIFEST")
  },
  "runtime_assets": {
    "aarch64-apple-darwin": {
      "name": "$(basename "$RUNTIME_ZIP")",
      "sha256": "$RUNTIME_SHA",
      "size_bytes": $RUNTIME_SIZE,
      "expanded_size_bytes": $RUNTIME_EXPANDED_SIZE
    },
    "x86_64-apple-darwin": {
      "name": "$(basename "$RUNTIME_X86_64_ZIP")",
      "sha256": "$RUNTIME_X86_64_SHA",
      "size_bytes": $RUNTIME_X86_64_SIZE,
      "expanded_size_bytes": $RUNTIME_X86_64_EXPANDED_SIZE
    }
  },
  "pyannote_manifest": {
    "name": "$(basename "$PYANNOTE_MANIFEST")",
    "sha256": "$PYANNOTE_MANIFEST_SHA",
    "size_bytes": $(file_size_bytes "$PYANNOTE_MANIFEST"),
    "expanded_size_bytes": $(file_size_bytes "$PYANNOTE_MANIFEST")
  },
  "pyannote_runtime_assets": {
    "aarch64-apple-darwin": {
      "name": "$(basename "$PYANNOTE_RUNTIME_ZIP")",
      "sha256": "$PYANNOTE_RUNTIME_SHA",
      "size_bytes": $PYANNOTE_RUNTIME_SIZE,
      "expanded_size_bytes": $PYANNOTE_RUNTIME_EXPANDED_SIZE
    },
    "x86_64-apple-darwin": {
      "name": "$(basename "$PYANNOTE_RUNTIME_X86_64_ZIP")",
      "sha256": "$PYANNOTE_RUNTIME_X86_64_SHA",
      "size_bytes": $PYANNOTE_RUNTIME_X86_64_SIZE,
      "expanded_size_bytes": $PYANNOTE_RUNTIME_X86_64_EXPANDED_SIZE
    }
  },
  "pyannote_model_asset": {
    "name": "$(basename "$PYANNOTE_MODEL_ZIP")",
    "sha256": "$PYANNOTE_MODEL_SHA",
    "size_bytes": $PYANNOTE_MODEL_SIZE,
    "expanded_size_bytes": $PYANNOTE_MODEL_EXPANDED_SIZE
  }
}
JSON

echo "Created release manifests in $ASSET_DIR"
