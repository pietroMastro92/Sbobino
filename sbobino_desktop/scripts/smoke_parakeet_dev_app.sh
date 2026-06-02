#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

fail() {
  echo "error: $*" >&2
  exit 1
}

if [[ -n "${SBOBINO_TAURI_TARGET_TRIPLE:-}" ]]; then
  TARGET_TRIPLE=$SBOBINO_TAURI_TARGET_TRIPLE
else
  case "$(uname -m)" in
    arm64) TARGET_TRIPLE="aarch64-apple-darwin" ;;
    x86_64) TARGET_TRIPLE="x86_64-apple-darwin" ;;
    *) fail "unsupported macOS architecture: $(uname -m)" ;;
  esac
fi
DEV_SIDECAR="$ROOT_DIR/apps/desktop/src-tauri/binaries/parakeet-cli-$TARGET_TRIPLE"
APP_SUPPORT_DIR="$HOME/Library/Application Support/com.sbobino.desktop"

if [[ ! -x "$DEV_SIDECAR" ]]; then
  fail "missing executable dev Parakeet sidecar: $DEV_SIDECAR"
fi

export SBOBINO_PARAKEET_CLI="$DEV_SIDECAR"
export SBOBINO_PARAKEET_MODELS_DIR="${SBOBINO_PARAKEET_MODELS_DIR:-$APP_SUPPORT_DIR/parakeet-models}"
export SBOBINO_PARAKEET_MODEL="${SBOBINO_PARAKEET_MODEL:-tdt-0.6b-v3-q4_k.gguf}"
export SBOBINO_ASR_SAMPLE="${SBOBINO_ASR_SAMPLE:-artemis}"
export PATH="$APP_SUPPORT_DIR/bin:$PATH"

echo "dev_app_parakeet_cli=$SBOBINO_PARAKEET_CLI"
echo "dev_app_parakeet_models_dir=$SBOBINO_PARAKEET_MODELS_DIR"
echo "dev_app_parakeet_model=$SBOBINO_PARAKEET_MODEL"
echo "dev_app_asr_sample=$SBOBINO_ASR_SAMPLE"

"$ROOT_DIR/scripts/smoke_parakeet_real.sh"
