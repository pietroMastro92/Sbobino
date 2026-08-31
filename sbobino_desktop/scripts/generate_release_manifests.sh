#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "Usage: $0 <version> <asset-dir>" >&2
  exit 1
fi

VERSION=${1#v}
ASSET_DIR=$2

if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Release version must be semantic MAJOR.MINOR.PATCH (got '$VERSION')." >&2
  exit 1
fi

RUNTIME_ZIP="$ASSET_DIR/speech-runtime-macos-aarch64.zip"
RUNTIME_X86_64_ZIP="$ASSET_DIR/speech-runtime-macos-x86_64.zip"
PYANNOTE_RUNTIME_ZIP="$ASSET_DIR/pyannote-runtime-macos-aarch64.zip"
PYANNOTE_RUNTIME_X86_64_ZIP="$ASSET_DIR/pyannote-runtime-macos-x86_64.zip"
RUNTIME_WINDOWS_ZIP="$ASSET_DIR/speech-runtime-windows-x86_64.zip"
PYANNOTE_RUNTIME_WINDOWS_ZIP="$ASSET_DIR/pyannote-runtime-windows-x86_64.zip"
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
  "$RUNTIME_WINDOWS_ZIP" \
  "$PYANNOTE_RUNTIME_WINDOWS_ZIP" \
  "$PYANNOTE_MODEL_ZIP"
do
  if [[ ! -s "$path" ]]; then
    echo "Missing required release asset: $path" >&2
    exit 1
  fi
done

python3 - "$RUNTIME_ZIP" "$RUNTIME_X86_64_ZIP" "$RUNTIME_WINDOWS_ZIP" \
  "$PYANNOTE_RUNTIME_ZIP" "$PYANNOTE_RUNTIME_X86_64_ZIP" \
  "$PYANNOTE_RUNTIME_WINDOWS_ZIP" "$PYANNOTE_MODEL_ZIP" <<'PY'
import pathlib
import sys
import zipfile


def members(path: pathlib.Path) -> set[str]:
    try:
        with zipfile.ZipFile(path) as archive:
            names = {
                name.replace("\\", "/").lstrip("./")
                for name in archive.namelist()
                if name and not name.endswith("/")
            }
    except (OSError, zipfile.BadZipFile) as error:
        raise SystemExit(f"release asset is not a readable ZIP archive: {path}: {error}") from error
    if not names:
        raise SystemExit(f"release asset ZIP is empty: {path}")
    return names


def require(path: pathlib.Path, alternatives: tuple[str, ...], label: str) -> None:
    names = members(path)
    if not any(candidate in names for candidate in alternatives):
        joined = ", ".join(alternatives)
        raise SystemExit(f"{label} is missing from {path}; expected one of: {joined}")


mac_runtime, intel_runtime, windows_runtime, mac_pyannote, intel_pyannote, windows_pyannote, model = map(
    pathlib.Path, sys.argv[1:]
)
for path, label in (
    (mac_runtime, "Apple Silicon speech runtime"),
    (intel_runtime, "Intel speech runtime"),
):
    for binary in ("ffmpeg", "whisper-cli", "whisper-stream", "parakeet-cli", "parakeet-batch-json"):
        require(path, (f"runtime/bin/{binary}",), label)

for binary in ("ffmpeg.exe", "whisper-cli.exe", "whisper-stream.exe", "parakeet-cli.exe", "parakeet-batch-json.exe"):
    require(windows_runtime, (f"runtime/bin/{binary}",), "Windows speech runtime")

for path, label in ((mac_pyannote, "Apple Silicon Pyannote runtime"), (intel_pyannote, "Intel Pyannote runtime")):
    require(path, ("python/bin/python3",), label)
require(windows_pyannote, ("python/python.exe",), "Windows Pyannote runtime")
require(model, ("model/config.yaml",), "Pyannote model asset")
PY

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
RUNTIME_WINDOWS_SHA=$(sha256 "$RUNTIME_WINDOWS_ZIP")
PYANNOTE_RUNTIME_WINDOWS_SHA=$(sha256 "$PYANNOTE_RUNTIME_WINDOWS_ZIP")
PYANNOTE_MODEL_SHA=$(sha256 "$PYANNOTE_MODEL_ZIP")
RUNTIME_SIZE=$(file_size_bytes "$RUNTIME_ZIP")
RUNTIME_EXPANDED_SIZE=$(zip_expanded_size_bytes "$RUNTIME_ZIP")
RUNTIME_X86_64_SIZE=$(file_size_bytes "$RUNTIME_X86_64_ZIP")
RUNTIME_X86_64_EXPANDED_SIZE=$(zip_expanded_size_bytes "$RUNTIME_X86_64_ZIP")
PYANNOTE_RUNTIME_SIZE=$(file_size_bytes "$PYANNOTE_RUNTIME_ZIP")
PYANNOTE_RUNTIME_EXPANDED_SIZE=$(zip_expanded_size_bytes "$PYANNOTE_RUNTIME_ZIP")
PYANNOTE_RUNTIME_X86_64_SIZE=$(file_size_bytes "$PYANNOTE_RUNTIME_X86_64_ZIP")
PYANNOTE_RUNTIME_X86_64_EXPANDED_SIZE=$(zip_expanded_size_bytes "$PYANNOTE_RUNTIME_X86_64_ZIP")
RUNTIME_WINDOWS_SIZE=$(file_size_bytes "$RUNTIME_WINDOWS_ZIP")
RUNTIME_WINDOWS_EXPANDED_SIZE=$(zip_expanded_size_bytes "$RUNTIME_WINDOWS_ZIP")
PYANNOTE_RUNTIME_WINDOWS_SIZE=$(file_size_bytes "$PYANNOTE_RUNTIME_WINDOWS_ZIP")
PYANNOTE_RUNTIME_WINDOWS_EXPANDED_SIZE=$(zip_expanded_size_bytes "$PYANNOTE_RUNTIME_WINDOWS_ZIP")
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
    },
    {
      "kind": "speech_runtime_windows_x86_64",
      "name": "$(basename "$RUNTIME_WINDOWS_ZIP")",
      "sha256": "$RUNTIME_WINDOWS_SHA",
      "size_bytes": $RUNTIME_WINDOWS_SIZE,
      "expanded_size_bytes": $RUNTIME_WINDOWS_EXPANDED_SIZE
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
      "kind": "pyannote_runtime_windows_x86_64",
      "name": "$(basename "$PYANNOTE_RUNTIME_WINDOWS_ZIP")",
      "sha256": "$PYANNOTE_RUNTIME_WINDOWS_SHA",
      "size_bytes": $PYANNOTE_RUNTIME_WINDOWS_SIZE,
      "expanded_size_bytes": $PYANNOTE_RUNTIME_WINDOWS_EXPANDED_SIZE
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
    },
    "x86_64-pc-windows-msvc": {
      "name": "$(basename "$RUNTIME_WINDOWS_ZIP")",
      "sha256": "$RUNTIME_WINDOWS_SHA",
      "size_bytes": $RUNTIME_WINDOWS_SIZE,
      "expanded_size_bytes": $RUNTIME_WINDOWS_EXPANDED_SIZE
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
    },
    "x86_64-pc-windows-msvc": {
      "name": "$(basename "$PYANNOTE_RUNTIME_WINDOWS_ZIP")",
      "sha256": "$PYANNOTE_RUNTIME_WINDOWS_SHA",
      "size_bytes": $PYANNOTE_RUNTIME_WINDOWS_SIZE,
      "expanded_size_bytes": $PYANNOTE_RUNTIME_WINDOWS_EXPANDED_SIZE
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
