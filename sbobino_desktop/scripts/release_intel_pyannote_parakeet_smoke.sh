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
REPORT_PARENT=$(dirname "$REPORT_PATH")
mkdir -p "$REPORT_PARENT"
REPORT_DIR=$(cd "$REPORT_PARENT" && pwd -P)
LOG_DIR="$REPORT_DIR/intel-pyannote-parakeet-smoke-logs"
RUN_DIR=$(mktemp -d "${TMPDIR:-/tmp}/sbobino-intel-pyannote-smoke.XXXXXX")
ASSET_DIR="$RUN_DIR/assets"
SPEECH_DIR="$RUN_DIR/speech"
PYANNOTE_DIR="$RUN_DIR/pyannote"
MODEL_DIR="$RUN_DIR/models"
mkdir -p "$ASSET_DIR" "$SPEECH_DIR" "$PYANNOTE_DIR" "$MODEL_DIR"

cleanup() {
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT

if [[ "$(uname -s)" != "Darwin" || "$(uname -m)" != "x86_64" ]]; then
  echo "Intel Pyannote/Parakeet smoke must run on a macOS x86_64 host." >&2
  exit 1
fi

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "Missing required command: $1" >&2
    exit 1
  }
}
for command in gh ditto curl shasum python3; do
  need_cmd "$command"
done

gh release download "$TAG" \
  --repo "$REPO_SLUG" \
  --pattern "speech-runtime-macos-x86_64.zip" \
  --pattern "pyannote-runtime-macos-x86_64.zip" \
  --pattern "pyannote-model-community-1.zip" \
  --dir "$ASSET_DIR"

ditto -x -k "$ASSET_DIR/speech-runtime-macos-x86_64.zip" "$SPEECH_DIR"
ditto -x -k "$ASSET_DIR/pyannote-runtime-macos-x86_64.zip" "$PYANNOTE_DIR"
ditto -x -k "$ASSET_DIR/pyannote-model-community-1.zip" "$MODEL_DIR"

SPEECH_ROOT="$SPEECH_DIR/runtime"
FFMPEG_BIN="$SPEECH_ROOT/bin/ffmpeg"
[[ -x "$FFMPEG_BIN" ]] || { echo "Packaged ffmpeg is missing or not executable." >&2; exit 1; }
PYTHON_ROOT="$PYANNOTE_DIR/python"
PYANNOTE_MODEL_ROOT="$MODEL_DIR/model"
PARAKEET_MODEL="tdt-0.6b-v3-q4_k.gguf"
PARAKEET_MODELS="$RUN_DIR/parakeet-models"
mkdir -p "$PARAKEET_MODELS"

PARAKEET_MODEL_SHA256="993d73feb4206dadda865ab25bd64b50c48dc4d013c3bf6126a721f28b1d5ee8"
CACHE_MODEL_ROOT="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/parakeet-readiness-model"
CACHE_MODEL_PATH="$CACHE_MODEL_ROOT/$PARAKEET_MODEL"
mkdir -p "$CACHE_MODEL_ROOT"
if [[ ! -f "$CACHE_MODEL_PATH" ]] ||
   ! printf '%s  %s\n' "$PARAKEET_MODEL_SHA256" "$CACHE_MODEL_PATH" | shasum -a 256 -c - >/dev/null 2>&1; then
  curl --fail --location --retry 5 --retry-all-errors \
    "https://huggingface.co/mudler/parakeet-cpp-gguf/resolve/bf0af9f425fa01809cadec671b3cb672709d13e9/$PARAKEET_MODEL?download=true" \
    --output "$CACHE_MODEL_PATH"
fi
printf '%s  %s\n' "$PARAKEET_MODEL_SHA256" "$CACHE_MODEL_PATH" | shasum -a 256 -c -
cp "$CACHE_MODEL_PATH" "$PARAKEET_MODELS/$PARAKEET_MODEL"

# Use the pinned real English LibriSpeech utterance from the Parakeet C++ test
# fixtures rather than a synthetic tone.  Cache it separately from the model
# so the hosted Intel job remains repeatable without silently changing the
# speech input when the upstream repository moves.
PARAKEET_FIXTURE_REF="9edf17c3ada66e0f881dcff155492867db7ac4cf"
PARAKEET_FIXTURE_SHA256="5fceacff0315d49cb59fcc505bcecf1ed5f2f35c2897b1e65a59f30e5d922150"
PARAKEET_FIXTURE_NAME="speech.wav"
CACHE_FIXTURE_ROOT="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/parakeet-readiness-fixture"
CACHE_FIXTURE_PATH="$CACHE_FIXTURE_ROOT/$PARAKEET_FIXTURE_NAME"
mkdir -p "$CACHE_FIXTURE_ROOT"
if [[ ! -f "$CACHE_FIXTURE_PATH" ]] ||
   ! printf '%s  %s\n' "$PARAKEET_FIXTURE_SHA256" "$CACHE_FIXTURE_PATH" | shasum -a 256 -c - >/dev/null 2>&1; then
  curl --fail --location --retry 5 --retry-all-errors \
    "https://raw.githubusercontent.com/mudler/parakeet.cpp/$PARAKEET_FIXTURE_REF/tests/fixtures/$PARAKEET_FIXTURE_NAME" \
    --output "$CACHE_FIXTURE_PATH"
fi
printf '%s  %s\n' "$PARAKEET_FIXTURE_SHA256" "$CACHE_FIXTURE_PATH" | shasum -a 256 -c -

# Loop the utterance without an artificial encoder gap and make the duration
# deterministic, longer than the production 45s batch-worker threshold.
LONG_AUDIO="$RUN_DIR/parakeet-long.wav"
"$FFMPEG_BIN" -hide_banner -loglevel error -y -stream_loop -1 \
  -i "$CACHE_FIXTURE_PATH" \
  -t 65 -map 0:a:0 -ar 16000 -ac 1 -c:a pcm_s16le "$LONG_AUDIO"
DURATION_SECONDS=$(python3 - "$LONG_AUDIO" <<'PY'
import sys
import wave
with wave.open(sys.argv[1], "rb") as audio:
    print(audio.getnframes() / audio.getframerate())
PY
)
python3 - "$DURATION_SECONDS" <<'PY'
import sys
if float(sys.argv[1]) <= 60.0:
    raise SystemExit(f"long Parakeet smoke audio is only {sys.argv[1]} seconds")
PY
LONG_AUDIO_SHA256=$(shasum -a 256 "$LONG_AUDIO" | awk '{print $1}')
FIXTURE_DURATION_SECONDS=$(python3 - "$CACHE_FIXTURE_PATH" <<'PY'
import sys
import wave
with wave.open(sys.argv[1], "rb") as audio:
    print(audio.getnframes() / audio.getframerate())
PY
)

# The Intel hosted runner has no Metal path; keep the contract explicit in
# the environment and report it as CPU/automatic-language smoke.
export SBOBINO_PARAKEET_CLI="$SPEECH_ROOT/bin/parakeet-cli"
export SBOBINO_PARAKEET_MODELS_DIR="$PARAKEET_MODELS"
export SBOBINO_PARAKEET_MODEL="$PARAKEET_MODEL"
export SBOBINO_PARAKEET_AUDIO="$LONG_AUDIO"
export SBOBINO_PARAKEET_REQUIRE_WORKER_RSS_MONITOR=1
export SBOBINO_PARAKEET_SMOKE_MODE=service
export SBOBINO_PARAKEET_SKIP_NEMOTRON=1
export SBOBINO_PARAKEET_FORCE_CPU=1
export SBOBINO_PARAKEET_LANGUAGE=auto
export SBOBINO_PARAKEET_EXPECTED_DETECTED_LANGUAGE=en
export SBOBINO_PARAKEET_EXPECTED_PROCESSING_LANGUAGE=en
export SBOBINO_ASR_SAMPLE=parakeet_fixture
export SBOBINO_PARAKEET_FIXTURE="$LONG_AUDIO"
export SBOBINO_ASR_TIMELINE_OUTPUT="$RUN_DIR/parakeet-file-timeline.json"
export GGML_METAL=0

"$ROOT_DIR/scripts/smoke_parakeet_real.sh" 2>&1 | tee "$RUN_DIR/parakeet.log"

python3 - "$RUN_DIR/parakeet-file-reference.json" "$DURATION_SECONDS" \
  "$FIXTURE_DURATION_SECONDS" "$LONG_AUDIO_SHA256" <<'PY'
import json
import math
import pathlib
import sys

output = pathlib.Path(sys.argv[1])
duration = float(sys.argv[2])
fixture_duration = float(sys.argv[3])
audio_sha256 = sys.argv[4]
text = (
    "Well, I don't wish to see it any more, observed Phebe, turning away her "
    "eyes. It is certainly very like the old portrait."
)
segments = [
    {
        "start_seconds": index * fixture_duration,
        "end_seconds": (index + 1) * fixture_duration,
        "language_code": "en",
        "text": text,
    }
    for index in range(math.floor(duration / fixture_duration))
]
payload = {
    "schema_version": 1,
    "review_status": "reviewed",
    "reference_source": "parakeet.cpp 9edf17c3 LibriSpeech 2086-149220-0033 NeMo TDT reference",
    "audio_sha256": audio_sha256,
    "duration_seconds": duration,
    "segments": segments,
}
output.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
PY

python3 "$ROOT_DIR/scripts/evaluate_asr_reference.py" \
  "$RUN_DIR/parakeet-file-reference.json" \
  "$RUN_DIR/parakeet-file-timeline.json" \
  --audio "$LONG_AUDIO" \
  --report "$RUN_DIR/parakeet-asr-report.json" \
  --max-wer 0.35 --max-cer 0.25 --max-gap-seconds 2.0 \
  --require-reviewed-reference

PYTHON_VERSION_DIR=$(find "$PYTHON_ROOT/lib" -maxdepth 1 -type d -name 'python3.*' -print -quit | xargs -I{} basename {})
[[ -n "$PYTHON_VERSION_DIR" ]] || { echo "Pyannote Python standard library is missing." >&2; exit 1; }
PYTHON_BIN="$PYTHON_ROOT/bin/python3"
PATH="$PYTHON_ROOT/bin:$SPEECH_ROOT/bin:/usr/bin:/bin" \
PYTHONHOME="$PYTHON_ROOT" \
PYTHONPATH="$PYTHON_ROOT/lib/$PYTHON_VERSION_DIR:$PYTHON_ROOT/lib/$PYTHON_VERSION_DIR/lib-dynload:$PYTHON_ROOT/lib/$PYTHON_VERSION_DIR/site-packages" \
PYTHONNOUSERSITE=1 \
HF_HUB_OFFLINE=1 \
TRANSFORMERS_OFFLINE=1 \
"$PYTHON_BIN" - "$PYANNOTE_MODEL_ROOT" <<'PY' 2>&1 | tee "$RUN_DIR/pyannote.log"
import importlib.metadata
import pathlib
import re
import sys

import torch
import torchcodec
from pyannote.audio import Pipeline

model = pathlib.Path(sys.argv[1])
if not (model / "config.yaml").is_file():
    raise SystemExit("Pyannote model config.yaml is missing")
if Pipeline.from_pretrained(str(model)) is None:
    raise SystemExit("Pyannote model deep smoke returned no pipeline")
print(f"torch_version={torch.__version__}")
print(f"torchcodec_version={importlib.metadata.version('torchcodec')}")
print("pyannote_deep_smoke=passed")
PY

PARAKEET_CLI_SHA256=$(shasum -a 256 "$SPEECH_ROOT/bin/parakeet-cli" | awk '{print $1}')
PARAKEET_WORKER_SHA256=$(shasum -a 256 "$SPEECH_ROOT/bin/parakeet-batch-json" | awk '{print $1}')
PARAKEET_LIB="$SPEECH_ROOT/lib/libparakeet.dylib"
[[ -f "$PARAKEET_LIB" ]] || { echo "Packaged libparakeet.dylib is missing." >&2; exit 1; }
PARAKEET_LIB_SHA256=$(shasum -a 256 "$PARAKEET_LIB" | awk '{print $1}')
python3 - "$REPORT_PATH" "$VERSION" "$TAG" "$DURATION_SECONDS" \
  "$RUN_DIR/parakeet-asr-report.json" \
  "$LONG_AUDIO_SHA256" "$PARAKEET_CLI_SHA256" "$PARAKEET_WORKER_SHA256" \
  "$PARAKEET_LIB_SHA256" "$PARAKEET_MODEL_SHA256" <<'PY'
import json
import pathlib
import sys
from datetime import datetime, timezone

report_path = pathlib.Path(sys.argv[1])
asr_report = json.loads(pathlib.Path(sys.argv[5]).read_text(encoding="utf-8"))
audio_sha256 = sys.argv[6]
runtime_hashes = {
    "parakeet_cli": sys.argv[7],
    "parakeet_batch_json": sys.argv[8],
    "libparakeet": sys.argv[9],
    "tdt_model": sys.argv[10],
}
common_evidence = {
    "evidence_class": "hosted-packaged-engine",
    "real_engine": True,
    "real_harness": True,
    "runner": "github-hosted macos-15-intel",
    "harness": "release_intel_pyannote_parakeet_smoke.sh@v2",
    "input_audio_sha256": audio_sha256,
    "runtime_artifact_sha256": runtime_hashes,
}
asr_report.update(common_evidence)
asr_report["engine"] = "parakeet.cpp/tdt-0.6b-v3-q4_k.gguf"
report = {
    "schema_version": 1,
    "version": sys.argv[2],
    "release_tag": sys.argv[3],
    "platform": "macos",
    "architecture": "x86_64",
    "status": "passed",
    "runner": "github-hosted macos-15-intel",
    "machine_class": "HOSTED-CLEANROOM-STANDARD",
    "tested_at_utc": datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "commit_sha": __import__("subprocess").check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
    "parakeet_duration_seconds": float(sys.argv[4]),
    "parakeet_compute_device": "cpu",
    "parakeet_language": "auto",
    "parakeet_live_cpu_compatibility": "not_certified_realtime_on_intel",
    "pyannote_deep_smoke": True,
    "asr_reference": asr_report,
    "logs": [
        "intel-pyannote-parakeet-smoke-logs/parakeet.log",
        "intel-pyannote-parakeet-smoke-logs/pyannote.log",
    ],
}
report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
PY

mkdir -p "$LOG_DIR"
cp "$RUN_DIR/parakeet.log" "$LOG_DIR/parakeet.log"
cp "$RUN_DIR/pyannote.log" "$LOG_DIR/pyannote.log"

echo "Intel Pyannote/Parakeet smoke passed: duration=${DURATION_SECONDS}s compute=cpu language=auto"
