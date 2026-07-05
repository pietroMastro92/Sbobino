#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source "$ROOT_DIR/scripts/lib/asr_samples.sh"

MODEL_FILENAME=${SBOBINO_PARAKEET_MODEL:-tdt-0.6b-v3-f16.gguf}
MODEL_URL="https://huggingface.co/mudler/parakeet-cpp-gguf/resolve/main/$MODEL_FILENAME"
NEMOTRON_STREAMING_Q4_MODEL="nemotron-3.5-asr-streaming-0.6b-q4_k.gguf"
NEMOTRON_STREAMING_Q4_URL="https://huggingface.co/mudler/parakeet-cpp-gguf/resolve/main/$NEMOTRON_STREAMING_Q4_MODEL"

fail() {
  echo "error: $*" >&2
  exit 1
}

require_abs_path() {
  local name=$1
  local value=${!name:-}
  if [[ -z "$value" ]]; then
    fail "$name must be set"
  fi
  if [[ "$value" != /* ]]; then
    fail "$name must be an absolute path: $value"
  fi
}

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    fail "missing required command: $1"
  fi
}

require_abs_path SBOBINO_PARAKEET_CLI
require_abs_path SBOBINO_PARAKEET_MODELS_DIR

[[ -x "$SBOBINO_PARAKEET_CLI" ]] || fail "SBOBINO_PARAKEET_CLI is not executable: $SBOBINO_PARAKEET_CLI"
[[ -d "$SBOBINO_PARAKEET_MODELS_DIR" ]] || fail "SBOBINO_PARAKEET_MODELS_DIR is not a directory: $SBOBINO_PARAKEET_MODELS_DIR"

MODEL_PATH="$SBOBINO_PARAKEET_MODELS_DIR/$MODEL_FILENAME"
if [[ ! -f "$MODEL_PATH" ]]; then
  echo "missing Parakeet model: $MODEL_PATH" >&2
  echo "download manually:" >&2
  echo "  curl -L -o '$MODEL_PATH' '$MODEL_URL'" >&2
  exit 1
fi

if [[ ! -f "$SBOBINO_PARAKEET_MODELS_DIR/$NEMOTRON_STREAMING_Q4_MODEL" ]]; then
  echo "missing Parakeet NVIDIA Nemotron live model in: $SBOBINO_PARAKEET_MODELS_DIR" >&2
  echo "download manually:" >&2
  echo "  curl -L -o '$SBOBINO_PARAKEET_MODELS_DIR/$NEMOTRON_STREAMING_Q4_MODEL' '$NEMOTRON_STREAMING_Q4_URL'" >&2
  exit 1
fi

need_cmd cargo
need_cmd ffmpeg

RUN_DIR=$(mktemp -d "${TMPDIR:-/tmp}/sbobino-parakeet-smoke.XXXXXX")
cleanup() {
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT

WAV_AUDIO="$RUN_DIR/parakeet-smoke.wav"
asr_resolve_source "$ROOT_DIR"
asr_prepare_wav "$ASR_SOURCE_PATH" "$WAV_AUDIO"
WAV_AUDIO="$ASR_NORMALIZED_WAV"
DURATION_SECONDS=$(asr_audio_duration_seconds "$WAV_AUDIO")
asr_print_source_report "$DURATION_SECONDS"

export SBOBINO_PARAKEET_MODEL="$MODEL_FILENAME"
export SBOBINO_PARAKEET_AUDIO="$WAV_AUDIO"

cd "$ROOT_DIR"

echo "running adapter smoke with parakeet-cli=$SBOBINO_PARAKEET_CLI model=$MODEL_FILENAME"
cargo test -p sbobino-infrastructure \
  --test parakeet_cpp_engine_tests \
  parakeet_cpp_real_smoke \
  -- --ignored --nocapture

echo "running service smoke with parakeet-cli=$SBOBINO_PARAKEET_CLI model=$MODEL_FILENAME"
cargo test -p sbobino-infrastructure \
  --test parakeet_real_service_smoke_tests \
  parakeet_service_real_smoke_persists_metadata \
  -- --ignored --nocapture
