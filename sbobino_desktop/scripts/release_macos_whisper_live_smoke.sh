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

EXPECT_PREFLIGHT_REJECTION=${SBOBINO_WHISPER_EXPECT_PREFLIGHT_REJECTION:-0}
ALLOW_PREFLIGHT_REJECTION=${SBOBINO_WHISPER_ALLOW_PREFLIGHT_REJECTION:-0}
case "$EXPECT_PREFLIGHT_REJECTION:$ALLOW_PREFLIGHT_REJECTION" in
  0:0|0:1|1:0) ;;
  1:1) echo "Expected and allowed preflight rejection modes are mutually exclusive." >&2; exit 1 ;;
  *) echo "Whisper preflight rejection controls must be 0 or 1." >&2; exit 1 ;;
esac
case "$EXPECT_PREFLIGHT_REJECTION" in
  0|1) ;;
  *) echo "SBOBINO_WHISPER_EXPECT_PREFLIGHT_REJECTION must be 0 or 1." >&2; exit 1 ;;
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
WHISPER_STRINGS="$RUN_DIR/whisper-stream.strings"
strings "$WHISPER_BIN" > "$WHISPER_STRINGS"
grep -Fq "SBOBINO_WHISPER_REPLAY_WAV" "$WHISPER_STRINGS" || {
  echo "Packaged whisper-stream has no deterministic WAV replay hook." >&2
  exit 1
}
grep -Fq "SBOBINO_WHISPER_LIVE_METRIC" "$WHISPER_STRINGS" || {
  echo "Packaged whisper-stream has no live telemetry hook." >&2
  exit 1
}

MODEL_MANIFEST="$ROOT_DIR/crates/domain/src/whisper_live_model.json"
[[ -f "$MODEL_MANIFEST" ]] || {
  echo "Whisper live model manifest is missing: $MODEL_MANIFEST" >&2
  exit 1
}
IFS=$'\t' read -r MODEL MODEL_URL MODEL_SHA256 ENCODER_DIR ENCODER_ARCHIVE ENCODER_URL ENCODER_SHA256 < <(
  python3 - "$MODEL_MANIFEST" <<'PY'
import json
import pathlib
import re
import sys

manifest = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
if manifest.get("schema_version") != 1:
    raise SystemExit("unsupported Whisper live model manifest schema")
if manifest.get("model") != "tiny" or manifest.get("filename") != "ggml-tiny-q8_0.bin":
    raise SystemExit("Whisper live smoke requires the certified Tiny model manifest")
url = str(manifest.get("url") or "")
digest = str(manifest.get("sha256") or "").lower()
if "/resolve/main/" in url or not re.fullmatch(r"[0-9a-f]{64}", digest):
    raise SystemExit("Whisper live model manifest must use an immutable URL and SHA-256")
encoder = manifest.get("coreml_encoder") or {}
encoder_url = str(encoder.get("url") or "")
encoder_digest = str(encoder.get("sha256") or "").lower()
if (
    encoder.get("directory") != "ggml-tiny-encoder.mlmodelc"
    or encoder.get("archive_filename") != "ggml-tiny-encoder.mlmodelc.zip"
    or "/resolve/main/" in encoder_url
    or not re.fullmatch(r"[0-9a-f]{64}", encoder_digest)
):
    raise SystemExit("Whisper live Core ML encoder must use an immutable URL and SHA-256")
print(
    f"{manifest['filename']}\t{url}\t{digest}\t"
    f"{encoder['directory']}\t{encoder['archive_filename']}\t"
    f"{encoder_url}\t{encoder_digest}"
)
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

EXPECT_COREML=0
if [[ "$ARCH" == "arm64" ]]; then
  EXPECT_COREML=1
  ENCODER_CACHE="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/whisper-live-readiness-model/$ENCODER_ARCHIVE"
  if [[ ! -f "$ENCODER_CACHE" ]] ||
     ! printf '%s  %s\n' "$ENCODER_SHA256" "$ENCODER_CACHE" | shasum -a 256 -c - >/dev/null 2>&1; then
    curl --fail --location --retry 5 --retry-all-errors \
      "$ENCODER_URL" \
      --output "$ENCODER_CACHE"
  fi
  printf '%s  %s\n' "$ENCODER_SHA256" "$ENCODER_CACHE" | shasum -a 256 -c -
  ditto -x -k "$ENCODER_CACHE" "$MODEL_DIR"
  [[ -f "$MODEL_DIR/$ENCODER_DIR/model.mil" ]] || {
    echo "Pinned Whisper live Core ML encoder is incomplete." >&2
    exit 1
  }
  [[ -f "$MODEL_DIR/$ENCODER_DIR/weights/weight.bin" ]] || {
    echo "Pinned Whisper live Core ML encoder weights are missing." >&2
    exit 1
  }
fi

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
PREFLIGHT_ARG=""
if [[ "$EXPECT_PREFLIGHT_REJECTION" == "1" ]]; then
  PREFLIGHT_ARG=--expect-preflight-rejection
elif [[ "$ALLOW_PREFLIGHT_REJECTION" == "1" ]]; then
  PREFLIGHT_ARG=--allow-preflight-rejection
fi

set +e
PATH="$SPEECH_ROOT/bin:$PATH" \
DYLD_LIBRARY_PATH="$SPEECH_ROOT/lib${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}" \
DYLD_FALLBACK_LIBRARY_PATH="$SPEECH_ROOT/lib${DYLD_FALLBACK_LIBRARY_PATH:+:$DYLD_FALLBACK_LIBRARY_PATH}" \
SBOBINO_WHISPER_EXPECT_COREML="$EXPECT_COREML" \
python3 "$ROOT_DIR/scripts/run_whisper_live_replay.py" \
  --binary "$WHISPER_BIN" \
  --model "$MODEL_DIR/$MODEL" \
  --audio "$LONG_AUDIO" \
  --fixture "$FIXTURE_CACHE" \
  --report "$RAW_REPORT" \
  --run-dir "$RUN_DIR" \
  --device "$DEVICE" \
  --platform "$PLATFORM" \
  ${PREFLIGHT_ARG:+"$PREFLIGHT_ARG"}
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
SBOBINO_WHISPER_EXPECT_COREML="$EXPECT_COREML" \
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

RAW_LIVE_MODE=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["live_mode"])' "$RAW_REPORT")
if [[ "$RAW_LIVE_MODE" != "realtime" ]]; then
  python3 - "$RAW_REPORT" "$EVALUATED_REPORT" <<'PY'
import json
import pathlib
import sys

raw = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
evaluated = {
    "schema_version": 1,
    "status": raw.get("status"),
    "engine": raw.get("engine"),
    "platform": raw.get("platform"),
    "metrics": {
        "dropped_samples": raw.get("dropped_samples", 0),
        "missing_segments": raw.get("missing_segments", 0),
        "duplicate_segments": raw.get("duplicate_segments", 0),
    },
    "failures": list(raw.get("failures") or []),
}
pathlib.Path(sys.argv[2]).write_text(json.dumps(evaluated, indent=2) + "\n", encoding="utf-8")
PY
  EVALUATE_STATUS=0
else
  set +e
  python3 "$ROOT_DIR/scripts/evaluate_live_latency.py" "$RAW_REPORT" \
    --report "$EVALUATED_REPORT" --max-latency-seconds 2.0 --max-rss-growth-mib 256.0
  EVALUATE_STATUS=$?
  set -e
fi

LIB_SHA256=$(shasum -a 256 "$WHISPER_BIN" | awk '{print $1}')
COMMIT_SHA=$(git rev-parse HEAD)
python3 - "$EVALUATED_REPORT" "$RAW_REPORT" "$REPORT_PATH" "$INPUT_SHA256" "$MODEL_SHA256" "$LIB_SHA256" "$DEVICE" "$VERSION" "$TAG" "$RECOVERY_REPORT" "$LIVE_DURATION_SECONDS" "$COMMIT_SHA" "$REPO_SLUG" "$ENCODER_SHA256" "$EXPECT_COREML" <<'PY'
import json
import os
import pathlib
import sys

evaluated = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
raw = json.loads(pathlib.Path(sys.argv[2]).read_text(encoding="utf-8"))
recovery = json.loads(pathlib.Path(sys.argv[10]).read_text(encoding="utf-8"))
runtime_hashes = {
    "whisper-stream": sys.argv[6],
    "whisper_model": sys.argv[5],
}
if sys.argv[15] == "1":
    runtime_hashes["whisper_coreml_encoder"] = sys.argv[14]
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
    "runner": os.environ.get("SBOBINO_LIVE_RUNNER", "github-hosted macos-15"),
    "harness": "release_macos_whisper_live_smoke.sh@v1",
    "engine": "whisper.cpp/whisper-stream",
    "compute_device": sys.argv[7],
    "duration_seconds": float(sys.argv[11]),
    "requested_duration_seconds": raw.get("requested_duration_seconds"),
    "captured_duration_seconds": raw.get("captured_duration_seconds"),
    "live_mode": raw.get("live_mode"),
    "realtime_capable": raw.get("live_mode") == "realtime",
    "preflight_rejected": raw.get("preflight_rejected", False),
    "preflight": raw.get("preflight"),
    "profile": raw.get("profile"),
    "commit_sha": sys.argv[12],
    "repo_slug": sys.argv[13],
    "input_audio_sha256": sys.argv[4],
    "runtime_artifact_sha256": runtime_hashes,
    "backlog_recovery": {
        "status": recovery.get("status"),
        "live_mode": recovery.get("live_mode"),
        "backlog_recovery_expected": recovery.get("backlog_recovery_expected"),
        "preflight_rejection_expected": recovery.get("preflight_rejection_expected"),
        "preflight_rejected": recovery.get("preflight_rejected"),
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
  echo "Packaged Whisper live smoke failed (run=$RUN_STATUS recovery=$RECOVERY_STATUS evaluate=$EVALUATE_STATUS); report written to $REPORT_PATH" >&2
  # The subprocesses intentionally capture their native output so the proof is
  # deterministic.  On failure, emit a compact diagnostic summary as well so a
  # hosted job remains debuggable even when its proof upload is the next step.
  python3 - "$RAW_REPORT" "$RECOVERY_REPORT" "$EVALUATED_REPORT" <<'PY' >&2 || true
import json
import pathlib
import sys

for label, raw_path in (("run", sys.argv[1]), ("recovery", sys.argv[2]), ("evaluation", sys.argv[3])):
    path = pathlib.Path(raw_path)
    if not path.is_file():
        print(f"{label}: report missing at {path}")
        continue
    try:
        report = json.loads(path.read_text(encoding="utf-8"))
    except Exception as error:
        print(f"{label}: report unreadable: {error}")
        continue
    summary = {
        "status": report.get("status"),
        "live_mode": report.get("live_mode"),
        "preflight_rejected": report.get("preflight_rejected"),
        "preflight": report.get("preflight"),
        "failures": report.get("failures", []),
        "captured_audio_frames": report.get("captured_audio_frames"),
        "saved_audio_frames": report.get("saved_audio_frames"),
        "samples": len(report.get("samples") or []),
    }
    print(f"{label}: {json.dumps(summary, sort_keys=True)}")
PY
  exit 1
fi

echo "macOS packaged Whisper live smoke passed (device=$DEVICE)."
