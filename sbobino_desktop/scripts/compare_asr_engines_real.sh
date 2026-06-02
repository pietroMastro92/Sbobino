#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source "$ROOT_DIR/scripts/lib/asr_samples.sh"

APP_ID=${SBOBINO_APP_ID:-com.sbobino.desktop}
APP_SUPPORT_DIR=${SBOBINO_APP_SUPPORT_DIR:-"$HOME/Library/Application Support/$APP_ID"}
PARAKEET_MODELS_DIR=${SBOBINO_PARAKEET_MODELS_DIR:-"$APP_SUPPORT_DIR/parakeet-models"}
PARAKEET_MODEL=${SBOBINO_PARAKEET_MODEL:-tdt-0.6b-v3-q4_k.gguf}
WHISPER_MODELS_DIR=${SBOBINO_WHISPER_MODELS_DIR:-"$APP_SUPPORT_DIR/models"}
WHISPER_MODEL=${SBOBINO_WHISPER_MODEL:-ggml-base.bin}
WHISPER_LANGUAGE=${SBOBINO_WHISPER_LANGUAGE:-en}
COMPARE_MODE=${SBOBINO_ASR_COMPARE_MODE:-transcribe}
THREADS=${SBOBINO_ASR_THREADS:-8}
BENCH_REPS=${SBOBINO_ASR_BENCH_REPS:-5}
REQUIRE_PARAKEET_GPU=${SBOBINO_REQUIRE_PARAKEET_GPU:-1}
SKIP_PARAKEET_CPU=${SBOBINO_COMPARE_SKIP_PARAKEET_CPU:-}
if [[ -z "$SKIP_PARAKEET_CPU" && "$COMPARE_MODE" == "bench" ]]; then
  # parakeet-cli bench-decode currently hangs on CPU-forced runs on this Apple Silicon setup.
  # Use transcribe mode for Metal-vs-CPU comparisons, and bench mode for Metal steady-state.
  SKIP_PARAKEET_CPU=1
elif [[ -z "$SKIP_PARAKEET_CPU" ]]; then
  SKIP_PARAKEET_CPU=0
fi

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    asr_fail "missing required command: $1"
  fi
}

target_triple() {
  case "$(uname -m)" in
    arm64) printf '%s\n' "aarch64-apple-darwin" ;;
    x86_64) printf '%s\n' "x86_64-apple-darwin" ;;
    *) asr_fail "unsupported macOS architecture: $(uname -m)" ;;
  esac
}

first_executable() {
  local candidate
  for candidate in "$@"; do
    if [[ -n "$candidate" && -x "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  return 1
}

resolve_command_path() {
  local command_name=$1
  command -v "$command_name" 2>/dev/null || true
}

rtfx_for() {
  local duration=$1
  local real=$2
  awk -v duration="$duration" -v real="$real" 'BEGIN {
    if (real > 0) {
      printf "%.2f", duration / real
    } else {
      printf "n/a"
    }
  }'
}

preview_file() {
  local path=$1
  if [[ ! -s "$path" ]]; then
    printf '%s\n' ""
    return 0
  fi
  tr '\n' ' ' < "$path" | cut -c 1-240
  printf '\n'
}

model_quantization() {
  local model=$1
  case "$model" in
    *-q4_k.gguf|*q4_k*) printf '%s\n' "q4_k" ;;
    *-q5_k.gguf|*q5_k*) printf '%s\n' "q5_k" ;;
    *-q6_k.gguf|*q6_k*) printf '%s\n' "q6_k" ;;
    *-q8_0.gguf|*q8_0*) printf '%s\n' "q8_0" ;;
    *-f16.gguf|*f16*) printf '%s\n' "f16" ;;
    *) printf '%s\n' "unknown" ;;
  esac
}

run_timed() {
  local label=$1
  shift
  local stdout="$RUN_DIR/$label.stdout"
  local stderr="$RUN_DIR/$label.stderr"
  local status
  local real
  local rtfx
  local preview

  echo "run=$label"
  set +e
  /usr/bin/time -p "$@" >"$stdout" 2>"$stderr"
  status=$?
  set -e

  real=$(awk '$1 == "real" { value = $2 } END { print value }' "$stderr")
  [[ -n "$real" ]] || real="0"
  rtfx=$(rtfx_for "$DURATION_SECONDS" "$real")
  preview=$(preview_file "$stdout")

  echo "${label}_status=$status"
  echo "${label}_real_seconds=$real"
  echo "${label}_rtfx=$rtfx"
  echo "${label}_stdout=$stdout"
  echo "${label}_stderr=$stderr"
  if [[ -n "$preview" ]]; then
    echo "${label}_preview=$preview"
  fi
  echo

  return "$status"
}

parakeet_gpu_used() {
  local stderr=$1
  grep -Eq "pk::Backend using GPU device|ggml_metal_init: found device|ggml_metal_init: picking default device: Apple" "$stderr"
}

parakeet_command_args() {
  if [[ "$COMPARE_MODE" == "bench" ]]; then
    PARAKEET_ARGS=(bench-decode --model "$PARAKEET_MODEL_PATH" --audio "$ASR_NORMALIZED_WAV" --threads "$THREADS" --reps "$BENCH_REPS")
  elif [[ "$COMPARE_MODE" == "transcribe" ]]; then
    PARAKEET_ARGS=(transcribe --model "$PARAKEET_MODEL_PATH" --input "$ASR_NORMALIZED_WAV" --json)
  else
    asr_fail "unsupported SBOBINO_ASR_COMPARE_MODE '$COMPARE_MODE' (expected transcribe or bench)"
  fi
}

need_cmd awk
need_cmd cut
need_cmd ffmpeg
need_cmd ffprobe
need_cmd grep
need_cmd tr

TRIPLE=$(target_triple)
BIN_DIR="$ROOT_DIR/apps/desktop/src-tauri/binaries"
PARAKEET_CLI=${SBOBINO_PARAKEET_CLI:-}
if [[ -z "$PARAKEET_CLI" ]]; then
  PARAKEET_CLI=$(first_executable \
    "$BIN_DIR/parakeet-cli-$TRIPLE" \
    "$APP_SUPPORT_DIR/bin/parakeet-cli" \
    "$(resolve_command_path parakeet-cli)") || true
fi
[[ -n "$PARAKEET_CLI" && -x "$PARAKEET_CLI" ]] || asr_fail "missing executable parakeet-cli"

WHISPER_CLI=${SBOBINO_WHISPER_CLI:-}
if [[ -z "$WHISPER_CLI" ]]; then
  WHISPER_CLI=$(first_executable \
    "$BIN_DIR/whisper-cli-$TRIPLE" \
    "$(resolve_command_path whisper-cli)") || true
fi

PARAKEET_MODEL_PATH="$PARAKEET_MODELS_DIR/$PARAKEET_MODEL"
WHISPER_MODEL_PATH="$WHISPER_MODELS_DIR/$WHISPER_MODEL"
[[ -f "$PARAKEET_MODEL_PATH" ]] || asr_fail "missing Parakeet model: $PARAKEET_MODEL_PATH"

RUN_DIR=$(mktemp -d "${TMPDIR:-/tmp}/sbobino-asr-compare.XXXXXX")
cleanup() {
  if [[ "${SBOBINO_KEEP_ASR_COMPARE_OUTPUT:-1}" == "1" ]]; then
    echo "kept_compare_output=$RUN_DIR"
  else
    rm -rf "$RUN_DIR"
  fi
}
trap cleanup EXIT

asr_resolve_source "$ROOT_DIR"
asr_prepare_wav "$ASR_SOURCE_PATH" "$RUN_DIR/source.wav"
DURATION_SECONDS=$(asr_audio_duration_seconds "$ASR_NORMALIZED_WAV")

asr_print_source_report "$DURATION_SECONDS"
echo "compare_mode=$COMPARE_MODE"
echo "bench_reps=$BENCH_REPS"
echo "parakeet_cpu_compare=$([[ "$SKIP_PARAKEET_CPU" == "1" ]] && echo skipped || echo enabled)"
echo "parakeet_cli=$PARAKEET_CLI"
echo "parakeet_model=$PARAKEET_MODEL_PATH"
echo "parakeet_quantization=$(model_quantization "$PARAKEET_MODEL")"
echo "parakeet_required_device=$([[ "$REQUIRE_PARAKEET_GPU" == "1" ]] && echo metal || echo any)"
echo "whisper_cli=${WHISPER_CLI:-missing}"
echo "whisper_model=$WHISPER_MODEL_PATH"
echo

parakeet_command_args
PARAKEET_STATUS=0
run_timed parakeet_default \
  env DYLD_LIBRARY_PATH="$BIN_DIR:${DYLD_LIBRARY_PATH:-}" "$PARAKEET_CLI" "${PARAKEET_ARGS[@]}" \
  || PARAKEET_STATUS=$?
if [[ "$PARAKEET_STATUS" -eq 0 && "$REQUIRE_PARAKEET_GPU" == "1" ]]; then
  if parakeet_gpu_used "$RUN_DIR/parakeet_default.stderr"; then
    echo "parakeet_default_gpu=used"
  else
    echo "parakeet_default_gpu=missing"
    echo "error: Parakeet default run did not initialize Metal GPU. See $RUN_DIR/parakeet_default.stderr" >&2
    PARAKEET_STATUS=1
  fi
  echo
fi

if [[ "$SKIP_PARAKEET_CPU" != "1" ]]; then
  parakeet_command_args
  run_timed parakeet_cpu \
    env PARAKEET_DEVICE=cpu DYLD_LIBRARY_PATH="$BIN_DIR:${DYLD_LIBRARY_PATH:-}" "$PARAKEET_CLI" "${PARAKEET_ARGS[@]}" \
    || true
else
  echo "parakeet_cpu_status=skipped"
fi

if [[ "$COMPARE_MODE" == "bench" ]]; then
  echo "whisper_status=skipped_for_parakeet_bench_mode"
elif [[ -n "${WHISPER_CLI:-}" && -x "$WHISPER_CLI" && -f "$WHISPER_MODEL_PATH" ]]; then
  if [[ "${SBOBINO_COMPARE_SKIP_WHISPER_GPU:-0}" != "1" ]]; then
    run_timed whisper_default \
      env DYLD_LIBRARY_PATH="$BIN_DIR:${DYLD_LIBRARY_PATH:-}" "$WHISPER_CLI" \
      -m "$WHISPER_MODEL_PATH" \
      -f "$ASR_NORMALIZED_WAV" \
      -l "$WHISPER_LANGUAGE" \
      -oj -ojf \
      -of "$RUN_DIR/whisper_default" \
      -np || true
  fi

  run_timed whisper_cpu \
    env DYLD_LIBRARY_PATH="$BIN_DIR:${DYLD_LIBRARY_PATH:-}" "$WHISPER_CLI" \
    --no-gpu \
    -m "$WHISPER_MODEL_PATH" \
    -f "$ASR_NORMALIZED_WAV" \
    -l "$WHISPER_LANGUAGE" \
    -oj -ojf \
    -of "$RUN_DIR/whisper_cpu" \
    -np || true
else
  echo "whisper_status=missing"
fi

exit "$PARAKEET_STATUS"
