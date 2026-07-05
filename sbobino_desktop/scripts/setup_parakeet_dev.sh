#!/usr/bin/env bash
set -euo pipefail

DRY_RUN=0
if [[ "${1:-}" == "--dry-run" ]]; then
  DRY_RUN=1
  shift
fi
if [[ $# -ne 0 ]]; then
  echo "Usage: $0 [--dry-run]" >&2
  exit 1
fi

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)
APP_ID=${SBOBINO_APP_ID:-com.sbobino.desktop}
APP_SUPPORT_DIR=${SBOBINO_APP_SUPPORT_DIR:-"$HOME/Library/Application Support/$APP_ID"}
INSTALL_BIN_DIR=${SBOBINO_PARAKEET_INSTALL_BIN_DIR:-"$APP_SUPPORT_DIR/bin"}
INSTALL_LIB_DIR=${SBOBINO_PARAKEET_INSTALL_LIB_DIR:-"$APP_SUPPORT_DIR/lib"}
MODELS_DIR=${SBOBINO_PARAKEET_MODELS_DIR:-"$APP_SUPPORT_DIR/parakeet-models"}
PARAKEET_CPP_REF=${SBOBINO_RUNTIME_PARAKEET_CPP_REF:-${SBOBINO_RUNTIME_PARAKEET_CPP_VERSION:-fa5aeef1e3d353679cbd374a426fee28387deb6e}}
MODEL_FILENAME=${SBOBINO_PARAKEET_MODEL:-tdt-0.6b-v3-f16.gguf}
EXTRA_MODELS=${SBOBINO_PARAKEET_EXTRA_MODELS:-nemotron-3.5-asr-streaming-0.6b-q4_k.gguf}
MODEL_BASE_URL="https://huggingface.co/mudler/parakeet-cpp-gguf/resolve/main"
BUILD_JOBS=${SBOBINO_RUNTIME_BUILD_JOBS:-$(sysctl -n hw.ncpu 2>/dev/null || echo 4)}
MACOS_DEPLOYMENT_TARGET=${SBOBINO_MACOS_RUNTIME_DEPLOYMENT_TARGET:-13.0}
PARAKEET_GGML_METAL=${SBOBINO_PARAKEET_METAL:-ON}

case "$PARAKEET_GGML_METAL" in
  1|ON|on|true|TRUE|yes|YES) PARAKEET_GGML_METAL=ON ;;
  0|OFF|off|false|FALSE|no|NO) PARAKEET_GGML_METAL=OFF ;;
  *) echo "Unsupported SBOBINO_PARAKEET_METAL value: $PARAKEET_GGML_METAL" >&2; exit 1 ;;
esac

case "$(uname -m)" in
  arm64) CMAKE_ARCH="arm64" ;;
  x86_64) CMAKE_ARCH="x86_64" ;;
  *)
    echo "Unsupported macOS architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

fail() {
  echo "error: $*" >&2
  exit 1
}

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    fail "missing required command: $1"
  fi
}

run() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
  if [[ "$DRY_RUN" -eq 0 ]]; then
    "$@"
  fi
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
  local ref
  ref=$(normalize_parakeet_ref "$PARAKEET_CPP_REF")
  run git clone https://github.com/mudler/parakeet.cpp.git "$SOURCE_ROOT"
  if [[ "$DRY_RUN" -eq 0 ]]; then
    (
      cd "$SOURCE_ROOT"
      git checkout "$ref"
      git submodule update --init --recursive --depth 1
    )
  else
    echo "+ cd '$SOURCE_ROOT' && git checkout '$ref'"
    echo "+ cd '$SOURCE_ROOT' && git submodule update --init --recursive --depth 1"
  fi
}

probe_parakeet_cli() {
  local output
  local status
  set +e
  output=$(env DYLD_LIBRARY_PATH="$INSTALL_LIB_DIR:${DYLD_LIBRARY_PATH:-}" "$INSTALL_CLI" --help 2>&1)
  status=$?
  set -e
  printf '%s\n' "$output"
  if [[ "$status" -ne 0 && "$output" != *"parakeet-cli transcribe"* ]]; then
    fail "parakeet-cli --help failed with status $status"
  fi
}

existing_manifest_value() {
  local key=$1
  [[ -f "$MANIFEST_PATH" ]] || return 0
  awk -F= -v key="$key" '$1 == key { print $2; found = 1; exit } END { if (!found) exit 0 }' "$MANIFEST_PATH"
}

write_runtime_manifest() {
  local resolved_ref=$1
  {
    echo "parakeet_cpp_ref=$PARAKEET_CPP_REF"
    echo "parakeet_cpp_resolved_ref=$resolved_ref"
    echo "parakeet_ggml_metal=$PARAKEET_GGML_METAL"
    echo "cmake_arch=$CMAKE_ARCH"
    echo "parakeet_cli_wrapper=$INSTALL_CLI"
    echo "parakeet_cli_binary=$INSTALL_CLI_BIN"
    echo "parakeet_batch_worker=$INSTALL_WORKER"
    echo "parakeet_live_library=$INSTALL_LIB"
    echo "model=$MODEL_FILENAME"
    if [[ -n "$EXTRA_MODELS" ]]; then
      echo "extra_models=$EXTRA_MODELS"
    fi
  } > "$MANIFEST_PATH"
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
  run chmod 755 "$wrapper"
}

need_cmd cmake
need_cmd curl
need_cmd git
need_cmd xcrun

SDKROOT=$(xcrun --sdk macosx --show-sdk-path)
WORK_DIR=$(mktemp -d "${TMPDIR:-/tmp}/sbobino-parakeet-dev.XXXXXX")

cleanup() {
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT

SOURCE_ROOT="$WORK_DIR/parakeet.cpp"
BUILD_ROOT="$WORK_DIR/parakeet-build"
INSTALL_CLI="$INSTALL_BIN_DIR/parakeet-cli"
INSTALL_CLI_BIN="$INSTALL_BIN_DIR/parakeet-cli-bin"
INSTALL_WORKER="$INSTALL_BIN_DIR/parakeet-batch-json"
INSTALL_LIB="$INSTALL_LIB_DIR/libparakeet.dylib"
MODEL_PATH="$MODELS_DIR/$MODEL_FILENAME"
MANIFEST_PATH="$INSTALL_BIN_DIR/parakeet-runtime-manifest.txt"

echo "parakeet_cpp_ref=$PARAKEET_CPP_REF"
echo "parakeet_ggml_metal=$PARAKEET_GGML_METAL"
echo "install_cli=$INSTALL_CLI"
echo "models_dir=$MODELS_DIR"
echo "model=$MODEL_FILENAME"
echo "extra_models=$EXTRA_MODELS"

run mkdir -p "$INSTALL_BIN_DIR" "$INSTALL_LIB_DIR" "$MODELS_DIR"

if [[ "${SBOBINO_PARAKEET_SKIP_BUILD:-0}" != "1" ]]; then
  checkout_parakeet_source

  run cmake -S "$SOURCE_ROOT" -B "$BUILD_ROOT" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_OSX_ARCHITECTURES="$CMAKE_ARCH" \
    -DCMAKE_OSX_DEPLOYMENT_TARGET="$MACOS_DEPLOYMENT_TARGET" \
    -DCMAKE_OSX_SYSROOT="$SDKROOT" \
    -DCMAKE_BUILD_WITH_INSTALL_RPATH=ON \
    -DCMAKE_INSTALL_RPATH="@loader_path;@loader_path/../lib" \
    -DPARAKEET_SHARED=ON \
    -DPARAKEET_BUILD_CLI=ON \
    -DPARAKEET_BUILD_TESTS=OFF \
    -DGGML_NATIVE=OFF \
    -DPARAKEET_GGML_METAL="$PARAKEET_GGML_METAL"
  run cmake --build "$BUILD_ROOT" -j"$BUILD_JOBS"

  if [[ "$DRY_RUN" -eq 0 ]]; then
    RESOLVED_REF=$(git -C "$SOURCE_ROOT" rev-parse HEAD)
    BUILT_CLI=$(find "$BUILD_ROOT" -type f -name parakeet-cli -print -quit)
    if [[ -z "$BUILT_CLI" ]]; then
      fail "Unable to find parakeet-cli after building parakeet.cpp"
    fi
    BUILT_LIB=$(find "$BUILD_ROOT" -type f -name libparakeet.dylib -print -quit)
    if [[ -z "$BUILT_LIB" ]]; then
      fail "Unable to find libparakeet.dylib after building parakeet.cpp"
    fi
    run cp "$BUILT_CLI" "$INSTALL_CLI_BIN"
    write_parakeet_cli_wrapper "$INSTALL_CLI" "parakeet-cli-bin"
    while IFS= read -r dylib; do
      run cp "$dylib" "$INSTALL_LIB_DIR/$(basename "$dylib")"
    done < <(find "$BUILD_ROOT" -type f -name 'lib*.dylib' -print)
    run clang++ -std=c++17 \
      -I"$SOURCE_ROOT/include" \
      "$SCRIPT_DIR/parakeet_batch_json.cpp" \
      -L"$INSTALL_LIB_DIR" \
      -lparakeet \
      -Wl,-rpath,@loader_path/../lib \
      -o "$INSTALL_WORKER"
    (
      cd "$INSTALL_LIB_DIR"
      for dylib in lib*.0.13.0.dylib; do
        [[ -e "$dylib" ]] || continue
        run ln -sf "$dylib" "${dylib/.0.13.0.dylib/.0.dylib}"
        run ln -sf "$dylib" "${dylib/.0.13.0.dylib/.dylib}"
      done
    )
    run chmod 755 "$INSTALL_CLI_BIN"
    run chmod 755 "$INSTALL_WORKER"
    for dylib in "$INSTALL_LIB_DIR"/lib*.dylib; do
      run chmod 755 "$dylib"
    done
    write_runtime_manifest "$RESOLVED_REF"
    probe_parakeet_cli
  else
    echo "+ find '$BUILD_ROOT' -type f -name parakeet-cli -print -quit"
    echo "+ cp <built-parakeet-cli> '$INSTALL_CLI_BIN'"
    echo "+ write parakeet CLI wrapper '$INSTALL_CLI'"
    echo "+ compile parakeet batch worker '$INSTALL_WORKER'"
    echo "+ cp <built-lib*.dylib> '$INSTALL_LIB_DIR/'"
    echo "+ ln -sf versioned ggml dylib names in '$INSTALL_LIB_DIR'"
    echo "+ chmod 755 '$INSTALL_CLI_BIN'"
    echo "+ chmod 755 '$INSTALL_WORKER'"
    echo "+ chmod 755 '$INSTALL_LIB_DIR'/lib*.dylib"
    echo "+ write '$MANIFEST_PATH'"
    echo "+ probe '$INSTALL_CLI' --help"
  fi
else
  [[ -x "$INSTALL_CLI" ]] || fail "SBOBINO_PARAKEET_SKIP_BUILD=1 but $INSTALL_CLI is not executable"
  probe_parakeet_cli
  if [[ "$DRY_RUN" -eq 0 ]]; then
    RESOLVED_REF=$(existing_manifest_value parakeet_cpp_resolved_ref)
    [[ -n "$RESOLVED_REF" ]] || RESOLVED_REF="unknown_skip_build"
    write_runtime_manifest "$RESOLVED_REF"
  fi
fi

if [[ "${SBOBINO_PARAKEET_SKIP_MODEL:-0}" != "1" ]]; then
  MODELS_TO_DOWNLOAD="$MODEL_FILENAME $EXTRA_MODELS"
  for model in $MODELS_TO_DOWNLOAD; do
    model_path="$MODELS_DIR/$model"
    if [[ -f "$model_path" ]]; then
      echo "model already present: $model_path"
    else
      run curl --fail --location --continue-at - --output "$model_path" "$MODEL_BASE_URL/$model"
    fi
  done
else
  [[ -f "$MODEL_PATH" ]] || fail "SBOBINO_PARAKEET_SKIP_MODEL=1 but $MODEL_PATH is missing"
fi

echo
echo "Parakeet dev runtime ready."
echo "CLI: $INSTALL_CLI"
echo "Model: $MODEL_PATH"
echo "Manifest: $MANIFEST_PATH"
echo
echo "Run dev app smoke with:"
echo "  scripts/smoke_parakeet_dev_app.sh"
