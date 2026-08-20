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

assert_portable_runtime() {
  # A package assembled from the wrong machine can look healthy until
  # TorchCodec imports it.  Reject host-managed dylib references and a
  # mismatched Mach-O slice before creating a release archive.
  if [[ "$ARCHIVE_ROOT_NAME" != "python" || "$(uname -s)" != "Darwin" ]]; then
    return 0
  fi

  local expected_arch
  case "$(uname -m)" in
    arm64) expected_arch=arm64 ;;
    x86_64) expected_arch=x86_64 ;;
    *) echo "Unsupported macOS packaging architecture: $(uname -m)" >&2; exit 1 ;;
  esac

  local binary architectures
  while IFS= read -r -d '' binary; do
    if [[ -L "$binary" ]]; then
      local resolved
      resolved=$(python3 - "$SOURCE_DIR" "$binary" <<'PY'
import os
import pathlib
import sys

root = pathlib.Path(sys.argv[1]).resolve()
path = pathlib.Path(sys.argv[2]).resolve()
try:
    path.relative_to(root)
except ValueError:
    raise SystemExit(f"symlink resolves outside Pyannote runtime: {sys.argv[2]} -> {path}")
print(path)
PY
      ) || exit 1
      binary=$resolved
    fi

    if ! architectures=$(lipo -archs "$binary" 2>/dev/null); then
      echo "Pyannote packaging rejected non-Mach-O native module '$binary'." >&2
      exit 1
    fi
    if [[ " $architectures " != *" $expected_arch "* ]]; then
      echo "Pyannote packaging rejected architecture-mismatched dylib '$binary': expected $expected_arch, got $architectures" >&2
      exit 1
    fi

    local dependencies
    dependencies=$(otool -L "$binary" 2>/dev/null || true)
    if printf '%s\n' "$dependencies" | tail -n +2 | awk '{print $1}' | grep -Eq '^(/opt/homebrew|/usr/local|/Library/Frameworks)'; then
      echo "Pyannote packaging rejected host-linked dylib '$binary'." >&2
      printf '%s\n' "$dependencies" >&2
      exit 1
    fi
  done < <(find "$SOURCE_DIR" \( -type f -o -type l \) \( -name '*.dylib' -o -name '*.so' \) -print0)

  local python_binary="$SOURCE_DIR/bin/python3"
  if [[ -x "$python_binary" ]]; then
    if ! architectures=$(lipo -archs "$python_binary" 2>/dev/null); then
      echo "Pyannote packaging rejected non-Mach-O Python launcher: $python_binary" >&2
      exit 1
    fi
    if [[ " $architectures " != *" $expected_arch "* ]]; then
      echo "Pyannote packaging rejected architecture-mismatched Python launcher: expected $expected_arch, got $architectures" >&2
      exit 1
    fi
  fi
}

assert_portable_runtime

mkdir -p "$(dirname "$OUTPUT_ZIP")"
rm -f "$OUTPUT_ZIP"

STAGE_DIR=$(mktemp -d)
trap 'rm -rf "$STAGE_DIR"' EXIT

TARGET_ROOT="$STAGE_DIR/$ARCHIVE_ROOT_NAME"
mkdir -p "$(dirname "$TARGET_ROOT")"
cp -R "$SOURCE_DIR" "$TARGET_ROOT"

ditto -c -k --sequesterRsrc --keepParent "$TARGET_ROOT" "$OUTPUT_ZIP"
echo "Created $OUTPUT_ZIP"
