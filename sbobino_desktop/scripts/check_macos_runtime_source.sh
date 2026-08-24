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

for patch_name in whisper-stream-audio-file.patch whisper-stream-fifo.patch whisper-stream-backlog.patch whisper-stream-finalization.patch whisper-stream-lossless-drain.patch; do
  patch_path="$script_dir/patches/$patch_name"
  if [[ ! -s "$patch_path" ]]; then
    printf 'missing pinned Whisper live patch: %s\n' "$patch_path" >&2
    exit 1
  fi
  if ! grep -Fq -- "patch -d \"\$source_root\" -p1 < \"\$SCRIPT_DIR/patches/$patch_name\"" "$source_file"; then
    printf 'macOS runtime package does not apply pinned patch: %s\n' "$patch_name" >&2
    exit 1
  fi
done

if ! grep -Fq -- 'SBOBINO_WHISPER_LIVE_BACKLOG' "$script_dir/patches/whisper-stream-backlog.patch"; then
  printf 'Whisper live backlog patch must fail closed instead of dropping audio\n' >&2
  exit 1
fi

if ! awk '
  /^\+[[:space:]]+audio\.pause\(\);/ {
    if ((getline next_line) > 0 && next_line ~ /^\+[[:space:]]+audio\.get\(0, pcmf32_new\);/) {
      found = 1
    }
  }
  END { exit(found ? 0 : 1) }
' "$script_dir/patches/whisper-stream-backlog.patch"; then
  printf 'Whisper live must stop capture before draining the final saved-audio tail\n' >&2
  exit 1
fi

lossless_patch="$script_dir/patches/whisper-stream-lossless-drain.patch"
for required in \
  'std::vector<float> expanded' \
  'if (!m_running && ms > 0)' \
  'SBOBINO_WHISPER_TEST_INFERENCE_DELAY_MS' \
  'std::thread backlog_monitor' \
  'audio.pause();' \
  'inference_backlog_failed = true'; do
  if ! grep -Fq -- "$required" "$lossless_patch"; then
    printf 'Whisper live lossless-drain contract is missing: %s\n' "$required" >&2
    exit 1
  fi
done

printf 'macOS runtime source contract passed: %s\n' "$source_file"
