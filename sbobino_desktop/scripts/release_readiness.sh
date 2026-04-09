#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "Usage: $0 <version> [app-path]" >&2
  exit 1
fi

VERSION=$1
APP_PATH=${2:-}
RELEASE_PROFILE=${SBOBINO_RELEASE_PROFILE:-public}
MACOS_RUNTIME_DEPLOYMENT_TARGET=${SBOBINO_MACOS_RUNTIME_DEPLOYMENT_TARGET:-13.0}

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
DESKTOP_DIR="$ROOT_DIR/apps/desktop"
SCRIPTS_DIR="$ROOT_DIR/scripts"
TEMP_DIR=$(mktemp -d)
ASSET_DIR="$TEMP_DIR/release-assets"
PYANNOTE_RUNTIME_ZIP="$ASSET_DIR/pyannote-runtime-macos-aarch64.zip"
PYANNOTE_MODEL_ZIP="$ASSET_DIR/pyannote-model-community-1.zip"
RUNTIME_ZIP="$ASSET_DIR/speech-runtime-macos-aarch64.zip"

cleanup() {
  rm -rf "$TEMP_DIR"
}
trap cleanup EXIT

resolve_bundled_pyannote_root() {
  local app_path=$1
  local candidates=(
    "$app_path/Contents/Resources/pyannote"
    "$app_path/Contents/Resources/resources/pyannote"
  )

  local candidate
  for candidate in "${candidates[@]}"; do
    if [[ -d "$candidate/python" && -d "$candidate/model" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done

  return 1
}

assert_bundle_pyannote_profile() {
  local app_path=$1
  local bundled_pyannote_root=""

  bundled_pyannote_root=$(resolve_bundled_pyannote_root "$app_path" || true)

  if [[ "$RELEASE_PROFILE" == "public" ]]; then
    if [[ -n "$bundled_pyannote_root" ]]; then
      echo "Public release bundle must not embed pyannote resources, but found '$bundled_pyannote_root'." >&2
      exit 1
    fi
    assert_bundle_contains_no_local_user_data "$app_path"
    return 0
  fi

  if [[ -z "$bundled_pyannote_root" ]]; then
    echo "Standalone-dev release bundle is missing bundled pyannote resources." >&2
    exit 1
  fi

  assert_bundle_contains_no_local_user_data "$app_path" "$bundled_pyannote_root"
}

assert_bundle_contains_no_local_user_data() {
  local app_path=$1
  local bundled_pyannote_root=${2:-}
  local hits=()

  local file_find_root=("$app_path/Contents")
  local dir_find_root=("$app_path/Contents")
  if [[ -n "$bundled_pyannote_root" ]]; then
    file_find_root=(
      "$app_path/Contents"
      "(" -path "$bundled_pyannote_root" -o -path "$bundled_pyannote_root/*" ")" -prune -o
    )
    dir_find_root=(
      "$app_path/Contents"
      "(" -path "$bundled_pyannote_root" -o -path "$bundled_pyannote_root/*" ")" -prune -o
    )
  fi

  while IFS= read -r match; do
    [[ -n "$match" ]] && hits+=("$match")
  done < <(
    find "${file_find_root[@]}" \
      \( \
        -iname 'settings.json' -o \
        -iname 'setup-report.json' -o \
        -iname 'artifacts.db' -o \
        -iname 'artifacts.db-*' -o \
        -iname '*.sqlite' -o \
        -iname '*.sqlite3' -o \
        -iname '*.wav' -o \
        -iname '*.mp3' -o \
        -iname '*.m4a' -o \
        -iname '*.aac' -o \
        -iname '*.ogg' -o \
        -iname '*.opus' -o \
        -iname '*.flac' -o \
        -iname '*.srt' -o \
        -iname '*.vtt' -o \
        -iname '*.docx' -o \
        -iname '*.pdf' \
      \) -print
  )

  while IFS= read -r match; do
    [[ -n "$match" ]] && hits+=("$match")
  done < <(
    find "${dir_find_root[@]}" -type d \
      \( \
        -iname 'audio-vault' -o \
        -iname 'artifacts' -o \
        -iname 'backups' -o \
        -iname 'deleted' \
      \) -print
  )

  if (( ${#hits[@]} > 0 )); then
    echo "Release bundle contains local user data or user-generated artifacts:" >&2
    printf ' - %s\n' "${hits[@]}" >&2
    exit 1
  fi
}

smoke_test_runtime_asset() {
  local runtime_zip=$1
  local runtime_stage
  runtime_stage=$(mktemp -d)
  trap 'rm -rf "$runtime_stage"' RETURN

  unzip -q "$runtime_zip" -d "$runtime_stage"

python3 - <<'PY' "$runtime_stage"
import os
import subprocess
import sys

root = sys.argv[1]
bin_dir = os.path.join(root, "runtime", "bin")
lib_dir = os.path.join(root, "runtime", "lib")
env = os.environ.copy()
env["DYLD_LIBRARY_PATH"] = lib_dir
env["DYLD_FALLBACK_LIBRARY_PATH"] = lib_dir
env["PATH"] = f"{bin_dir}:/usr/bin:/bin"

def run_probe(binary: str, args: list[str], timeout: int, allow_timeout: bool) -> None:
    candidate = os.path.join(bin_dir, binary)
    try:
        result = subprocess.run(
            [candidate, *args],
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=timeout,
            env=env,
        )
    except subprocess.TimeoutExpired as exc:
        if allow_timeout:
            return
        raise SystemExit(
            f"{binary} did not respond within {timeout}s while validating extracted runtime asset."
        ) from exc
    if result.returncode != 0:
        preview = "\n".join(result.stdout.splitlines()[:20])
        raise SystemExit(
            f"{binary} exited with code {result.returncode} while validating extracted runtime asset.\n{preview}"
        )

def generate_test_wav(path: str) -> None:
    import math
    import wave

    sample_rate = 16000
    duration_seconds = 1.0
    total_samples = int(sample_rate * duration_seconds)
    frequency = 440.0
    amplitude = 0.35

    with wave.open(path, "wb") as wav_file:
        wav_file.setnchannels(1)
        wav_file.setsampwidth(2)
        wav_file.setframerate(sample_rate)

        frames = bytearray()
        for index in range(total_samples):
            value = int(
                max(-1.0, min(1.0, amplitude * math.sin(2.0 * math.pi * frequency * index / sample_rate)))
                * 32767
            )
            frames.extend(int(value).to_bytes(2, byteorder="little", signed=True))
        wav_file.writeframes(bytes(frames))

for binary, args, timeout, cold_timeout in (
    ("ffmpeg", ["-version"], 60, 45),
    ("whisper-cli", ["--help"], 30, 15),
    ("whisper-stream", ["--help"], 30, 15),
):
    candidate = os.path.join(bin_dir, binary)
    if not os.path.isfile(candidate):
        raise SystemExit(f"Runtime asset is missing expected binary: {candidate}")

    run_probe(binary, args, cold_timeout, True)
    run_probe(binary, args, timeout, False)

input_wav = os.path.join(root, "runtime", "smoke-input.wav")
output_wav = os.path.join(root, "runtime", "smoke-output.wav")
generate_test_wav(input_wav)

ffmpeg_result = subprocess.run(
    [
        os.path.join(bin_dir, "ffmpeg"),
        "-y",
        "-i",
        input_wav,
        "-ar",
        "16000",
        "-ac",
        "1",
        "-c:a",
        "pcm_s16le",
        output_wav,
    ],
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
    text=True,
    timeout=120,
    env=env,
)
if ffmpeg_result.returncode != 0:
    preview = "\n".join(ffmpeg_result.stdout.splitlines()[:40])
    raise SystemExit(
        "ffmpeg failed a real conversion while validating the extracted runtime asset.\n"
        f"{preview}"
    )
if not os.path.isfile(output_wav) or os.path.getsize(output_wav) == 0:
    raise SystemExit("ffmpeg completed but did not produce a valid smoke-test wav output.")
PY
}

assert_runtime_asset_portability() {
  local runtime_zip=$1
  local runtime_stage
  runtime_stage=$(mktemp -d)
  trap 'rm -rf "$runtime_stage"' RETURN

  unzip -q "$runtime_zip" -d "$runtime_stage"

  local binary
  for binary in ffmpeg whisper-cli whisper-stream; do
    local candidate="$runtime_stage/runtime/bin/$binary"
    if [[ ! -x "$candidate" ]]; then
      echo "Runtime asset is missing expected executable: $candidate" >&2
      exit 1
    fi

    local minos
    minos=$(otool -l "$candidate" | awk '
      /LC_BUILD_VERSION/ { flag=1; next }
      flag && $1 == "minos" { print $2; exit }
      /LC_VERSION_MIN_MACOSX/ { legacy=1; next }
      legacy && $1 == "version" { print $2; exit }
    ')
    if [[ -z "$minos" ]]; then
      echo "Unable to determine deployment target for $candidate" >&2
      exit 1
    fi

    if ! python3 - "$MACOS_RUNTIME_DEPLOYMENT_TARGET" "$minos" <<'PY'
import sys

def parse(value: str) -> tuple[int, ...]:
    return tuple(int(part) for part in value.split("."))

supported = parse(sys.argv[1])
actual = parse(sys.argv[2])
if actual > supported:
    raise SystemExit(1)
PY
    then
      echo "$binary in the runtime asset targets macOS $minos, newer than the supported $MACOS_RUNTIME_DEPLOYMENT_TARGET floor." >&2
      exit 1
    fi

    local bad_refs
    bad_refs=$(otool -L "$candidate" | tail -n +2 | awk '{print $1}' | grep -E '^(/opt/homebrew|/usr/local)' || true)
    if [[ -n "$bad_refs" ]]; then
      echo "$binary still links against non-portable host paths:" >&2
      printf ' - %s\n' $bad_refs >&2
      exit 1
    fi
  done
}

mkdir -p "$ASSET_DIR"

"$SCRIPTS_DIR/check_release_versions.sh" "$VERSION"

"$SCRIPTS_DIR/package_macos_runtime_asset.sh" "$RUNTIME_ZIP"
"$SCRIPTS_DIR/package_pyannote_asset.sh" \
  "$DESKTOP_DIR/src-tauri/resources/pyannote/python/aarch64-apple-darwin" \
  python \
  "$PYANNOTE_RUNTIME_ZIP"
"$SCRIPTS_DIR/package_pyannote_asset.sh" \
  "$DESKTOP_DIR/src-tauri/resources/pyannote/model" \
  model \
  "$PYANNOTE_MODEL_ZIP"
"$SCRIPTS_DIR/generate_release_manifests.sh" "$VERSION" "$ASSET_DIR"
assert_runtime_asset_portability "$RUNTIME_ZIP"
smoke_test_runtime_asset "$RUNTIME_ZIP"

export SBOBINO_LOCAL_RELEASE_ASSETS_DIR="$ASSET_DIR"

pushd "$DESKTOP_DIR" >/dev/null
npm test -- initialSetup provisioningUi appBootstrap
popd >/dev/null

pushd "$ROOT_DIR" >/dev/null
cargo test -p sbobino-infrastructure runtime_health_reports_version_mismatch_as_repair_required
cargo test -p sbobino-infrastructure runtime_health_reports_install_incomplete_when_python_stdlib_is_missing
cargo test -p sbobino-infrastructure runtime_health_self_heals_missing_manifest_and_status_from_bundled_override
cargo test -p sbobino-infrastructure runnable_ffmpeg_probe_accepts_slow_cold_start
cargo test -p sbobino-infrastructure managed_runtime_accepts_slow_whisper_cli_cold_start
cargo test -p sbobino-infrastructure managed_runtime_accepts_slow_whisper_stream_cold_start
cargo test -p sbobino-infrastructure public_runtime_health_requires_managed_runtime_binaries
cargo test -p sbobino-infrastructure public_runtime_health_ignores_configured_host_binaries
cargo test -p sbobino-desktop install_pyannote_archive_extracts_expected_root
cargo test -p sbobino-desktop verify_file_sha256_rejects_wrong_checksum
cargo test -p sbobino-desktop validate_setup_manifest_rejects_mismatched_release_tag
cargo test -p sbobino-desktop validate_manifest_asset_descriptor_rejects_checksum_mismatch
popd >/dev/null

if [[ -n "$APP_PATH" ]]; then
  if [[ ! -d "$APP_PATH" ]]; then
    echo "Built app not found at '$APP_PATH'." >&2
    exit 1
  fi

  APP_EXECUTABLE_NAME=$(/usr/libexec/PlistBuddy -c "Print :CFBundleExecutable" "$APP_PATH/Contents/Info.plist")
  APP_EXEC="$APP_PATH/Contents/MacOS/$APP_EXECUTABLE_NAME"
  if [[ ! -x "$APP_EXEC" ]]; then
    echo "App executable missing at '$APP_EXEC'." >&2
    exit 1
  fi

  for binary in whisper-cli whisper-stream ffmpeg; do
    if [[ ! -x "$APP_PATH/Contents/MacOS/$binary" ]]; then
      echo "Bundled binary missing: $APP_PATH/Contents/MacOS/$binary" >&2
      exit 1
    fi
  done

  assert_bundle_pyannote_profile "$APP_PATH"
fi

echo "Build readiness checks passed for version $VERSION"
