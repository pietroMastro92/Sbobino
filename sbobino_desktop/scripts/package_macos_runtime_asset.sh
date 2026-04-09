#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "Usage: $0 <output_zip>" >&2
  exit 1
fi

OUTPUT_ZIP=$1
ROOT_NAME="runtime"
MACOS_DEPLOYMENT_TARGET=${SBOBINO_MACOS_RUNTIME_DEPLOYMENT_TARGET:-13.0}
SDL2_VERSION=${SBOBINO_RUNTIME_SDL2_VERSION:-2.32.10}
WHISPER_CPP_VERSION=${SBOBINO_RUNTIME_WHISPER_CPP_VERSION:-1.8.4}
FFMPEG_VERSION=${SBOBINO_RUNTIME_FFMPEG_VERSION:-8.1}
BUILD_JOBS=${SBOBINO_RUNTIME_BUILD_JOBS:-$(sysctl -n hw.ncpu 2>/dev/null || echo 4)}

STAGE_DIR=$(mktemp -d)
SOURCE_DIR="$STAGE_DIR/src"
BUILD_DIR="$STAGE_DIR/build"
INSTALL_PREFIX="$STAGE_DIR/install"
TARGET_ROOT="$STAGE_DIR/$ROOT_NAME"
TARGET_BIN="$TARGET_ROOT/bin"
TARGET_LIB="$TARGET_ROOT/lib"

cleanup() {
  rm -rf "$STAGE_DIR"
}
trap cleanup EXIT

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

for command in clang cmake codesign curl make otool python3 tar xcrun; do
  need_cmd "$command"
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This runtime packaging flow only supports macOS." >&2
  exit 1
fi

if [[ "$(uname -m)" != "arm64" ]]; then
  echo "This runtime packaging flow must run on Apple Silicon (arm64)." >&2
  exit 1
fi

SDKROOT=$(xcrun --sdk macosx --show-sdk-path)

mkdir -p "$SOURCE_DIR" "$BUILD_DIR" "$INSTALL_PREFIX" "$TARGET_BIN" "$TARGET_LIB"
mkdir -p "$(dirname "$OUTPUT_ZIP")"
rm -f "$OUTPUT_ZIP"

download_source_archive() {
  local url=$1
  local output=$2
  curl --fail --location --silent --show-error "$url" --output "$output"
}

extract_source_archive() {
  local archive=$1
  local destination=$2
  mkdir -p "$destination"
  case "$archive" in
    *.tar.gz|*.tgz)
      tar -xzf "$archive" -C "$destination"
      ;;
    *.tar.xz)
      tar -xf "$archive" -C "$destination"
      ;;
    *)
      echo "Unsupported archive format: $archive" >&2
      exit 1
      ;;
  esac
}

read_binary_minos() {
  local binary=$1
  local minos
  minos=$(otool -l "$binary" | awk '
    /LC_BUILD_VERSION/ { flag=1; next }
    flag && $1 == "minos" { print $2; exit }
    /LC_VERSION_MIN_MACOSX/ { legacy=1; next }
    legacy && $1 == "version" { print $2; exit }
  ')
  if [[ -z "$minos" ]]; then
    echo "Unable to determine macOS deployment target for '$binary'." >&2
    exit 1
  fi
  printf '%s\n' "$minos"
}

assert_version_not_newer_than() {
  local allowed=$1
  local actual=$2
  python3 - "$allowed" "$actual" <<'PY'
import sys

def parse(value: str) -> tuple[int, ...]:
    return tuple(int(part) for part in value.split("."))

allowed = parse(sys.argv[1])
actual = parse(sys.argv[2])
if actual > allowed:
    raise SystemExit(1)
PY
}

assert_binary_portable() {
  local binary=$1
  local label=$2
  local minos
  minos=$(read_binary_minos "$binary")
  if ! assert_version_not_newer_than "$MACOS_DEPLOYMENT_TARGET" "$minos"; then
    echo "$label was built for macOS $minos, newer than the supported $MACOS_DEPLOYMENT_TARGET target." >&2
    exit 1
  fi

  local bad_refs
  bad_refs=$(otool -L "$binary" | tail -n +2 | awk '{print $1}' | grep -E '^(/opt/homebrew|/usr/local)' || true)
  if [[ -n "$bad_refs" ]]; then
    echo "$label still links against non-portable host paths:" >&2
    printf ' - %s\n' $bad_refs >&2
    exit 1
  fi
}

build_sdl2_static() {
  local archive="$SOURCE_DIR/SDL2-${SDL2_VERSION}.tar.gz"
  local source_root="$BUILD_DIR/SDL2-${SDL2_VERSION}"
  local build_root="$BUILD_DIR/sdl2-build"

  download_source_archive \
    "https://github.com/libsdl-org/SDL/releases/download/release-${SDL2_VERSION}/SDL2-${SDL2_VERSION}.tar.gz" \
    "$archive"
  extract_source_archive "$archive" "$BUILD_DIR"

  cmake -S "$source_root" -B "$build_root" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$INSTALL_PREFIX" \
    -DCMAKE_OSX_ARCHITECTURES=arm64 \
    -DCMAKE_OSX_DEPLOYMENT_TARGET="$MACOS_DEPLOYMENT_TARGET" \
    -DCMAKE_OSX_SYSROOT="$SDKROOT" \
    -DSDL_SHARED=OFF \
    -DSDL_STATIC=ON
  cmake --build "$build_root" -j"$BUILD_JOBS"
  cmake --install "$build_root"
}

build_whisper_binaries() {
  local archive="$SOURCE_DIR/whisper.cpp-${WHISPER_CPP_VERSION}.tar.gz"
  local source_root="$BUILD_DIR/whisper.cpp-${WHISPER_CPP_VERSION}"
  local build_root="$BUILD_DIR/whisper-build"

  download_source_archive \
    "https://github.com/ggml-org/whisper.cpp/archive/refs/tags/v${WHISPER_CPP_VERSION}.tar.gz" \
    "$archive"
  extract_source_archive "$archive" "$BUILD_DIR"

  PKG_CONFIG_PATH="$INSTALL_PREFIX/lib/pkgconfig" \
  cmake -S "$source_root" -B "$build_root" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$INSTALL_PREFIX" \
    -DCMAKE_PREFIX_PATH="$INSTALL_PREFIX" \
    -DCMAKE_OSX_ARCHITECTURES=arm64 \
    -DCMAKE_OSX_DEPLOYMENT_TARGET="$MACOS_DEPLOYMENT_TARGET" \
    -DCMAKE_OSX_SYSROOT="$SDKROOT" \
    -DBUILD_SHARED_LIBS=OFF \
    -DWHISPER_BUILD_EXAMPLES=ON \
    -DWHISPER_BUILD_TESTS=OFF \
    -DWHISPER_BUILD_SERVER=OFF \
    -DWHISPER_SDL2=ON \
    -DGGML_BLAS=OFF \
    -DGGML_ACCELERATE=OFF \
    -DWHISPER_USE_SYSTEM_GGML=OFF
  cmake --build "$build_root" -j"$BUILD_JOBS" --target whisper-cli whisper-stream

  cp "$build_root/bin/whisper-cli" "$TARGET_BIN/whisper-cli"
  cp "$build_root/bin/whisper-stream" "$TARGET_BIN/whisper-stream"
}

build_ffmpeg_binary() {
  local archive="$SOURCE_DIR/ffmpeg-${FFMPEG_VERSION}.tar.xz"
  local source_root="$BUILD_DIR/ffmpeg-${FFMPEG_VERSION}"

  download_source_archive \
    "https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VERSION}.tar.xz" \
    "$archive"
  extract_source_archive "$archive" "$BUILD_DIR"

  (
    cd "$source_root"
    export MACOSX_DEPLOYMENT_TARGET
    ./configure \
      --prefix="$INSTALL_PREFIX" \
      --arch=arm64 \
      --target-os=darwin \
      --enable-cross-compile \
      --cc=clang \
      --extra-cflags="-mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET}" \
      --extra-ldflags="-mmacosx-version-min=${MACOS_DEPLOYMENT_TARGET}" \
      --disable-autodetect \
      --disable-debug \
      --disable-doc \
      --disable-ffplay \
      --disable-ffprobe \
      --disable-network \
      --disable-appkit \
      --disable-avfoundation \
      --disable-audiotoolbox \
      --disable-coreimage \
      --disable-libxcb \
      --disable-libxcb-shm \
      --disable-libxcb-xfixes \
      --disable-metal \
      --disable-sdl2 \
      --disable-xlib \
      --disable-indevs \
      --disable-outdevs \
      --disable-securetransport \
      --disable-videotoolbox
    make -j"$BUILD_JOBS"
    make install
  )

  cp "$INSTALL_PREFIX/bin/ffmpeg" "$TARGET_BIN/ffmpeg"
}

probe_runtime_binary() {
  local binary=$1
  shift
  env -i \
    PATH="$TARGET_BIN:/usr/bin:/bin" \
    DYLD_LIBRARY_PATH="$TARGET_LIB" \
    DYLD_FALLBACK_LIBRARY_PATH="$TARGET_LIB" \
    "$binary" "$@" >/dev/null 2>&1
}

build_sdl2_static
build_whisper_binaries
build_ffmpeg_binary

for binary in ffmpeg whisper-cli whisper-stream; do
  chmod 755 "$TARGET_BIN/$binary"
  codesign --force --sign - "$TARGET_BIN/$binary" >/dev/null 2>&1 || true
done

assert_binary_portable "$TARGET_BIN/ffmpeg" "ffmpeg"
assert_binary_portable "$TARGET_BIN/whisper-cli" "whisper-cli"
assert_binary_portable "$TARGET_BIN/whisper-stream" "whisper-stream"

probe_runtime_binary "$TARGET_BIN/ffmpeg" -version
probe_runtime_binary "$TARGET_BIN/whisper-cli" --help
probe_runtime_binary "$TARGET_BIN/whisper-stream" --help

ditto -c -k --sequesterRsrc --keepParent "$TARGET_ROOT" "$OUTPUT_ZIP"
echo "Created $OUTPUT_ZIP"
