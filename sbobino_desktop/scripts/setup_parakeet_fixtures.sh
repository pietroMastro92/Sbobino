#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
source "$ROOT_DIR/scripts/lib/asr_samples.sh"

PARAKEET_CPP_REF=${SBOBINO_RUNTIME_PARAKEET_CPP_REF:-9edf17c3ada66e0f881dcff155492867db7ac4cf}
FIXTURES_DIR=${SBOBINO_PARAKEET_FIXTURES_DIR:-$(asr_default_parakeet_fixture_dir)}
BASE_URL="https://raw.githubusercontent.com/mudler/parakeet.cpp/$PARAKEET_CPP_REF/tests/fixtures"

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    asr_fail "missing required command: $1"
  fi
}

download_fixture() {
  local filename=$1
  local output="$FIXTURES_DIR/$filename"
  if [[ -f "$output" ]]; then
    echo "fixture already present: $output"
    return 0
  fi
  curl --fail --location --show-error --output "$output" "$BASE_URL/$filename"
  echo "downloaded fixture: $output"
}

need_cmd curl

mkdir -p "$FIXTURES_DIR"
download_fixture speech.wav
download_fixture clip.wav

echo
echo "Parakeet fixtures ready."
echo "parakeet_cpp_ref=$PARAKEET_CPP_REF"
echo "fixtures_dir=$FIXTURES_DIR"
echo
echo "Run benchmark-like comparison with:"
echo "  SBOBINO_ASR_SAMPLE=parakeet_fixture SBOBINO_PARAKEET_FIXTURE=speech.wav scripts/compare_asr_engines_real.sh"
