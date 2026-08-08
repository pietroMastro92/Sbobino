#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
source_file=${1:-"$script_dir/package_macos_runtime_asset.sh"}

if [[ ! -f "$source_file" ]]; then
  printf 'missing macOS runtime source: %s\n' "$source_file" >&2
  exit 1
fi

# Keep this contract scoped to the standalone worker invocation. The other
# runtime binaries are built by CMake or FFmpeg and have their own target
# settings; this command must carry all three settings itself.
worker_compile=$(
  awk '
    /^[[:space:]]*clang\+\+ -std=c\+\+17 \\/ { capture = 1 }
    capture { print }
    capture && /-o "\$TARGET_BIN\/parakeet-batch-json"/ { exit }
  ' "$source_file"
)

if [[ -z "$worker_compile" ]]; then
  printf 'missing standalone parakeet-batch-json clang++ invocation\n' >&2
  exit 1
fi

for required in \
  '-arch "$RUNTIME_ARCH"' \
  '-mmacosx-version-min="$MACOS_DEPLOYMENT_TARGET"' \
  '-isysroot "$SDKROOT"'; do
  if ! grep -Fq -- "$required" <<<"$worker_compile"; then
    printf 'missing standalone worker portability flag: %s\n' "$required" >&2
    exit 1
  fi
done

if ! grep -Fq -- 'assert_binary_portable "$TARGET_BIN/parakeet-batch-json"' "$source_file"; then
  printf 'standalone worker must remain covered by assert_binary_portable\n' >&2
  exit 1
fi

printf 'macOS runtime source contract passed: %s\n' "$source_file"
