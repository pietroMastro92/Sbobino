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

live_smoke="$script_dir/release_macos_whisper_live_smoke.sh"
if grep -Eq 'strings[[:space:]]+"?\$WHISPER_BIN"?[[:space:]]*\|[[:space:]]*grep' "$live_smoke"; then
  printf 'macOS Whisper live smoke must not combine strings and early-exit grep under pipefail\n' >&2
  exit 1
fi
for required in \
  'strings "$WHISPER_BIN" > "$WHISPER_STRINGS"' \
  'grep -Fq "SBOBINO_WHISPER_REPLAY_WAV" "$WHISPER_STRINGS"' \
  'grep -Fq "SBOBINO_WHISPER_LIVE_METRIC" "$WHISPER_STRINGS"'; do
  if ! grep -Fq -- "$required" "$live_smoke"; then
    printf 'macOS Whisper live smoke is missing pipefail-safe binary hook validation: %s\n' "$required" >&2
    exit 1
  fi
done

if ! grep -Fq -- 'assert_binary_portable "$TARGET_BIN/parakeet-batch-json"' "$source_file"; then
  printf 'standalone worker must remain covered by assert_binary_portable\n' >&2
  exit 1
fi

for required in \
  '-DGGML_METAL=ON' \
  '-DGGML_ACCELERATE=ON' \
  '-DWHISPER_COREML="$WHISPER_COREML"' \
  '-DWHISPER_COREML_ALLOW_FALLBACK=ON'; do
  if ! grep -Fq -- "$required" "$source_file"; then
    printf 'packaged Whisper must enable the native macOS backend: %s\n' "$required" >&2
    exit 1
  fi
done

if ! grep -Fq -- 'SBOBINO_RUNTIME_STAGE_PARENT' "$source_file"; then
  printf 'macOS runtime staging must support an explicit build volume\n' >&2
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

if ! grep -Fq -- 'std::this_thread::sleep_until(replay_deadline)' "$script_dir/patches/whisper-stream-audio-file.patch"; then
  printf 'Whisper replay must use absolute deadlines without accumulating scheduler drift\n' >&2
  exit 1
fi

if ! grep -Fq -- 'SBOBINO_WHISPER_LIVE_BACKLOG' "$script_dir/patches/whisper-stream-backlog.patch"; then
  printf 'Whisper live backlog patch must fail closed instead of dropping audio\n' >&2
  exit 1
fi
for required in \
  'failed to warm up the live transcription backend' \
  'whisper_reset_timings(ctx)' \
  'SBOBINO_WHISPER_LIVE_PREFLIGHT' \
  'SBOBINO_WHISPER_SKIP_LIVE_PREFLIGHT' \
  'SBOBINO_WHISPER_TEST_PREFLIGHT_DELAY_MS' \
  'SBOBINO_WHISPER_TEST_PREFLIGHT_FIRST_ROUND_DELAY_MS' \
  'probe_round < 2' \
  'if (probe_round == 0) {' \
  'probe_index < 3' \
  'std::max(std::min(probe_a_ms, probe_b_ms)' \
  'round_max_ms <= budget_ms' \
  'const bool probe_passed = skip_probe || probe_round_passed;' \
  'params.max_tokens     = params.max_tokens > 0 ? std::min(params.max_tokens, 16) : 16;' \
  'const uint64_t backlog = audio.buffered_samples();' \
  'std::remove(capture_filename.c_str())'; do
  if ! grep -Fq -- "$required" "$script_dir/patches/whisper-stream-backlog.patch"; then
    printf 'Whisper live warmup contract is missing: %s\n' "$required" >&2
    exit 1
  fi
done

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
  'const uint64_t backlog = audio.buffered_samples();' \
  'audio.pause();' \
  'inference_backlog_failed = true'; do
  if ! grep -Fq -- "$required" "$lossless_patch"; then
    printf 'Whisper live lossless-drain contract is missing: %s\n' "$required" >&2
    exit 1
  fi
done

if grep -Fq -- 'captured > n_samples_inferred' \
    "$script_dir/patches/whisper-stream-backlog.patch" \
    "$lossless_patch"; then
  printf 'Whisper live backlog must exclude audio already dequeued for inference\n' >&2
  exit 1
fi

printf 'macOS runtime source contract passed: %s\n' "$source_file"
