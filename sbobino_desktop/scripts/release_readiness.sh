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

smoke_test_pyannote_runtime_asset() {
  local pyannote_runtime_zip=$1
  local pyannote_stage
  pyannote_stage=$(mktemp -d)
  trap 'rm -rf "$pyannote_stage"' RETURN

  /usr/bin/ditto -x -k "$pyannote_runtime_zip" "$pyannote_stage"

python3 - <<'PY' "$pyannote_stage"
import os
import pathlib
import subprocess
import sys

root = pathlib.Path(sys.argv[1]) / "python"
python = root / "bin" / "python3"
host_prefixes = ("/opt/homebrew", "/usr/local")
if not python.is_file():
    raise SystemExit(f"Pyannote runtime asset is missing expected binary: {python}")

stdlib_dirs = [
    path
    for path in (root / "lib").glob("python3.*")
    if path.is_dir()
    and (path / "encodings").is_dir()
    and (path / "types.py").is_file()
    and (path / "traceback.py").is_file()
    and (path / "collections" / "__init__.py").is_file()
    and (path / "collections" / "abc.py").is_file()
]
if not stdlib_dirs:
    raise SystemExit("Pyannote runtime asset is missing an embedded Python standard library.")

pyvenv_cfg = root / "pyvenv.cfg"
if pyvenv_cfg.exists():
    body = pyvenv_cfg.read_text(encoding="utf-8")
    for forbidden in (*host_prefixes, "/var/folders/"):
        if forbidden in body:
            raise SystemExit(
                f"Pyannote runtime asset still contains machine-specific pyvenv.cfg paths ({forbidden})."
            )


def parse_otool_dependencies(output: str) -> list[str]:
    refs: list[str] = []
    for line in output.splitlines()[1:]:
        stripped = line.strip()
        if not stripped:
            continue
        ref = stripped.split(" (", 1)[0].split(" ", 1)[0].strip()
        if ref:
            refs.append(ref)
    return refs


def parse_otool_rpaths(output: str) -> list[str]:
    refs: list[str] = []
    in_rpath = False
    for line in output.splitlines():
        stripped = line.strip()
        if stripped == "cmd LC_RPATH":
            in_rpath = True
            continue
        if in_rpath and stripped.startswith("path "):
            refs.append(stripped.split("path ", 1)[1].split(" (offset ", 1)[0])
            in_rpath = False
    return refs


def runtime_binary_roots() -> list[pathlib.Path]:
    roots: list[pathlib.Path] = []
    for stdlib_dir in stdlib_dirs:
        for relative in ("lib-dynload", "site-packages"):
            candidate = stdlib_dir / relative
            if candidate.is_dir():
                roots.append(candidate)
    embedded_dir = root / "lib" / "embedded-dylibs"
    if embedded_dir.is_dir():
        roots.append(embedded_dir)
    return roots


def iter_runtime_native_binaries() -> list[pathlib.Path]:
    binaries: list[pathlib.Path] = []
    seen: set[pathlib.Path] = set()
    for search_root in runtime_binary_roots():
        for binary in sorted(search_root.rglob("*")):
            if not binary.is_file() or binary.suffix not in {".so", ".dylib"}:
                continue
            resolved = binary.resolve()
            if resolved in seen:
                continue
            seen.add(resolved)
            binaries.append(resolved)
    return binaries


for binary in iter_runtime_native_binaries():
    deps = subprocess.run(
        ["/usr/bin/otool", "-L", str(binary)],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    ).stdout
    for dep in parse_otool_dependencies(deps):
        if dep.startswith(host_prefixes):
            raise SystemExit(
                f"Pyannote runtime asset still links a native module against a host path: {binary} -> {dep}"
            )

    rpath_output = subprocess.run(
        ["/usr/bin/otool", "-l", str(binary)],
        check=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    ).stdout
    for rpath in parse_otool_rpaths(rpath_output):
        if rpath.startswith(host_prefixes):
            raise SystemExit(
                f"Pyannote runtime asset still exposes a host LC_RPATH: {binary} -> {rpath}"
            )

torchcodec_dir = stdlib_dirs[0] / "site-packages" / "torchcodec"
if torchcodec_dir.is_dir():
    binaries = sorted(
        list(torchcodec_dir.glob("libtorchcodec_core*.dylib"))
        + list(torchcodec_dir.glob("libtorchcodec_custom_ops*.dylib"))
        + list(torchcodec_dir.glob("libtorchcodec_pybind_ops*.so"))
    )
    for binary in binaries:
        deps = subprocess.run(
            ["/usr/bin/otool", "-L", str(binary)],
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        ).stdout
        for dep in parse_otool_dependencies(deps):
            if dep.startswith(host_prefixes):
                raise SystemExit(
                    f"Pyannote runtime asset still links torchcodec against a host path: {binary} -> {dep}"
                )

        rpath_output = subprocess.run(
            ["/usr/bin/otool", "-l", str(binary)],
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        ).stdout
        for rpath in parse_otool_rpaths(rpath_output):
            if rpath.startswith(host_prefixes):
                raise SystemExit(
                    f"Pyannote runtime asset still exposes a host LC_RPATH: {binary} -> {rpath}"
                )

    for name in (
        "libavutil.60.dylib",
        "libavcodec.62.dylib",
        "libavformat.62.dylib",
        "libavdevice.62.dylib",
        "libavfilter.11.dylib",
        "libswscale.9.dylib",
        "libswresample.6.dylib",
    ):
        if not (torchcodec_dir / ".dylibs" / name).exists():
            raise SystemExit(
                f"Pyannote runtime asset is missing bundled TorchCodec FFmpeg library: {torchcodec_dir / '.dylibs' / name}"
            )

env = {
    "PATH": "/usr/bin:/bin",
    "PYTHONHOME": str(root),
    "PYTHONNOUSERSITE": "1",
}
for key in (
    "PYTHONPATH",
    "PYTHONEXECUTABLE",
    "__PYVENV_LAUNCHER__",
    "VIRTUAL_ENV",
    "CONDA_PREFIX",
    "CONDA_DEFAULT_ENV",
):
    env.pop(key, None)

probe = subprocess.run(
    [
        str(python),
        "-c",
        "import collections.abc,ctypes,csv,encodings,traceback,types; import torch; from pyannote.audio import Pipeline; print('ok')",
    ],
    stdout=subprocess.PIPE,
    stderr=subprocess.STDOUT,
    text=True,
    timeout=180,
    env=env,
)
if probe.returncode != 0:
    preview = "\n".join(probe.stdout.splitlines()[:40])
    raise SystemExit(
        "Pyannote runtime asset failed an import probe while validating the extracted asset.\n"
        f"{preview}"
    )
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
"$SCRIPTS_DIR/setup_bundled_pyannote.sh" --force

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
smoke_test_pyannote_runtime_asset "$PYANNOTE_RUNTIME_ZIP"

export SBOBINO_LOCAL_RELEASE_ASSETS_DIR="$ASSET_DIR"

pushd "$DESKTOP_DIR" >/dev/null
npm test -- initialSetup provisioningUi appBootstrap updateState
popd >/dev/null

pushd "$ROOT_DIR" >/dev/null
cargo test -p sbobino-infrastructure runtime_health_reports_compatibility_level_mismatch_as_repair_required
cargo test -p sbobino-infrastructure runtime_health_backfills_legacy_pyannote_manifest_compat_level
cargo test -p sbobino-infrastructure load_settings_migrates_legacy_pyannote_runtime_directory_when_current_is_missing
cargo test -p sbobino-infrastructure load_settings_recovers_pyannote_runtime_from_interrupted_backup_swap
cargo test -p sbobino-infrastructure runtime_health_reports_install_incomplete_when_python_stdlib_is_missing
cargo test -p sbobino-infrastructure runtime_health_self_heals_missing_manifest_and_status_from_bundled_override
cargo test -p sbobino-infrastructure runnable_ffmpeg_probe_accepts_slow_cold_start
cargo test -p sbobino-infrastructure managed_runtime_accepts_slow_whisper_cli_cold_start
cargo test -p sbobino-infrastructure managed_runtime_accepts_slow_whisper_stream_cold_start
cargo test -p sbobino-infrastructure public_runtime_health_requires_managed_runtime_binaries
cargo test -p sbobino-infrastructure public_runtime_health_ignores_configured_host_binaries
cargo test -p sbobino-infrastructure runtime_health_trusts_cached_ready_pyannote_status_on_warm_start
cargo test -p sbobino-desktop plan_pyannote_background_action_skips_missing_install_when_diarization_disabled
cargo test -p sbobino-desktop plan_pyannote_background_action_installs_missing_runtime_when_diarization_enabled
cargo test -p sbobino-desktop plan_pyannote_background_action_reports_real_compat_mismatch_as_asset_migration
cargo test -p sbobino-desktop plan_pyannote_background_action_self_heals_stale_incomplete_status
cargo test -p sbobino-desktop plan_pyannote_background_action_requests_manifest_only_migration_on_patch_update
cargo test -p sbobino-desktop plan_pyannote_background_action_requests_asset_migration_on_checksum_mismatch
cargo test -p sbobino-desktop install_pyannote_archive_extracts_expected_root
cargo test -p sbobino-desktop promote_staged_pyannote_runtime_swaps_only_after_staging_finishes
cargo test -p sbobino-desktop pyannote_runtime_swap_rolls_back_previous_install_on_failure
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

  if ! /usr/libexec/PlistBuddy -c "Print :NSMicrophoneUsageDescription" "$APP_PATH/Contents/Info.plist" >/dev/null 2>&1; then
    echo "Bundled app is missing NSMicrophoneUsageDescription in Info.plist." >&2
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
