#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "Usage: $0 <output_zip>" >&2
  exit 1
fi

OUTPUT_ZIP=$1
SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
ROOT_NAME="runtime"
MACOS_DEPLOYMENT_TARGET=${SBOBINO_MACOS_RUNTIME_DEPLOYMENT_TARGET:-13.0}
SDL2_VERSION=${SBOBINO_RUNTIME_SDL2_VERSION:-2.32.10}
WHISPER_CPP_VERSION=${SBOBINO_RUNTIME_WHISPER_CPP_VERSION:-1.8.4}
PARAKEET_CPP_REF=${SBOBINO_RUNTIME_PARAKEET_CPP_REF:-${SBOBINO_RUNTIME_PARAKEET_CPP_VERSION:-fa5aeef1e3d353679cbd374a426fee28387deb6e}}
FFMPEG_VERSION=${SBOBINO_RUNTIME_FFMPEG_VERSION:-8.1}
BUILD_JOBS=${SBOBINO_RUNTIME_BUILD_JOBS:-$(sysctl -n hw.ncpu 2>/dev/null || echo 4)}
PARAKEET_GGML_METAL=${SBOBINO_PARAKEET_METAL:-ON}

case "$PARAKEET_GGML_METAL" in
  1|ON|on|true|TRUE|yes|YES) PARAKEET_GGML_METAL=ON ;;
  0|OFF|off|false|FALSE|no|NO) PARAKEET_GGML_METAL=OFF ;;
  *) echo "Unsupported SBOBINO_PARAKEET_METAL value: $PARAKEET_GGML_METAL" >&2; exit 1 ;;
esac

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

for command in clang clang++ cmake codesign curl find git lipo make otool python3 tar xcrun; do
  need_cmd "$command"
done

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This runtime packaging flow only supports macOS." >&2
  exit 1
fi

case "$(uname -m)" in
  arm64)
    RUNTIME_ARCH=arm64
    ;;
  x86_64)
    RUNTIME_ARCH=x86_64
    ;;
  *)
    echo "Unsupported macOS architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

SDKROOT=$(xcrun --sdk macosx --show-sdk-path)

mkdir -p "$SOURCE_DIR" "$BUILD_DIR" "$INSTALL_PREFIX" "$TARGET_BIN" "$TARGET_LIB"
mkdir -p "$(dirname "$OUTPUT_ZIP")"
rm -f "$OUTPUT_ZIP"

download_source_archive() {
  local url=$1
  local output=$2
  curl --fail --location --silent --show-error \
    --connect-timeout 20 \
    --retry 8 \
    --retry-delay 5 \
    --retry-max-time 240 \
    --retry-all-errors \
    "$url" \
    --output "$output"
}

retry_command() {
  local attempt=1
  local max_attempts=${SBOBINO_RUNTIME_NETWORK_RETRIES:-5}
  while true; do
    if "$@"; then
      return 0
    fi
    if (( attempt >= max_attempts )); then
      return 1
    fi
    echo "Command failed, retrying ($attempt/$max_attempts): $*" >&2
    sleep $(( attempt * 5 ))
    attempt=$((attempt + 1))
  done
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

normalize_parakeet_ref() {
  case "$1" in
    v*|[0-9a-f][0-9a-f][0-9a-f][0-9a-f]*|master|main|HEAD)
      printf '%s\n' "$1"
      ;;
    [0-9]*)
      printf 'v%s\n' "$1"
      ;;
    *)
      printf '%s\n' "$1"
      ;;
  esac
}

checkout_parakeet_source() {
  local source_root=$1
  local ref
  ref=$(normalize_parakeet_ref "$PARAKEET_CPP_REF")
  retry_command git clone https://github.com/mudler/parakeet.cpp.git "$source_root"
  (
    cd "$source_root"
    git checkout "$ref"
    retry_command git submodule update --init --recursive --depth 1
  )
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
    if [[ "${SBOBINO_RUNTIME_ALLOW_NONPORTABLE:-0}" != "1" ]]; then
      exit 1
    fi
    echo "SBOBINO_RUNTIME_ALLOW_NONPORTABLE=1: continuing anyway (local build)." >&2
  fi

  local bad_refs
  bad_refs=$(otool -L "$binary" | tail -n +2 | awk '{print $1}' | grep -E '^(/opt/homebrew|/usr/local)' || true)
  if [[ -n "$bad_refs" ]]; then
    echo "$label still links against non-portable host paths:" >&2
    printf ' - %s\n' $bad_refs >&2
    exit 1
  fi
}

write_parakeet_cli_wrapper() {
  local wrapper=$1
  local binary_name=$2
  cat > "$wrapper" <<EOF
#!/bin/sh
set -eu
SCRIPT_DIR=\$(cd "\$(dirname "\$0")" && pwd -P)
export GGML_METAL_NO_RESIDENCY=1
export GGML_METAL_SHARED_BUFFERS_DISABLE=1
export GGML_METAL_CONCURRENCY_DISABLE=1
exec "\$SCRIPT_DIR/$binary_name" "\$@"
EOF
  chmod 755 "$wrapper"
}

assert_binary_architecture() {
  local binary=$1
  local architectures
  architectures=$(lipo -archs "$binary")
  if [[ " $architectures " != *" $RUNTIME_ARCH "* ]]; then
    echo "$binary does not contain the expected $RUNTIME_ARCH architecture: $architectures" >&2
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
    -DCMAKE_OSX_ARCHITECTURES="$RUNTIME_ARCH" \
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
    -DCMAKE_OSX_ARCHITECTURES="$RUNTIME_ARCH" \
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

build_parakeet_binary() {
  local source_root="$BUILD_DIR/parakeet.cpp"
  local build_root="$BUILD_DIR/parakeet-build"
  local binary
  local library
  local resolved_ref

  checkout_parakeet_source "$source_root"
  resolved_ref=$(git -C "$source_root" rev-parse HEAD)

  cmake -S "$source_root" -B "$build_root" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$INSTALL_PREFIX" \
    -DCMAKE_OSX_ARCHITECTURES="$RUNTIME_ARCH" \
    -DCMAKE_OSX_DEPLOYMENT_TARGET="$MACOS_DEPLOYMENT_TARGET" \
    -DCMAKE_OSX_SYSROOT="$SDKROOT" \
    -DCMAKE_BUILD_WITH_INSTALL_RPATH=ON \
    -DCMAKE_INSTALL_RPATH="@loader_path;@loader_path/../lib" \
    -DPARAKEET_SHARED=ON \
    -DPARAKEET_BUILD_CLI=ON \
    -DPARAKEET_BUILD_TESTS=OFF \
    -DGGML_NATIVE=OFF \
    -DPARAKEET_GGML_METAL="$PARAKEET_GGML_METAL"
  cmake --build "$build_root" -j"$BUILD_JOBS"

  binary=$(find "$build_root" -type f -name parakeet-cli -print -quit)
  if [[ -z "$binary" ]]; then
    echo "Unable to find parakeet-cli after building parakeet.cpp." >&2
    exit 1
  fi

  cp "$binary" "$TARGET_BIN/parakeet-cli-bin"
  write_parakeet_cli_wrapper "$TARGET_BIN/parakeet-cli" "parakeet-cli-bin"
  library=$(find "$build_root" -type f -name libparakeet.dylib -print -quit)
  if [[ -z "$library" ]]; then
    echo "Unable to find libparakeet.dylib after building parakeet.cpp." >&2
    exit 1
  fi
  while IFS= read -r dylib; do
    cp "$dylib" "$TARGET_LIB/$(basename "$dylib")"
  done < <(find "$build_root" -type f -name 'lib*.dylib' -print)
  clang++ -std=c++17 \
    -arch "$RUNTIME_ARCH" \
    -mmacosx-version-min="$MACOS_DEPLOYMENT_TARGET" \
    -isysroot "$SDKROOT" \
    -I"$source_root/include" \
    "$SCRIPT_DIR/parakeet_batch_json.cpp" \
    -L"$TARGET_LIB" \
    -lparakeet \
    -Wl,-rpath,@loader_path/../lib \
    -o "$TARGET_BIN/parakeet-batch-json"
  (
    cd "$TARGET_LIB"
    for dylib in lib*.0.13.0.dylib; do
      [[ -e "$dylib" ]] || continue
      ln -sf "$dylib" "${dylib/.0.13.0.dylib/.0.dylib}"
      ln -sf "$dylib" "${dylib/.0.13.0.dylib/.dylib}"
    done
  )
  {
    echo "parakeet_cpp_ref=$PARAKEET_CPP_REF"
    echo "parakeet_cpp_resolved_ref=$resolved_ref"
    echo "parakeet_ggml_metal=$PARAKEET_GGML_METAL"
    echo "cmake_arch=$RUNTIME_ARCH"
    echo "parakeet_cli_wrapper=bin/parakeet-cli"
    echo "parakeet_cli_binary=bin/parakeet-cli-bin"
    echo "parakeet_batch_worker=bin/parakeet-batch-json"
    echo "parakeet_live_library=lib/libparakeet.dylib"
    echo "parakeet_shared_libraries=$(find "$TARGET_LIB" -type f -name 'lib*.dylib' | wc -l | tr -d ' ')"
  } > "$TARGET_BIN/parakeet-runtime-manifest.txt"
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
      --arch="$RUNTIME_ARCH" \
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

probe_parakeet_binary() {
  local output
  local status
  set +e
  output=$(env -i \
    PATH="$TARGET_BIN:/usr/bin:/bin" \
    DYLD_LIBRARY_PATH="$TARGET_LIB" \
    DYLD_FALLBACK_LIBRARY_PATH="$TARGET_LIB" \
    "$TARGET_BIN/parakeet-cli" --help 2>&1)
  status=$?
  set -e
  if [[ "$status" -ne 0 && "$output" != *"parakeet-cli transcribe"* ]]; then
    echo "$output" >&2
    echo "parakeet-cli --help probe failed with status $status." >&2
    exit 1
  fi
}

build_sdl2_static
build_whisper_binaries
build_parakeet_binary
build_ffmpeg_binary

for binary in ffmpeg whisper-cli whisper-stream parakeet-cli-bin parakeet-batch-json; do
  chmod 755 "$TARGET_BIN/$binary"
  codesign --force --sign - "$TARGET_BIN/$binary" >/dev/null 2>&1 || true
  assert_binary_architecture "$TARGET_BIN/$binary"
done
chmod 755 "$TARGET_BIN/parakeet-cli"
chmod 755 "$TARGET_LIB/libparakeet.dylib"
for dylib in "$TARGET_LIB"/lib*.dylib; do
  chmod 755 "$dylib"
  codesign --force --sign - "$dylib" >/dev/null 2>&1 || true
  assert_binary_architecture "$dylib"
done

assert_binary_portable "$TARGET_BIN/ffmpeg" "ffmpeg"
assert_binary_portable "$TARGET_BIN/whisper-cli" "whisper-cli"
assert_binary_portable "$TARGET_BIN/whisper-stream" "whisper-stream"
assert_binary_portable "$TARGET_BIN/parakeet-cli-bin" "parakeet-cli-bin"
assert_binary_portable "$TARGET_BIN/parakeet-batch-json" "parakeet-batch-json"
for dylib in "$TARGET_LIB"/lib*.dylib; do
  assert_binary_portable "$dylib" "$(basename "$dylib")"
done

probe_runtime_binary "$TARGET_BIN/ffmpeg" -version
probe_runtime_binary "$TARGET_BIN/whisper-cli" --help
probe_runtime_binary "$TARGET_BIN/whisper-stream" --help
probe_parakeet_binary

ditto -c -k --sequesterRsrc --keepParent "$TARGET_ROOT" "$OUTPUT_ZIP"
echo "Created $OUTPUT_ZIP"
