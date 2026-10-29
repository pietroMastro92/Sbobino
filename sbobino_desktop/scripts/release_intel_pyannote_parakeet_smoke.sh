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
mkdir -p "$ASSET_DIR" "$SPEECH_DIR" "$PYANNOTE_DIR" "$MODEL_DIR" "$LOG_DIR"

cleanup() {
  local status=$?
  if python3 - "$RUN_DIR" "$LOG_DIR" "$status" <<'PYRAW'
import pathlib
import shutil
import sys

source, target = map(pathlib.Path, sys.argv[1:3])
target.mkdir(parents=True, exist_ok=True)
(target / "exit-code.txt").write_text(sys.argv[3] + "\n", encoding="utf-8")
for path in source.rglob("*"):
    if path.relative_to(source).parts[0] in {"assets", "speech", "pyannote", "models", "parakeet-models"}:
        continue
    if path.is_file() and path.suffix in {".json", ".log", ".stdout", ".stderr", ".txt"}:
        destination = target / path.relative_to(source)
        destination.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, destination)
PYRAW
  then
    rm -rf "$RUN_DIR"
  else
    echo "Could not preserve smoke diagnostics; retaining $RUN_DIR" >&2
  fi
  return "$status"
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

RELEASE_API_JSON="$RUN_DIR/release-api.json"
RELEASE_ASSET_VERIFICATION_JSON="$RUN_DIR/release-assets-verification.json"
gh api "repos/$REPO_SLUG/releases/tags/$TAG" > "$RELEASE_API_JSON"
python3 - "$RELEASE_API_JSON" "$ASSET_DIR" "$RELEASE_ASSET_VERIFICATION_JSON" \
  "speech-runtime-macos-x86_64.zip" \
  "pyannote-runtime-macos-x86_64.zip" \
  "pyannote-model-community-1.zip" <<'PY'
import hashlib
import json
import pathlib
import re
import sys

release_path = pathlib.Path(sys.argv[1])
asset_dir = pathlib.Path(sys.argv[2])
verification_path = pathlib.Path(sys.argv[3])
asset_names = sys.argv[4:]
release = json.loads(release_path.read_text(encoding="utf-8"))
assets = {
    str(asset.get("name")): asset
    for asset in release.get("assets", [])
    if isinstance(asset, dict) and asset.get("name")
}
verified = {}
for name in asset_names:
    path = asset_dir / name
    if not path.is_file():
        raise SystemExit(f"downloaded release asset is missing: {name}")
    asset = assets.get(name)
    if asset is None:
        raise SystemExit(f"release API asset is missing: {name}")
    digest = str(asset.get("digest") or "")
    if not re.fullmatch(r"sha256:[0-9a-fA-F]{64}", digest):
        raise SystemExit(f"release API has no usable SHA-256 digest for {name}")
    expected = digest.split(":", 1)[1].lower()
    hasher = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            hasher.update(chunk)
    actual = hasher.hexdigest()
    if actual != expected:
        raise SystemExit(
            f"release asset digest mismatch for {name}: expected {expected}, got {actual}"
        )
    verified[name] = {
        "github_api_digest": digest,
        "sha256": actual,
        "size_bytes": path.stat().st_size,
    }

verification_path.write_text(json.dumps(verified, indent=2) + "\n", encoding="utf-8")
PY

ditto -x -k "$ASSET_DIR/speech-runtime-macos-x86_64.zip" "$SPEECH_DIR"
ditto -x -k "$ASSET_DIR/pyannote-runtime-macos-x86_64.zip" "$PYANNOTE_DIR"
ditto -x -k "$ASSET_DIR/pyannote-model-community-1.zip" "$MODEL_DIR"

SPEECH_ROOT="$SPEECH_DIR/runtime"
FFMPEG_BIN="$SPEECH_ROOT/bin/ffmpeg"
[[ -x "$FFMPEG_BIN" ]] || { echo "Packaged ffmpeg is missing or not executable." >&2; exit 1; }
# The service smoke constructs its transcoder from this explicit path.  Put
# the packaged bin directory first as well so every helper invoked by the
# harness resolves the same self-contained FFmpeg runtime on a clean runner.
export PATH="$SPEECH_ROOT/bin:$PATH"
PYTHON_ROOT="$PYANNOTE_DIR/python"
PYANNOTE_MODEL_ROOT="$MODEL_DIR/model"
PYANNOTE_HELPER="$ROOT_DIR/scripts/pyannote_diarize.py"
[[ -f "$PYANNOTE_HELPER" ]] || { echo "Pyannote inference helper is missing: $PYANNOTE_HELPER" >&2; exit 1; }
PYANNOTE_HELPER_SHA256=$(shasum -a 256 "$PYANNOTE_HELPER" | awk '{print $1}')
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
export SBOBINO_WHISPER_FFMPEG="$FFMPEG_BIN"
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

PYANNOTE_INFERENCE_JSON="$RUN_DIR/pyannote-inference.json"
PYANNOTE_INFERENCE_STDERR="$RUN_DIR/pyannote-inference.stderr"
PYANNOTE_INFERENCE_SUMMARY="$RUN_DIR/pyannote-inference-summary.json"
if PATH="$PYTHON_ROOT/bin:$SPEECH_ROOT/bin:/usr/bin:/bin" \
  PYTHONHOME="$PYTHON_ROOT" \
  PYTHONPATH="$PYTHON_ROOT/lib/$PYTHON_VERSION_DIR:$PYTHON_ROOT/lib/$PYTHON_VERSION_DIR/lib-dynload:$PYTHON_ROOT/lib/$PYTHON_VERSION_DIR/site-packages" \
  PYTHONNOUSERSITE=1 \
  HF_HUB_OFFLINE=1 \
  TRANSFORMERS_OFFLINE=1 \
  "$PYTHON_BIN" "$PYANNOTE_HELPER" \
  --audio-path "$LONG_AUDIO" \
  --model-path "$PYANNOTE_MODEL_ROOT" \
  --device cpu \
  >"$PYANNOTE_INFERENCE_JSON" \
  2>"$PYANNOTE_INFERENCE_STDERR"; then
  :
else
  status=$?
  echo "Pyannote inference helper failed with status $status." >&2
  cat "$PYANNOTE_INFERENCE_STDERR" >&2 || true
  exit "$status"
fi

python3 - "$PYANNOTE_INFERENCE_JSON" "$DURATION_SECONDS" "$PYANNOTE_INFERENCE_SUMMARY" <<'PY'
import json
import math
import pathlib
import sys

result_path = pathlib.Path(sys.argv[1])
duration = float(sys.argv[2])
summary_path = pathlib.Path(sys.argv[3])
if not math.isfinite(duration) or duration <= 0.0:
    raise SystemExit(f"invalid audio duration: {duration}")
payload = json.loads(result_path.read_text(encoding="utf-8"))
turns = payload.get("speakers") if isinstance(payload, dict) else None
if not isinstance(turns, list) or not turns:
    raise SystemExit("Pyannote inference returned no speaker turns")

speaker_ids = set()
starts = []
ends = []
for index, turn in enumerate(turns):
    if not isinstance(turn, dict):
        raise SystemExit(f"Pyannote speaker turn {index} is not an object")
    speaker_id = str(turn.get("speaker_id") or "").strip()
    if not speaker_id:
        raise SystemExit(f"Pyannote speaker turn {index} has no speaker_id")
    start = turn.get("start_seconds")
    end = turn.get("end_seconds")
    if (
        isinstance(start, bool)
        or isinstance(end, bool)
        or not isinstance(start, (int, float))
        or not isinstance(end, (int, float))
        or not math.isfinite(float(start))
        or not math.isfinite(float(end))
    ):
        raise SystemExit(f"Pyannote speaker turn {index} has non-finite bounds")
    start = float(start)
    end = float(end)
    if start < 0.0 or end <= start or end > duration:
        raise SystemExit(
            f"Pyannote speaker turn {index} is outside audio duration: "
            f"{start}..{end} (audio={duration})"
        )
    speaker_ids.add(speaker_id)
    starts.append(start)
    ends.append(end)

summary = {
    "status": "passed",
    "audio_duration_seconds": duration,
    "speaker_turns": len(turns),
    "unique_speakers": sorted(speaker_ids),
    "min_start_seconds": min(starts),
    "max_end_seconds": max(ends),
}
summary_path.write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
PY

PYANNOTE_INFERENCE_SHA256=$(shasum -a 256 "$PYANNOTE_INFERENCE_JSON" | awk '{print $1}')
PARAKEET_CLI_SHA256=$(shasum -a 256 "$SPEECH_ROOT/bin/parakeet-cli" | awk '{print $1}')
PARAKEET_WORKER_SHA256=$(shasum -a 256 "$SPEECH_ROOT/bin/parakeet-batch-json" | awk '{print $1}')
PARAKEET_LIB="$SPEECH_ROOT/lib/libparakeet.dylib"
[[ -f "$PARAKEET_LIB" ]] || { echo "Packaged libparakeet.dylib is missing." >&2; exit 1; }
PARAKEET_LIB_SHA256=$(shasum -a 256 "$PARAKEET_LIB" | awk '{print $1}')
COMMIT_SHA=$(git rev-parse HEAD)
python3 - "$REPORT_PATH" "$VERSION" "$TAG" "$DURATION_SECONDS" \
  "$RUN_DIR/parakeet-asr-report.json" \
  "$LONG_AUDIO_SHA256" "$PARAKEET_CLI_SHA256" "$PARAKEET_WORKER_SHA256" \
  "$PARAKEET_LIB_SHA256" "$PARAKEET_MODEL_SHA256" "$COMMIT_SHA" "$REPO_SLUG" \
  "$PYANNOTE_INFERENCE_SUMMARY" "$RELEASE_ASSET_VERIFICATION_JSON" \
  "$PYANNOTE_HELPER_SHA256" "$PYANNOTE_INFERENCE_SHA256" <<'PY'
import json
import pathlib
import sys
from datetime import datetime, timezone

report_path = pathlib.Path(sys.argv[1])
asr_report = json.loads(pathlib.Path(sys.argv[5]).read_text(encoding="utf-8"))
pyannote_summary = json.loads(pathlib.Path(sys.argv[13]).read_text(encoding="utf-8"))
release_assets = json.loads(pathlib.Path(sys.argv[14]).read_text(encoding="utf-8"))
audio_sha256 = sys.argv[6]
runtime_hashes = {
    "parakeet_cli": sys.argv[7],
    "parakeet_batch_json": sys.argv[8],
    "libparakeet": sys.argv[9],
    "tdt_model": sys.argv[10],
}
common_evidence = {
    "version": sys.argv[2],
    "release_tag": sys.argv[3],
    "evidence_class": "hosted-packaged-engine",
    "real_engine": True,
    "real_harness": True,
    "runner": "github-hosted macos-15-intel",
    "harness": "release_intel_pyannote_parakeet_smoke.sh@v3",
    "input_audio_sha256": audio_sha256,
    "runtime_artifact_sha256": runtime_hashes,
    "commit_sha": sys.argv[11],
    "repo_slug": sys.argv[12],
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
    "commit_sha": sys.argv[11],
    "repo_slug": sys.argv[12],
    "parakeet_duration_seconds": float(sys.argv[4]),
    "parakeet_compute_device": "cpu",
    "parakeet_language": "auto",
    "parakeet_live_cpu_compatibility": "not_certified_realtime_on_intel",
    "pyannote_deep_smoke": True,
    "pyannote_inference": {
        **pyannote_summary,
        "device": "cpu",
        "helper": "scripts/pyannote_diarize.py",
        "helper_sha256": sys.argv[15],
        "result_json": "intel-pyannote-parakeet-smoke-logs/pyannote-inference.json",
        "result_json_sha256": sys.argv[16],
        "stderr_log": "intel-pyannote-parakeet-smoke-logs/pyannote-inference.stderr",
    },
    "release_asset_sha256": release_assets,
    "asr_reference": asr_report,
    "logs": [
        "intel-pyannote-parakeet-smoke-logs/parakeet.log",
        "intel-pyannote-parakeet-smoke-logs/pyannote.log",
        "intel-pyannote-parakeet-smoke-logs/pyannote-inference.json",
        "intel-pyannote-parakeet-smoke-logs/pyannote-inference.stderr",
        "intel-pyannote-parakeet-smoke-logs/pyannote-inference-summary.json",
        "intel-pyannote-parakeet-smoke-logs/release-assets-verification.json",
    ],
}
report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
PY

echo "Intel Pyannote/Parakeet smoke passed: duration=${DURATION_SECONDS}s compute=cpu language=auto"
