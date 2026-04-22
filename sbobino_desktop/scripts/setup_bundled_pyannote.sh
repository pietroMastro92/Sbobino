#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: setup_bundled_pyannote.sh [--force] [--python-version VERSION] [--model-repo URL]

Downloads a local offline pyannote model and creates a bundled Python runtime
under apps/desktop/src-tauri/resources/pyannote so Tauri builds can ship a
standalone diarization-capable app without post-install provisioning.

Options:
  --force                 Rebuild runtime and re-download the model.
  --python-version X.Y    Python version for the bundled runtime. Default: 3.11
  --model-repo URL        Git/LFS repository containing an offline pyannote
                          pipeline. Default:
                          https://huggingface.co/pyannote-community/speaker-diarization-community-1
EOF
}

FORCE=0
PYTHON_VERSION=3.11
MODEL_REPO="https://huggingface.co/pyannote-community/speaker-diarization-community-1"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --force)
      FORCE=1
      shift
      ;;
    --python-version)
      PYTHON_VERSION=${2:-}
      shift 2
      ;;
    --model-repo)
      MODEL_REPO=${2:-}
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 1
      ;;
  esac
done

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

need_cmd git
need_cmd git-lfs
need_cmd rsync
need_cmd install_name_tool
need_cmd codesign
need_cmd otool
need_cmd curl
need_cmd shasum
need_cmd python3

TORCHCODEC_FFMPEG_BASE_URL="https://pytorch.s3.amazonaws.com/torchcodec/ffmpeg/2025-03-14/macos_arm64"
TORCHCODEC_FFMPEG_VERSION="8.0"
TORCHCODEC_FFMPEG_SHA256="beb936b76f25d2621228a12cdb67c9ae3d1eff7aa713ef8d1167ebf0c25bd5ec"
TORCHCODEC_FFMPEG_LIB_PATTERNS=(
  "libavutil*.dylib"
  "libavcodec*.dylib"
  "libavformat*.dylib"
  "libavdevice*.dylib"
  "libavfilter*.dylib"
  "libswscale*.dylib"
  "libswresample*.dylib"
)

download_source_archive() {
  local url=$1
  local output=$2
  curl --fail --location --silent --show-error "$url" --output "$output"
}

python_matches_version() {
  local python_bin=$1
  local requested=$2
  "$python_bin" - <<PY >/dev/null 2>&1
import sys
requested = "$requested".strip()
major_minor = f"{sys.version_info.major}.{sys.version_info.minor}"
raise SystemExit(0 if requested == major_minor else 1)
PY
}

python_is_healthy() {
  local python_bin=$1
  "$python_bin" - <<'PY' >/dev/null 2>&1
import ctypes
import csv
import encodings
print("ok")
PY
}

python_supports_venv_creation() {
  local python_bin=$1
  local probe_dir
  probe_dir=$(mktemp -d)
  if "$python_bin" -m venv --copies "$probe_dir/venv" >/dev/null 2>&1; then
    rm -rf "$probe_dir"
    return 0
  fi
  rm -rf "$probe_dir"
  return 1
}

rewrite_embedded_python_app_launcher() {
  local runtime_dir=$1
  local libpython_target=$2
  local python_app="$runtime_dir/lib/Resources/Python.app/Contents/MacOS/Python"

  if [[ ! -x "$python_app" ]]; then
    return 0
  fi

  local python_app_dep=""
  while IFS= read -r dep; do
    if [[ "$dep" == /*Python.framework/Versions/*/Python ]]; then
      python_app_dep=$dep
      break
    fi
  done < <(otool -L "$python_app" | tail -n +2 | awk '{print $1}')

  if [[ -z "$python_app_dep" ]]; then
    return 0
  fi

  install_name_tool -add_rpath "@executable_path/../../../../" "$python_app" 2>/dev/null || true
  install_name_tool -change "$python_app_dep" "@rpath/$(basename "$libpython_target")" "$python_app"
  codesign --force --sign - "$python_app" >/dev/null 2>&1 || true
}

verify_no_external_python_framework_refs() {
  local runtime_dir=$1
  local candidate

  for candidate in \
    "$runtime_dir/bin/python" \
    "$runtime_dir/bin/python3" \
    "$runtime_dir/lib/Resources/Python.app/Contents/MacOS/Python"; do
    if [[ ! -e "$candidate" ]]; then
      continue
    fi

    if otool -L "$candidate" | tail -n +2 | awk '{print $1}' | grep -E '^/.+Python\.framework/Versions/.+/Python$' >/dev/null; then
      echo "Bundled runtime still references an external Python.framework in $candidate" >&2
      otool -L "$candidate" >&2
      exit 1
    fi
  done
}

verify_sha256() {
  local file_path=$1
  local expected=$2
  local actual
  actual=$(shasum -a 256 "$file_path" | awk '{print $1}')
  if [[ "$actual" != "$expected" ]]; then
    echo "SHA256 mismatch for '$file_path': expected $expected, got $actual" >&2
    exit 1
  fi
}

bundle_torchcodec_ffmpeg_runtime() {
  local runtime_dir=$1
  local version_dir_name=$2
  local site_packages_dir="$runtime_dir/lib/$version_dir_name/site-packages"
  local torchcodec_dir="$site_packages_dir/torchcodec"

  if [[ ! -d "$torchcodec_dir" ]]; then
    return 0
  fi

  local archive="$STAGE_DIR/torchcodec-ffmpeg-${TORCHCODEC_FFMPEG_VERSION}.tar.gz"
  local extract_root="$STAGE_DIR/torchcodec-ffmpeg-${TORCHCODEC_FFMPEG_VERSION}"
  local ffmpeg_lib_dir="$extract_root/ffmpeg/lib"
  local embedded_lib_dir="$torchcodec_dir/.dylibs"

  rm -rf "$extract_root"
  mkdir -p "$extract_root" "$embedded_lib_dir"

  download_source_archive \
    "${TORCHCODEC_FFMPEG_BASE_URL}/${TORCHCODEC_FFMPEG_VERSION}.tar.gz" \
    "$archive"
  verify_sha256 "$archive" "$TORCHCODEC_FFMPEG_SHA256"
  tar -xzf "$archive" -C "$extract_root"

  local pattern
  for pattern in "${TORCHCODEC_FFMPEG_LIB_PATTERNS[@]}"; do
    rsync -a "$ffmpeg_lib_dir"/$pattern "$embedded_lib_dir"/
  done

  local binary
  while IFS= read -r binary; do
    install_name_tool -delete_rpath "/opt/homebrew/opt/ffmpeg/lib" "$binary" 2>/dev/null || true
    install_name_tool -delete_rpath "/usr/local/opt/ffmpeg/lib" "$binary" 2>/dev/null || true
    install_name_tool -add_rpath "@loader_path/.dylibs" "$binary" 2>/dev/null || true
    codesign --force --sign - "$binary" >/dev/null 2>&1 || true
  done < <(
    find "$torchcodec_dir" -maxdepth 1 -type f \
      \( -name 'libtorchcodec_core*.dylib' -o -name 'libtorchcodec_custom_ops*.dylib' -o -name 'libtorchcodec_pybind_ops*.so' \) \
      | sort
  )

  local dylib
  while IFS= read -r dylib; do
    codesign --force --sign - "$dylib" >/dev/null 2>&1 || true
  done < <(find "$embedded_lib_dir" -maxdepth 1 \( -type f -o -type l \) | sort)
}

assert_torchcodec_runtime_is_portable() {
  local runtime_dir=$1
  local version_dir_name=$2
  local torchcodec_dir="$runtime_dir/lib/$version_dir_name/site-packages/torchcodec"

  if [[ ! -d "$torchcodec_dir" ]]; then
    return 0
  fi

  local binary
  while IFS= read -r binary; do
    local host_refs
    host_refs=$(otool -L "$binary" | tail -n +2 | awk '{print $1}' | grep -E '^(/opt/homebrew|/usr/local)' || true)
    if [[ -n "$host_refs" ]]; then
      echo "Torchcodec binary still links against host paths: $binary" >&2
      printf ' - %s\n' $host_refs >&2
      exit 1
    fi

    local host_rpaths
    host_rpaths=$(otool -l "$binary" | awk '
      /LC_RPATH/ { flag=1; next }
      flag && /path / { print $2; flag=0 }
    ' | grep -E '^(/opt/homebrew|/usr/local)' || true)
    if [[ -n "$host_rpaths" ]]; then
      echo "Torchcodec binary still exposes host rpaths: $binary" >&2
      printf ' - %s\n' $host_rpaths >&2
      exit 1
    fi
  done < <(
    find "$torchcodec_dir" -maxdepth 1 -type f \
      \( -name 'libtorchcodec_core*.dylib' -o -name 'libtorchcodec_custom_ops*.dylib' -o -name 'libtorchcodec_pybind_ops*.so' \) \
      | sort
  )

  local required
  for required in libavutil.60.dylib libavcodec.62.dylib libavformat.62.dylib libavdevice.62.dylib libavfilter.11.dylib libswscale.9.dylib libswresample.6.dylib; do
    if [[ ! -e "$torchcodec_dir/.dylibs/$required" ]]; then
      echo "Torchcodec runtime is missing bundled FFmpeg library: $torchcodec_dir/.dylibs/$required" >&2
      exit 1
    fi
  done
}

bundle_portable_python_native_dependencies() {
  local runtime_dir=$1
  python3 - <<'PY' "$runtime_dir"
import os
import shutil
import stat
import subprocess
import sys
from pathlib import Path

runtime_root = Path(sys.argv[1]).resolve()
embedded_dir = runtime_root / "lib" / "embedded-dylibs"
host_prefixes = ("/opt/homebrew", "/usr/local")


def run_command(args: list[str], *, check: bool = True) -> str:
    completed = subprocess.run(
        args,
        check=check,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    return completed.stdout


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
    previous = ""
    for line in output.splitlines():
        stripped = line.strip()
        if previous == "cmd LC_RPATH" and stripped.startswith("path "):
            refs.append(stripped.split("path ", 1)[1].split(" (offset ", 1)[0])
        previous = stripped
    return refs


def candidate_binaries() -> list[Path]:
    roots: list[Path] = []
    for version_dir in sorted((runtime_root / "lib").glob("python3.*")):
        for relative in ("lib-dynload", "site-packages"):
            candidate = version_dir / relative
            if candidate.is_dir():
                roots.append(candidate)
    if embedded_dir.is_dir():
        roots.append(embedded_dir)

    binaries: list[Path] = []
    seen: set[str] = set()
    for root in roots:
        for path in sorted(root.rglob("*")):
            if not path.is_file() or path.suffix not in {".so", ".dylib"}:
                continue
            resolved = str(path.resolve())
            if resolved in seen:
                continue
            seen.add(resolved)
            binaries.append(path.resolve())
    return binaries


def loader_reference(binary: Path, target: Path) -> str:
    relative = os.path.relpath(target, start=binary.parent).replace(os.sep, "/")
    return f"@loader_path/{relative}"


def patch_install_id(path: Path) -> None:
    run_command(
        [
            "/usr/bin/install_name_tool",
            "-id",
            f"@rpath/{path.name}",
            str(path),
        ]
    )


def codesign(path: Path) -> None:
    run_command(
        [
            "/usr/bin/codesign",
            "--force",
            "--sign",
            "-",
            str(path),
        ]
    )


def ensure_owner_writable(path: Path) -> None:
    mode = path.stat().st_mode
    if not (mode & stat.S_IWUSR):
        path.chmod(mode | stat.S_IWUSR)


pending = candidate_binaries()
processed: set[Path] = set()

while pending:
    binary = pending.pop(0)
    if binary in processed or not binary.exists():
        continue
    processed.add(binary)

    try:
        deps_output = run_command(["/usr/bin/otool", "-L", str(binary)])
    except subprocess.CalledProcessError:
        continue

    changed = False
    for dep in parse_otool_dependencies(deps_output):
        if not dep.startswith(host_prefixes):
            continue
        source = Path(dep)
        if not source.exists():
            raise SystemExit(
                f"Host-managed dylib '{dep}' required by '{binary}' is missing on the build machine."
            )
        embedded_dir.mkdir(parents=True, exist_ok=True)
        target = embedded_dir / source.name
        if target.exists():
            ensure_owner_writable(target)
        else:
            shutil.copy2(source, target)
            ensure_owner_writable(target)
            patch_install_id(target)
        new_ref = loader_reference(binary, target)
        run_command(
            [
                "/usr/bin/install_name_tool",
                "-change",
                dep,
                new_ref,
                str(binary),
            ]
        )
        pending.append(target.resolve())
        changed = True

    try:
        rpath_output = run_command(["/usr/bin/otool", "-l", str(binary)])
    except subprocess.CalledProcessError:
        rpath_output = ""

    for rpath in parse_otool_rpaths(rpath_output):
        if not rpath.startswith(host_prefixes):
            continue
        run_command(
            [
                "/usr/bin/install_name_tool",
                "-delete_rpath",
                rpath,
                str(binary),
            ]
        )
        changed = True

    if binary.parent == embedded_dir:
        patch_install_id(binary)
        changed = True

    if changed:
        codesign(binary)
PY
}

assert_python_native_runtime_is_portable() {
  local runtime_dir=$1
  python3 - <<'PY' "$runtime_dir"
import subprocess
import sys
from pathlib import Path

runtime_root = Path(sys.argv[1]).resolve()
host_prefixes = ("/opt/homebrew", "/usr/local")


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
    previous = ""
    for line in output.splitlines():
        stripped = line.strip()
        if previous == "cmd LC_RPATH" and stripped.startswith("path "):
            refs.append(stripped.split("path ", 1)[1].split(" (offset ", 1)[0])
        previous = stripped
    return refs


roots = []
for version_dir in sorted((runtime_root / "lib").glob("python3.*")):
    for relative in ("lib-dynload", "site-packages"):
        candidate = version_dir / relative
        if candidate.is_dir():
            roots.append(candidate)
embedded_dir = runtime_root / "lib" / "embedded-dylibs"
if embedded_dir.is_dir():
    roots.append(embedded_dir)

for root in roots:
    for binary in sorted(root.rglob("*")):
        if not binary.is_file() or binary.suffix not in {".so", ".dylib"}:
            continue
        deps_output = subprocess.run(
            ["/usr/bin/otool", "-L", str(binary)],
            check=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
        ).stdout
        for dep in parse_otool_dependencies(deps_output):
            if dep.startswith(host_prefixes):
                raise SystemExit(
                    f"Bundled pyannote runtime still links against a host-managed path: {binary} -> {dep}"
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
                    f"Bundled pyannote runtime still exposes a host LC_RPATH: {binary} -> {rpath}"
                )
PY
}

resolve_python_executable() {
  local requested=$1
  local candidates=()
  local candidate_path

  if [[ -x "$requested" ]]; then
    candidates+=("$requested")
  fi

  while IFS= read -r candidate_path; do
    [[ -n "$candidate_path" ]] && candidates+=("$candidate_path")
  done < <(which -a "python$requested" 2>/dev/null || true)

  while IFS= read -r candidate_path; do
    [[ -n "$candidate_path" ]] && candidates+=("$candidate_path")
  done < <(which -a python3 2>/dev/null || true)

  if [[ -x "$HOME/miniconda3/bin/python3" ]]; then
    candidates+=("$HOME/miniconda3/bin/python3")
  fi

  local deduped=()
  local seen_candidates=""
  local candidate
  for candidate in "${candidates[@]}"; do
    [[ -n "$candidate" ]] || continue
    if [[ "$seen_candidates" != *$'\n'"$candidate"$'\n'* ]]; then
      deduped+=("$candidate")
      seen_candidates+=$'\n'"$candidate"$'\n'
    fi
  done

  for candidate in "${deduped[@]}"; do
    if python_matches_version "$candidate" "$requested" \
      && python_is_healthy "$candidate" \
      && python_supports_venv_creation "$candidate"; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done

  echo "Could not find a healthy Python $requested interpreter on this machine that can also create a relocatable venv." >&2
  echo "Install one first, then rerun this script. Example: brew install python@$requested" >&2
  exit 1
}

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
RESOURCE_ROOT="$ROOT_DIR/apps/desktop/src-tauri/resources/pyannote"
MODEL_DIR="$RESOURCE_ROOT/model"
PYTHON_ROOT="$RESOURCE_ROOT/python"

HOST_ARCH=$(uname -m)
case "$HOST_ARCH" in
  arm64)
    TARGET_TRIPLE="aarch64-apple-darwin"
    ;;
  x86_64)
    TARGET_TRIPLE="x86_64-apple-darwin"
    ;;
  *)
    echo "Unsupported macOS architecture: $HOST_ARCH" >&2
    exit 1
    ;;
esac

RUNTIME_DIR="$PYTHON_ROOT/$TARGET_TRIPLE"

if [[ $FORCE -eq 0 && -f "$MODEL_DIR/config.yaml" && -x "$RUNTIME_DIR/bin/python3" ]]; then
  echo "Bundled pyannote assets already present in $RESOURCE_ROOT"
  exit 0
fi

mkdir -p "$RESOURCE_ROOT" "$PYTHON_ROOT"
git lfs install >/dev/null

STAGE_DIR=$(mktemp -d)
trap 'rm -rf "$STAGE_DIR"' EXIT

STAGE_MODEL_DIR="$STAGE_DIR/model"
STAGE_RUNTIME_DIR="$STAGE_DIR/python/$TARGET_TRIPLE"

echo "Cloning offline pyannote model from $MODEL_REPO"
GIT_LFS_SKIP_SMUDGE=0 git clone --depth 1 "$MODEL_REPO" "$STAGE_MODEL_DIR"
(
  cd "$STAGE_MODEL_DIR"
  git lfs pull
)
rm -rf "$STAGE_MODEL_DIR/.git"
rm -f \
  "$STAGE_MODEL_DIR/.gitattributes" \
  "$STAGE_MODEL_DIR/README.md" \
  "$STAGE_MODEL_DIR/diarization.gif"

if [[ ! -f "$STAGE_MODEL_DIR/config.yaml" ]]; then
  echo "Downloaded model is missing config.yaml: $STAGE_MODEL_DIR" >&2
  exit 1
fi

echo "Creating bundled Python runtime for $TARGET_TRIPLE with Python $PYTHON_VERSION"
PYTHON_EXECUTABLE=$(resolve_python_executable "$PYTHON_VERSION")
echo "Using Python executable: $PYTHON_EXECUTABLE"
"$PYTHON_EXECUTABLE" -m venv --copies "$STAGE_RUNTIME_DIR"
"$STAGE_RUNTIME_DIR/bin/python" -m pip install --upgrade pip setuptools wheel >/dev/null
"$STAGE_RUNTIME_DIR/bin/python" -m pip install "pyannote.audio==4.0.4"

echo "Embedding Python standard library into the bundled runtime"
VERSION_DIR_NAME=$("$STAGE_RUNTIME_DIR/bin/python" - <<'PY'
import sys
print(f"python{sys.version_info.major}.{sys.version_info.minor}")
PY
)
SOURCE_STDLIB=$("$PYTHON_EXECUTABLE" - <<'PY'
import sysconfig
print(sysconfig.get_path("stdlib"))
PY
)
mkdir -p "$STAGE_RUNTIME_DIR/lib/$VERSION_DIR_NAME"
rsync -a --exclude 'site-packages' "$SOURCE_STDLIB/" "$STAGE_RUNTIME_DIR/lib/$VERSION_DIR_NAME/"
SOURCE_RESOURCES_DIR=$("$PYTHON_EXECUTABLE" - <<'PY'
import sys
from pathlib import Path

base = Path(sys.base_prefix)
candidates = [
    base / "Resources",
    base / "Frameworks" / "Python.framework" / "Versions" / f"{sys.version_info.major}.{sys.version_info.minor}" / "Resources",
]
for candidate in candidates:
    if candidate.is_dir():
        print(candidate)
        raise SystemExit(0)
PY
)
if [[ -n "$SOURCE_RESOURCES_DIR" ]]; then
  rsync -a "$SOURCE_RESOURCES_DIR/" "$STAGE_RUNTIME_DIR/lib/Resources/"
fi

echo "Embedding libpython into the bundled runtime"
LIBPY_SOURCE=$("$PYTHON_EXECUTABLE" - <<'PY'
import sys
import sysconfig
from pathlib import Path

version = f"{sys.version_info.major}.{sys.version_info.minor}"
ldlibrary = Path(sysconfig.get_config_var("LDLIBRARY") or "")
libdir = Path(sysconfig.get_config_var("LIBDIR") or "")
names = [f"libpython{version}.dylib"]
if ldlibrary.name and ldlibrary.name not in names:
    names.append(ldlibrary.name)

roots = [
    Path(sys.base_prefix),
    Path(sys.base_prefix).parent,
    Path(sys.base_prefix).parent.parent,
    libdir,
    libdir.parent,
]

seen = set()
for root in roots:
    if not root.exists():
        continue
    root = root.resolve()
    if root in seen:
        continue
    seen.add(root)
    for name in names:
        for match in root.rglob(name):
            if match.is_file():
                print(match)
                raise SystemExit(0)

raise SystemExit(
    f"Could not locate libpython via sysconfig (base_prefix={sys.base_prefix}, LIBDIR={libdir}, LDLIBRARY={ldlibrary})"
)
PY
)
LIBPY_TARGET="$STAGE_RUNTIME_DIR/lib/libpython${PYTHON_VERSION}.dylib"
cp "$LIBPY_SOURCE" "$LIBPY_TARGET"
install_name_tool -id "@rpath/$(basename "$LIBPY_TARGET")" "$LIBPY_TARGET"
PYTHON_BINARY_DEP=$(/usr/bin/otool -L "$STAGE_RUNTIME_DIR/bin/python3" | awk 'NR==2 {print $1}')
for python_bin in "$STAGE_RUNTIME_DIR"/bin/python*; do
  if [[ -x "$python_bin" ]]; then
    install_name_tool -add_rpath "@executable_path/../lib" "$python_bin" 2>/dev/null || true
    install_name_tool -change "$PYTHON_BINARY_DEP" "@rpath/$(basename "$LIBPY_TARGET")" "$python_bin" 2>/dev/null || true
    codesign --force --sign - "$python_bin" >/dev/null 2>&1 || true
  fi
done
rewrite_embedded_python_app_launcher "$STAGE_RUNTIME_DIR" "$LIBPY_TARGET"
codesign --force --sign - "$LIBPY_TARGET" >/dev/null 2>&1 || true
verify_no_external_python_framework_refs "$STAGE_RUNTIME_DIR"
echo "embedded $LIBPY_SOURCE -> $LIBPY_TARGET"

echo "Repairing Python config symlinks for standalone bundling"
STAGE_RUNTIME_DIR="$STAGE_RUNTIME_DIR" "$PYTHON_EXECUTABLE" - <<'PY'
import os
from pathlib import Path

runtime_root = Path(os.environ["STAGE_RUNTIME_DIR"])
lib_root = runtime_root / "lib"

for config_dir in lib_root.glob("python*/config-*-darwin"):
    if not config_dir.is_dir():
        continue
    for dylib_link in config_dir.glob("libpython*.dylib"):
        version = dylib_link.name.removeprefix("libpython").removesuffix(".dylib")
        target = Path("..") / ".." / f"libpython{version}.dylib"
        if dylib_link.exists() or dylib_link.is_symlink():
            dylib_link.unlink()
        dylib_link.symlink_to(target)
    for archive_link in config_dir.glob("libpython*.a"):
        if archive_link.exists() or archive_link.is_symlink():
            archive_link.unlink()
PY

echo "Removing pyvenv.cfg to keep the packaged runtime relocatable"
STAGE_RUNTIME_DIR="$STAGE_RUNTIME_DIR" "$PYTHON_EXECUTABLE" - <<'PY'
import os
from pathlib import Path

cfg_path = Path(os.environ["STAGE_RUNTIME_DIR"]) / "pyvenv.cfg"
if cfg_path.exists():
    cfg_path.unlink()
PY

echo "Bundling TorchCodec FFmpeg runtime"
bundle_torchcodec_ffmpeg_runtime "$STAGE_RUNTIME_DIR" "$VERSION_DIR_NAME"

echo "Bundling portable Python native dependencies"
bundle_portable_python_native_dependencies "$STAGE_RUNTIME_DIR"

echo "Asserting standalone portability for bundled Python native modules"
assert_torchcodec_runtime_is_portable "$STAGE_RUNTIME_DIR" "$VERSION_DIR_NAME"
assert_python_native_runtime_is_portable "$STAGE_RUNTIME_DIR"

echo "Verifying bundled pyannote runtime"
env -i \
  PATH="/usr/bin:/bin" \
  PYTHONHOME="$STAGE_RUNTIME_DIR" \
  PYTHONPATH="$STAGE_RUNTIME_DIR/lib/$VERSION_DIR_NAME:$STAGE_RUNTIME_DIR/lib/$VERSION_DIR_NAME/lib-dynload:$STAGE_RUNTIME_DIR/lib/$VERSION_DIR_NAME/site-packages" \
  PYTHONNOUSERSITE="1" \
  "$STAGE_RUNTIME_DIR/bin/python3" - <<PY
import ctypes
import csv
import encodings
from pyannote.audio import Pipeline
Pipeline.from_pretrained(r"$STAGE_MODEL_DIR")
print("pyannote pipeline loaded successfully")
PY

rm -rf "$RUNTIME_DIR"
mkdir -p "$(dirname "$RUNTIME_DIR")" "$MODEL_DIR"
rsync -a "$STAGE_RUNTIME_DIR/" "$RUNTIME_DIR/"
rsync -a --delete --exclude '.gitkeep' --exclude 'README.md' "$STAGE_MODEL_DIR/" "$MODEL_DIR/"

echo "Bundled pyannote runtime installed:"
echo "  runtime: $RUNTIME_DIR"
echo "  model:   $MODEL_DIR"
echo
echo "Next steps:"
echo "  1. Build the desktop app with 'cd apps/desktop && npm run tauri:build'"
echo "  2. On first launch, the app will auto-install these bundled assets into app data."
