#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 5 ]]; then
  echo "Usage: $0 <version> <arm64-staging-dir> <intel-staging-dir> <windows-staging-dir> <output-dir>" >&2
  exit 1
fi

VERSION=$1
ARM_DIR=$2
INTEL_DIR=$3
WINDOWS_DIR=$4
OUTPUT_DIR=$5
ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
REPO_SLUG=${SBOBINO_RELEASE_REPOSITORY:-pietroMastro92/Sbobino}
RELEASE_URL="https://github.com/$REPO_SLUG/releases/download/v$VERSION"

for path in \
  "$ARM_DIR/Sbobino_${VERSION}_aarch64.dmg" \
  "$ARM_DIR/Sbobino_${VERSION}_aarch64.app.tar.gz" \
  "$ARM_DIR/Sbobino_${VERSION}_aarch64.app.tar.gz.sig" \
  "$ARM_DIR/speech-runtime-macos-aarch64.zip" \
  "$ARM_DIR/pyannote-runtime-macos-aarch64.zip" \
  "$ARM_DIR/pyannote-model-community-1.zip" \
  "$INTEL_DIR/Sbobino_${VERSION}_x86_64.dmg" \
  "$INTEL_DIR/Sbobino_${VERSION}_x86_64.app.tar.gz" \
  "$INTEL_DIR/Sbobino_${VERSION}_x86_64.app.tar.gz.sig" \
  "$INTEL_DIR/speech-runtime-macos-x86_64.zip" \
  "$INTEL_DIR/pyannote-runtime-macos-x86_64.zip" \
  "$WINDOWS_DIR/Sbobino_${VERSION}_windows_x86_64-setup.exe" \
  "$WINDOWS_DIR/Sbobino_${VERSION}_windows_x86_64.nsis.zip" \
  "$WINDOWS_DIR/Sbobino_${VERSION}_windows_x86_64.nsis.zip.sig" \
  "$WINDOWS_DIR/speech-runtime-windows-x86_64.zip" \
  "$WINDOWS_DIR/pyannote-runtime-windows-x86_64.zip"
do
  if [[ ! -f "$path" ]]; then
    echo "Missing native staging artifact: $path" >&2
    exit 1
  fi
done

mkdir -p "$OUTPUT_DIR"
for source in \
  "$ARM_DIR/Sbobino_${VERSION}_aarch64.dmg" \
  "$ARM_DIR/Sbobino_${VERSION}_aarch64.app.tar.gz" \
  "$ARM_DIR/Sbobino_${VERSION}_aarch64.app.tar.gz.sig" \
  "$ARM_DIR/speech-runtime-macos-aarch64.zip" \
  "$ARM_DIR/pyannote-runtime-macos-aarch64.zip" \
  "$ARM_DIR/pyannote-model-community-1.zip" \
  "$INTEL_DIR/Sbobino_${VERSION}_x86_64.dmg" \
  "$INTEL_DIR/Sbobino_${VERSION}_x86_64.app.tar.gz" \
  "$INTEL_DIR/Sbobino_${VERSION}_x86_64.app.tar.gz.sig" \
  "$INTEL_DIR/speech-runtime-macos-x86_64.zip" \
  "$INTEL_DIR/pyannote-runtime-macos-x86_64.zip" \
  "$WINDOWS_DIR/Sbobino_${VERSION}_windows_x86_64-setup.exe" \
  "$WINDOWS_DIR/Sbobino_${VERSION}_windows_x86_64.nsis.zip" \
  "$WINDOWS_DIR/Sbobino_${VERSION}_windows_x86_64.nsis.zip.sig" \
  "$WINDOWS_DIR/speech-runtime-windows-x86_64.zip" \
  "$WINDOWS_DIR/pyannote-runtime-windows-x86_64.zip"
do
  cp "$source" "$OUTPUT_DIR/"
done

"$ROOT_DIR/scripts/generate_release_manifests.sh" "$VERSION" "$OUTPUT_DIR"

ARM_SIGNATURE=$(tr -d '\n' < "$OUTPUT_DIR/Sbobino_${VERSION}_aarch64.app.tar.gz.sig")
INTEL_SIGNATURE=$(tr -d '\n' < "$OUTPUT_DIR/Sbobino_${VERSION}_x86_64.app.tar.gz.sig")
WINDOWS_SIGNATURE=$(tr -d '\n' < "$OUTPUT_DIR/Sbobino_${VERSION}_windows_x86_64.nsis.zip.sig")
python3 - "$OUTPUT_DIR/latest.json" "$VERSION" "$RELEASE_URL" "$ARM_SIGNATURE" "$INTEL_SIGNATURE" "$WINDOWS_SIGNATURE" <<'PY'
import json
import sys
from datetime import datetime, timezone

path, version, base_url, arm_sig, intel_sig, windows_sig = sys.argv[1:]
document = {
    "version": version,
    "pub_date": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "platforms": {
        "darwin-aarch64": {
            "url": f"{base_url}/Sbobino_{version}_aarch64.app.tar.gz",
            "signature": arm_sig,
        },
        "darwin-x86_64": {
            "url": f"{base_url}/Sbobino_{version}_x86_64.app.tar.gz",
            "signature": intel_sig,
        },
        "windows-x86_64": {
            "url": f"{base_url}/Sbobino_{version}_windows_x86_64.nsis.zip",
            "signature": windows_sig,
        },
    },
}
with open(path, "w", encoding="utf-8") as handle:
    json.dump(document, handle, indent=2)
    handle.write("\n")
PY

python3 "$ROOT_DIR/scripts/generate_release_candidate_metadata.py" \
  "$OUTPUT_DIR" \
  "$VERSION" \
  --release-profile public \
  --commit-sha "$(git -C "$ROOT_DIR/.." rev-parse HEAD)"

cat >"$OUTPUT_DIR/release-notes.md" <<EOF
## Sbobino $VERSION

This prerelease provides native Apple Silicon, macOS Intel, and Windows x86_64
installers, with architecture-matched updaters and first-launch runtime assets.
EOF

cat <<EOF
Multi-platform local candidate prepared in:
  $OUTPUT_DIR

Required native DMGs:
  - Sbobino_${VERSION}_aarch64.dmg
  - Sbobino_${VERSION}_x86_64.dmg
  - Sbobino_${VERSION}_windows_x86_64-setup.exe
EOF
