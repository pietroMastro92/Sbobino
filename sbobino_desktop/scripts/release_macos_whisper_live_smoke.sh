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
RUN_DIR=$(mktemp -d "${TMPDIR:-/tmp}/sbobino-macos-whisper-live.XXXXXX")
ASSET_DIR="$RUN_DIR/assets"
SPEECH_DIR="$RUN_DIR/speech"
MODEL_DIR="$RUN_DIR/models"
mkdir -p "$ASSET_DIR" "$SPEECH_DIR" "$MODEL_DIR"

cleanup() {
  rm -rf "$RUN_DIR"
}
trap cleanup EXIT

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "Packaged Whisper live smoke must run on macOS." >&2
  exit 1
fi

for command in ditto curl python3 shasum strings; do
  command -v "$command" >/dev/null 2>&1 || {
    echo "Missing required command: $command" >&2
    exit 1
  }
done

ARCH=$(uname -m)
if [[ "${SBOBINO_WHISPER_LIVE_ARCH:-}" != "" && "${SBOBINO_WHISPER_LIVE_ARCH}" != "$ARCH" ]]; then
  echo "Whisper live smoke requires ${SBOBINO_WHISPER_LIVE_ARCH}, found $ARCH." >&2
  exit 1
fi

DEVICE=${SBOBINO_WHISPER_LIVE_DEVICE:-auto}
case "$DEVICE" in
  auto|cpu) ;;
  *) echo "SBOBINO_WHISPER_LIVE_DEVICE must be auto or cpu." >&2; exit 1 ;;
esac

ASSET_ARCH=$ARCH
PLATFORM=macos-x86_64
if [[ "$ARCH" == "arm64" ]]; then
  ASSET_ARCH=aarch64
  PLATFORM=macos-arm64
fi
if [[ -n "${SBOBINO_WHISPER_LIVE_RUNTIME_ZIP:-}" ]]; then
  RUNTIME_ZIP=$SBOBINO_WHISPER_LIVE_RUNTIME_ZIP
  [[ -f "$RUNTIME_ZIP" ]] || { echo "Local speech runtime zip is missing: $RUNTIME_ZIP" >&2; exit 1; }
else
  command -v gh >/dev/null 2>&1 || { echo "Missing required command: gh" >&2; exit 1; }
  gh release download "$TAG" --repo "$REPO_SLUG" \
    --pattern "speech-runtime-macos-${ASSET_ARCH}.zip" --dir "$ASSET_DIR"
  RUNTIME_ZIP="$ASSET_DIR/speech-runtime-macos-${ASSET_ARCH}.zip"
fi
ditto -x -k "$RUNTIME_ZIP" "$SPEECH_DIR"
SPEECH_ROOT="$SPEECH_DIR/runtime"
WHISPER_BIN="$SPEECH_ROOT/bin/whisper-stream"
FFMPEG_BIN="$SPEECH_ROOT/bin/ffmpeg"
[[ -x "$WHISPER_BIN" ]] || { echo "Packaged whisper-stream is missing." >&2; exit 1; }
[[ -x "$FFMPEG_BIN" ]] || { echo "Packaged ffmpeg is missing." >&2; exit 1; }

# A --help probe alone is not release evidence: require the hooks compiled
# into the exact runtime binary that is about to be exercised.
strings "$WHISPER_BIN" | grep -Fq "SBOBINO_WHISPER_REPLAY_WAV" || {
  echo "Packaged whisper-stream has no deterministic WAV replay hook." >&2
  exit 1
}
strings "$WHISPER_BIN" | grep -Fq "SBOBINO_WHISPER_LIVE_METRIC" || {
  echo "Packaged whisper-stream has no live telemetry hook." >&2
  exit 1
}

MODEL_MANIFEST="$ROOT_DIR/crates/domain/src/whisper_live_model.json"
[[ -f "$MODEL_MANIFEST" ]] || {
  echo "Whisper live model manifest is missing: $MODEL_MANIFEST" >&2
  exit 1
}
IFS=$'\t' read -r MODEL MODEL_URL MODEL_SHA256 < <(
  python3 - "$MODEL_MANIFEST" <<'PY'
import json
import pathlib
import re
import sys

manifest = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
if manifest.get("schema_version") != 1:
    raise SystemExit("unsupported Whisper live model manifest schema")
if manifest.get("model") != "base" or manifest.get("filename") != "ggml-base.bin":
    raise SystemExit("Whisper live smoke requires the certified Base model manifest")
url = str(manifest.get("url") or "")
digest = str(manifest.get("sha256") or "").lower()
if "/resolve/main/" in url or not re.fullmatch(r"[0-9a-f]{64}", digest):
    raise SystemExit("Whisper live model manifest must use an immutable URL and SHA-256")
print(f"{manifest['filename']}\t{url}\t{digest}")
PY
)
MODEL_CACHE="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/whisper-live-readiness-model/$MODEL"
mkdir -p "$(dirname "$MODEL_CACHE")"
if [[ ! -f "$MODEL_CACHE" ]] ||
   ! printf '%s  %s\n' "$MODEL_SHA256" "$MODEL_CACHE" | shasum -a 256 -c - >/dev/null 2>&1; then
  curl --fail --location --retry 5 --retry-all-errors \
    "$MODEL_URL" \
    --output "$MODEL_CACHE"
fi
printf '%s  %s\n' "$MODEL_SHA256" "$MODEL_CACHE" | shasum -a 256 -c -
cp "$MODEL_CACHE" "$MODEL_DIR/$MODEL"

FIXTURE_REF="9edf17c3ada66e0f881dcff155492867db7ac4cf"
FIXTURE_SHA256="5fceacff0315d49cb59fcc505bcecf1ed5f2f35c2897b1e65a59f30e5d922150"
FIXTURE_CACHE="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/whisper-live-readiness-fixture/speech.wav"
mkdir -p "$(dirname "$FIXTURE_CACHE")"
if [[ ! -f "$FIXTURE_CACHE" ]] ||
   ! printf '%s  %s\n' "$FIXTURE_SHA256" "$FIXTURE_CACHE" | shasum -a 256 -c - >/dev/null 2>&1; then
  curl --fail --location --retry 5 --retry-all-errors \
    "https://raw.githubusercontent.com/mudler/parakeet.cpp/$FIXTURE_REF/tests/fixtures/speech.wav" \
    --output "$FIXTURE_CACHE"
fi
printf '%s  %s\n' "$FIXTURE_SHA256" "$FIXTURE_CACHE" | shasum -a 256 -c -

# The release contract requires a sustained 15-minute session. A shorter
# duration may be requested only by focused local developer tests.
LIVE_DURATION_SECONDS=${SBOBINO_WHISPER_LIVE_DURATION_SECONDS:-900}
if (( LIVE_DURATION_SECONDS < 65 )); then
  echo "Whisper live smoke duration must be at least 65 seconds." >&2
  exit 1
fi
LONG_AUDIO="$RUN_DIR/live-${LIVE_DURATION_SECONDS}s.wav"
"$FFMPEG_BIN" -hide_banner -loglevel error -y -stream_loop -1 -i "$FIXTURE_CACHE" \
  -t "$LIVE_DURATION_SECONDS" -ar 16000 -ac 1 -c:a pcm_s16le "$LONG_AUDIO"

INPUT_SHA256=$(shasum -a 256 "$LONG_AUDIO" | awk '{print $1}')
RAW_REPORT="$RUN_DIR/live-input.json"
EVALUATED_REPORT="$RUN_DIR/live-evaluated.json"

set +e
PATH="$SPEECH_ROOT/bin:$PATH" \
DYLD_LIBRARY_PATH="$SPEECH_ROOT/lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" \
DYLD_FALLBACK_LIBRARY_PATH="$SPEECH_ROOT/lib${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}" \
python3 "$ROOT_DIR/scripts/run_whisper_live_replay.py" \
  --binary "$WHISPER_BIN" \
  --model "$MODEL_DIR/$MODEL" \
  --audio "$LONG_AUDIO" \
  --fixture "$FIXTURE_CACHE" \
  --report "$RAW_REPORT" \
  --run-dir "$RUN_DIR" \
  --device "$DEVICE" \
  --platform "$PLATFORM"
RUN_STATUS=$?
set -e

RECOVERY_DIR="$RUN_DIR/backlog-recovery"
RECOVERY_REPORT="$RUN_DIR/backlog-recovery.json"
mkdir -p "$RECOVERY_DIR"
set +e
PATH="$SPEECH_ROOT/bin:$PATH" \
DYLD_LIBRARY_PATH="$SPEECH_ROOT/lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" \
DYLD_FALLBACK_LIBRARY_PATH="$SPEECH_ROOT/lib${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}" \
SBOBINO_WHISPER_TEST_INFERENCE_DELAY_MS=5000 \
python3 "$ROOT_DIR/scripts/run_whisper_live_replay.py" \
  --binary "$WHISPER_BIN" \
  --model "$MODEL_DIR/$MODEL" \
  --audio "$LONG_AUDIO" \
  --fixture "$FIXTURE_CACHE" \
  --report "$RECOVERY_REPORT" \
  --run-dir "$RECOVERY_DIR" \
  --device "$DEVICE" \
  --platform "$PLATFORM" \
  --expect-backlog-recovery
RECOVERY_STATUS=$?
set -e

set +e
python3 "$ROOT_DIR/scripts/evaluate_live_latency.py" "$RAW_REPORT" \
  --report "$EVALUATED_REPORT" --max-latency-seconds 2.0 --max-rss-growth-mib 256.0
EVALUATE_STATUS=$?
set -e

LIB_SHA256=$(shasum -a 256 "$WHISPER_BIN" | awk '{print $1}')
python3 - "$EVALUATED_REPORT" "$RAW_REPORT" "$REPORT_PATH" "$INPUT_SHA256" "$MODEL_SHA256" "$LIB_SHA256" "$DEVICE" "$VERSION" "$TAG" "$RECOVERY_REPORT" "$LIVE_DURATION_SECONDS" <<'PY'
import json
import os
import pathlib
import sys

evaluated = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
raw = json.loads(pathlib.Path(sys.argv[2]).read_text(encoding="utf-8"))
recovery = json.loads(pathlib.Path(sys.argv[10]).read_text(encoding="utf-8"))
evaluated["metrics"]["dropped_samples"] = raw.get("dropped_samples", 0)
evaluated["metrics"]["missing_segments"] = raw.get("missing_segments", 0)
evaluated["metrics"]["duplicate_segments"] = raw.get("duplicate_segments", 0)
if raw.get("failures"):
    evaluated["failures"].extend(raw["failures"])
    evaluated["status"] = "failed"
if recovery.get("failures") or recovery.get("status") != "passed":
    evaluated["failures"].extend(
        f"backlog recovery: {failure}" for failure in recovery.get("failures", [])
    )
    evaluated["status"] = "failed"
evaluated.update({
    "version": sys.argv[8],
    "release_tag": sys.argv[9],
    "evidence_class": "hosted-packaged-engine",
    "real_engine": True,
    "real_harness": True,
    "runner": os.environ.get("SBOBINO_LIVE_RUNNER", "github-hosted macos-14"),
    "harness": "release_macos_whisper_live_smoke.sh@v1",
    "engine": "whisper.cpp/whisper-stream",
    "compute_device": sys.argv[7],
    "duration_seconds": float(sys.argv[11]),
    "input_audio_sha256": sys.argv[4],
    "runtime_artifact_sha256": {
        "whisper-stream": sys.argv[6],
        "whisper_model": sys.argv[5],
    },
    "backlog_recovery": {
        "status": recovery.get("status"),
        "captured_audio_frames": recovery.get("captured_audio_frames"),
        "saved_audio_frames": recovery.get("saved_audio_frames"),
        "dropped_samples": recovery.get("dropped_samples"),
        "backlog_reaction_seconds": recovery.get("backlog_reaction_seconds"),
    },
})
output = pathlib.Path(sys.argv[3])
output.parent.mkdir(parents=True, exist_ok=True)
output.write_text(json.dumps(evaluated, indent=2) + "\n", encoding="utf-8")
PY

if [[ "$RUN_STATUS" -ne 0 || "$RECOVERY_STATUS" -ne 0 || "$EVALUATE_STATUS" -ne 0 ]]; then
  echo "Packaged Whisper live smoke failed; report written to $REPORT_PATH" >&2
  exit 1
fi

echo "macOS packaged Whisper live smoke passed (device=$DEVICE)."
