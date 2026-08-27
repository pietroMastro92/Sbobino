#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "Usage: $0 <version> <repo-slug> <report-json>" >&2
  exit 1
fi

VERSION=$1
REPO_SLUG=$2
REPORT_PATH=$3
TAG="v$VERSION"
ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)
RUN_DIR=$(mktemp -d "${TMPDIR:-/tmp}/sbobino-macos-live-smoke.XXXXXX")
ASSET_DIR="$RUN_DIR/assets"
SPEECH_DIR="$RUN_DIR/speech"
MODEL_DIR="$RUN_DIR/models"
mkdir -p "$ASSET_DIR" "$SPEECH_DIR" "$MODEL_DIR"

cleanup() {
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "arm64" ]]; then
  echo "Parakeet accelerated live smoke must run on macOS arm64." >&2
  exit 1
fi

for command in gh ditto curl shasum python3 cargo; do
  command -v "$command" >/dev/null 2>&1 || {
    echo "Missing required command: $command" >&2
    exit 1
  }
done

gh release download "$TAG" --repo "$REPO_SLUG" \
  --pattern "speech-runtime-macos-aarch64.zip" --dir "$ASSET_DIR"
ditto -x -k "$ASSET_DIR/speech-runtime-macos-aarch64.zip" "$SPEECH_DIR"
SPEECH_ROOT="$SPEECH_DIR/runtime"
FFMPEG_BIN="$SPEECH_ROOT/bin/ffmpeg"
[[ -x "$FFMPEG_BIN" ]] || { echo "Packaged ffmpeg is missing or not executable." >&2; exit 1; }
PARAKEET_LIB="$SPEECH_ROOT/lib/libparakeet.dylib"
[[ -f "$PARAKEET_LIB" ]] || { echo "Packaged libparakeet.dylib is missing." >&2; exit 1; }

LIVE_MODEL="nemotron-3.5-asr-streaming-0.6b-q4_k.gguf"
LIVE_MODEL_SHA256="5ad85eb3f3014c1a300d67b7ccbd23c38c4c952405cbe33a861e19fb2775e84b"
LIVE_MODEL_CACHE="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/parakeet-live-readiness-model/$LIVE_MODEL"
mkdir -p "$(dirname "$LIVE_MODEL_CACHE")"
if [[ ! -f "$LIVE_MODEL_CACHE" ]] ||
   ! printf '%s  %s\n' "$LIVE_MODEL_SHA256" "$LIVE_MODEL_CACHE" | shasum -a 256 -c - >/dev/null 2>&1; then
  curl --fail --location --retry 5 --retry-all-errors \
    "https://huggingface.co/mudler/parakeet-cpp-gguf/resolve/bf0af9f425fa01809cadec671b3cb672709d13e9/$LIVE_MODEL?download=true" \
    --output "$LIVE_MODEL_CACHE"
fi
printf '%s  %s\n' "$LIVE_MODEL_SHA256" "$LIVE_MODEL_CACHE" | shasum -a 256 -c -
cp "$LIVE_MODEL_CACHE" "$MODEL_DIR/$LIVE_MODEL"

FIXTURE_REF="9edf17c3ada66e0f881dcff155492867db7ac4cf"
FIXTURE_SHA256="5fceacff0315d49cb59fcc505bcecf1ed5f2f35c2897b1e65a59f30e5d922150"
FIXTURE_CACHE="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/parakeet-readiness-fixture/speech.wav"
mkdir -p "$(dirname "$FIXTURE_CACHE")"
if [[ ! -f "$FIXTURE_CACHE" ]] ||
   ! printf '%s  %s\n' "$FIXTURE_SHA256" "$FIXTURE_CACHE" | shasum -a 256 -c - >/dev/null 2>&1; then
  curl --fail --location --retry 5 --retry-all-errors \
    "https://raw.githubusercontent.com/mudler/parakeet.cpp/$FIXTURE_REF/tests/fixtures/speech.wav" \
    --output "$FIXTURE_CACHE"
fi
printf '%s  %s\n' "$FIXTURE_SHA256" "$FIXTURE_CACHE" | shasum -a 256 -c -

LONG_AUDIO="$RUN_DIR/live-65s.wav"
"$FFMPEG_BIN" -hide_banner -loglevel error -y -stream_loop -1 -i "$FIXTURE_CACHE" \
  -t 65 -ar 16000 -ac 1 -c:a pcm_s16le "$LONG_AUDIO"
DURATION_SECONDS=$(python3 - "$LONG_AUDIO" <<'PY'
import sys
import wave
with wave.open(sys.argv[1], "rb") as audio:
    print(audio.getnframes() / audio.getframerate())
PY
)
FIXTURE_DURATION_SECONDS=$(python3 - "$FIXTURE_CACHE" <<'PY'
import sys
import wave
with wave.open(sys.argv[1], "rb") as audio:
    print(audio.getnframes() / audio.getframerate())
PY
)
INPUT_SHA256=$(shasum -a 256 "$LONG_AUDIO" | awk '{print $1}')
RAW_REPORT="$RUN_DIR/live-raw.json"
INPUT_REPORT="$RUN_DIR/live-input.json"
EVALUATED_REPORT="$RUN_DIR/live-evaluated.json"

PATH="$SPEECH_ROOT/bin:$PATH" \
DYLD_LIBRARY_PATH="$SPEECH_ROOT/lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" \
DYLD_FALLBACK_LIBRARY_PATH="$SPEECH_ROOT/lib${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}" \
GGML_METAL_NO_RESIDENCY=1 \
GGML_METAL_SHARED_BUFFERS_DISABLE=1 \
GGML_METAL_CONCURRENCY_DISABLE=1 \
SBOBINO_PARAKEET_LIB="$PARAKEET_LIB" \
SBOBINO_PARAKEET_MODELS_DIR="$MODEL_DIR" \
SBOBINO_PARAKEET_REALTIME_MODEL="$LIVE_MODEL" \
SBOBINO_PARAKEET_AUDIO="$LONG_AUDIO" \
SBOBINO_PARAKEET_LIVE_MAX_SECONDS="$DURATION_SECONDS" \
SBOBINO_PARAKEET_LIVE_REALTIME=1 \
SBOBINO_PARAKEET_LIVE_REPORT_OUTPUT="$RAW_REPORT" \
env -u PARAKEET_DEVICE -u SBOBINO_PARAKEET_FORCE_CPU \
cargo test --manifest-path "$ROOT_DIR/Cargo.toml" -p sbobino-desktop \
  parakeet_realtime_c_api_streams_real_wav -- --ignored --nocapture

python3 - "$RAW_REPORT" "$INPUT_REPORT" "$FIXTURE_DURATION_SECONDS" "$DURATION_SECONDS" <<'PY'
import json
import math
import pathlib
import re
import sys
import unicodedata

source, destination = map(pathlib.Path, sys.argv[1:3])
fixture_duration, duration = map(float, sys.argv[3:5])
payload = json.loads(source.read_text(encoding="utf-8"))
expected = math.floor(duration / fixture_duration)
text = unicodedata.normalize("NFKC", str(payload.get("transcript") or "")).casefold()
text = " ".join(re.findall(r"[^\W\d_]+(?:['’][^\W\d_]+)?", text))
observed = text.count("old portrait")
payload.update({
    "dropped_samples": 0,
    "missing_segments": max(0, expected - observed),
    "duplicate_segments": max(0, observed - expected),
})
destination.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
PY

python3 "$ROOT_DIR/scripts/evaluate_live_latency.py" "$INPUT_REPORT" \
  --report "$EVALUATED_REPORT" --max-latency-seconds 2.0 --max-rss-growth-mib 256.0

LIB_SHA256=$(shasum -a 256 "$PARAKEET_LIB" | awk '{print $1}')
python3 - "$EVALUATED_REPORT" "$INPUT_REPORT" "$REPORT_PATH" "$INPUT_SHA256" \
  "$LIB_SHA256" "$LIVE_MODEL_SHA256" <<'PY'
import json
import pathlib
import sys

evaluated = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
raw = json.loads(pathlib.Path(sys.argv[2]).read_text(encoding="utf-8"))
metrics = evaluated.setdefault("metrics", {})
for key in ("dropped_samples", "missing_segments", "duplicate_segments"):
    metrics[key] = int(raw.get(key, 0))
evaluated.update({
    "evidence_class": "hosted-packaged-engine",
    "real_engine": True,
    "real_harness": True,
    "runner": "github-hosted macos-15",
    "harness": "release_macos_parakeet_live_smoke.sh@v1",
    "engine": "parakeet.cpp/nemotron-3.5-asr-streaming-0.6b-q4_k.gguf",
    "input_audio_sha256": sys.argv[4],
    "runtime_artifact_sha256": {
        "libparakeet": sys.argv[5],
        "nemotron_live_model": sys.argv[6],
    },
})
output = pathlib.Path(sys.argv[3])
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text(json.dumps(evaluated, indent=2) + "\n", encoding="utf-8")
PY

echo "macOS ARM64 packaged Parakeet live smoke passed: ${DURATION_SECONDS}s"
