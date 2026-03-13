#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "Usage: $0 <source_dir> <archive_root_name> <output_zip>" >&2
  exit 1
fi

SOURCE_DIR=$1
ARCHIVE_ROOT_NAME=$2
OUTPUT_ZIP=$3

if [[ ! -d "$SOURCE_DIR" ]]; then
  echo "Source directory not found: $SOURCE_DIR" >&2
  exit 1
fi

mkdir -p "$(dirname "$OUTPUT_ZIP")"
rm -f "$OUTPUT_ZIP"

STAGE_DIR=$(mktemp -d)
trap 'rm -rf "$STAGE_DIR"' EXIT

TARGET_ROOT="$STAGE_DIR/$ARCHIVE_ROOT_NAME"
mkdir -p "$(dirname "$TARGET_ROOT")"
cp -R "$SOURCE_DIR" "$TARGET_ROOT"

ditto -c -k --sequesterRsrc --keepParent "$TARGET_ROOT" "$OUTPUT_ZIP"
echo "Created $OUTPUT_ZIP"
