#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 5 ]]; then
  echo "Usage: $0 <version> <runtime_aarch64_zip> <runtime_x86_64_zip> <model_zip> <output_json>" >&2
  exit 1
fi

VERSION=$1
RUNTIME_AARCH64=$2
RUNTIME_X86_64=$3
MODEL_ZIP=$4
OUTPUT_JSON=$5

for path in "$RUNTIME_AARCH64" "$RUNTIME_X86_64" "$MODEL_ZIP"; do
  if [[ ! -f "$path" ]]; then
    echo "Missing asset: $path" >&2
    exit 1
  fi
done

mkdir -p "$(dirname "$OUTPUT_JSON")"

sha256() {
  shasum -a 256 "$1" | awk '{print $1}'
}

cat >"$OUTPUT_JSON" <<JSON
{
  "app_version": "$VERSION",
  "assets": [
    {
      "kind": "pyannote_runtime_macos_aarch64",
      "name": "$(basename "$RUNTIME_AARCH64")",
      "sha256": "$(sha256 "$RUNTIME_AARCH64")"
    },
    {
      "kind": "pyannote_runtime_macos_x86_64",
      "name": "$(basename "$RUNTIME_X86_64")",
      "sha256": "$(sha256 "$RUNTIME_X86_64")"
    },
    {
      "kind": "pyannote_model",
      "name": "$(basename "$MODEL_ZIP")",
      "sha256": "$(sha256 "$MODEL_ZIP")"
    }
  ]
}
JSON

echo "Created $OUTPUT_JSON"
