#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 5 ]]; then
  echo "Usage: $0 <version> <runtime_aarch64_dir> <runtime_x86_64_dir> <model_dir> <output_dir>" >&2
  exit 1
fi

VERSION=$1
RUNTIME_AARCH64_DIR=$2
RUNTIME_X86_64_DIR=$3
MODEL_DIR=$4
OUTPUT_DIR=$5

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
mkdir -p "$OUTPUT_DIR"

RUNTIME_AARCH64_ZIP="$OUTPUT_DIR/pyannote-runtime-macos-aarch64.zip"
RUNTIME_X86_64_ZIP="$OUTPUT_DIR/pyannote-runtime-macos-x86_64.zip"
MODEL_ZIP="$OUTPUT_DIR/pyannote-model-community-1.zip"
MANIFEST_JSON="$OUTPUT_DIR/pyannote-manifest.json"

"$SCRIPT_DIR/package_pyannote_asset.sh" "$RUNTIME_AARCH64_DIR" "python" "$RUNTIME_AARCH64_ZIP"
"$SCRIPT_DIR/package_pyannote_asset.sh" "$RUNTIME_X86_64_DIR" "python" "$RUNTIME_X86_64_ZIP"
"$SCRIPT_DIR/package_pyannote_asset.sh" "$MODEL_DIR" "model" "$MODEL_ZIP"
"$SCRIPT_DIR/generate_pyannote_manifest.sh" \
  "$VERSION" \
  "$RUNTIME_AARCH64_ZIP" \
  "$RUNTIME_X86_64_ZIP" \
  "$MODEL_ZIP" \
  "$MANIFEST_JSON"

echo "Pyannote release assets ready in $OUTPUT_DIR"
