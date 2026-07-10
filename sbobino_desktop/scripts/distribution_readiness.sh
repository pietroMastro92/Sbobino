#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "Usage: $0 <version> [repo-slug]" >&2
  exit 1
fi

VERSION=$1
REPO_SLUG=${2:-pietroMastro92/Sbobino}
TAG="v$VERSION"
BASE_URL="https://github.com/$REPO_SLUG/releases/download/$TAG"
TEMP_DIR=$(mktemp -d)
CACHE_BUSTER=$(date +%s)

case "$(uname -m)" in
  arm64) RELEASE_ARCH=aarch64 ;;
  x86_64) RELEASE_ARCH=x86_64 ;;
  *)
    echo "Unsupported macOS architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

cleanup() {
  rm -rf "$TEMP_DIR"
}
trap cleanup EXIT

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

need_cmd curl
need_cmd python3
need_cmd shasum
need_cmd ditto
need_cmd lipo
need_cmd otool

RELEASE_API_URL="https://api.github.com/repos/$REPO_SLUG/releases/tags/$TAG"

python3 - "$RELEASE_API_URL" <<'PY'
import json
import os
import sys
import urllib.error
import urllib.request

url = sys.argv[1]
token = (os.environ.get("GH_TOKEN") or os.environ.get("GITHUB_TOKEN") or "").strip()
headers = {"User-Agent": "sbobino-distribution-readiness"}
if token:
    headers["Authorization"] = f"Bearer {token}"

request = urllib.request.Request(url, headers=headers)
try:
    with urllib.request.urlopen(request) as response:
        release = json.load(response)
except urllib.error.HTTPError as exc:
    if exc.code == 403 and not token:
        raise SystemExit(
            "GitHub API rate limit exceeded while checking release metadata. "
            "Set GH_TOKEN or GITHUB_TOKEN to run distribution_readiness.sh reliably."
        ) from exc
    raise
if release.get("draft", False):
    raise SystemExit("distribution_readiness.sh cannot validate draft releases.")
PY

ASSETS=(
  "Sbobino_${VERSION}_aarch64.dmg"
  "Sbobino_${VERSION}_x86_64.dmg"
  "Sbobino_${VERSION}_aarch64.app.tar.gz"
  "Sbobino_${VERSION}_aarch64.app.tar.gz.sig"
  "Sbobino_${VERSION}_x86_64.app.tar.gz"
  "Sbobino_${VERSION}_x86_64.app.tar.gz.sig"
  "latest.json"
  "setup-manifest.json"
  "runtime-manifest.json"
  "speech-runtime-macos-aarch64.zip"
  "speech-runtime-macos-x86_64.zip"
  "pyannote-manifest.json"
  "pyannote-runtime-macos-aarch64.zip"
  "pyannote-runtime-macos-x86_64.zip"
  "pyannote-model-community-1.zip"
)

download_asset() {
  local asset_name=$1
  local destination="$TEMP_DIR/$asset_name"
  local url="$BASE_URL/$asset_name?nocache=$CACHE_BUSTER"

  mkdir -p "$(dirname "$destination")"
  curl \
    --fail \
    --location \
    --retry 3 \
    --retry-delay 2 \
    --silent \
    --show-error \
    --user-agent "sbobino-distribution-readiness" \
    --output "$destination" \
    "$url"
}

for asset in "${ASSETS[@]}"; do
  download_asset "$asset"
done

python3 - "$VERSION" "$TAG" "$BASE_URL" "$TEMP_DIR" "$RELEASE_ARCH" <<'PY'
import hashlib
import json
import pathlib
import sys

version, tag, base_url, asset_dir_raw, host_arch = sys.argv[1:6]
asset_dir = pathlib.Path(asset_dir_raw)

def sha256(name: str) -> str:
    return hashlib.sha256((asset_dir / name).read_bytes()).hexdigest()

def file_size(name: str) -> int:
    return (asset_dir / name).stat().st_size

def expanded_size(name: str) -> int:
    path = asset_dir / name
    if path.suffix != ".zip":
        return file_size(name)
    import zipfile

    with zipfile.ZipFile(path) as archive:
        return sum(entry.file_size for entry in archive.infolist())

def load_json(name: str):
    return json.loads((asset_dir / name).read_text())

latest = load_json("latest.json")
setup = load_json("setup-manifest.json")
runtime = load_json("runtime-manifest.json")
pyannote = load_json("pyannote-manifest.json")
expected_pyannote_compat_level = 1

if latest.get("version") != version:
    raise SystemExit(f"latest.json version mismatch: expected {version}, got {latest.get('version')}")

architectures = {
    "aarch64": {
        "target": "aarch64-apple-darwin",
        "platform": "darwin-aarch64",
        "runtime_name": "speech-runtime-macos-aarch64.zip",
        "runtime_kind": "speech_runtime_macos_aarch64",
        "pyannote_name": "pyannote-runtime-macos-aarch64.zip",
        "pyannote_kind": "pyannote_runtime_macos_aarch64",
    },
    "x86_64": {
        "target": "x86_64-apple-darwin",
        "platform": "darwin-x86_64",
        "runtime_name": "speech-runtime-macos-x86_64.zip",
        "runtime_kind": "speech_runtime_macos_x86_64",
        "pyannote_name": "pyannote-runtime-macos-x86_64.zip",
        "pyannote_kind": "pyannote_runtime_macos_x86_64",
    },
}

if host_arch not in architectures:
    raise SystemExit(f"unsupported host architecture: {host_arch}")

for arch, descriptor in architectures.items():
    updater_tar = f"Sbobino_{version}_{arch}.app.tar.gz"
    updater_sig = f"{updater_tar}.sig"
    platform = latest.get("platforms", {}).get(descriptor["platform"])
    if not isinstance(platform, dict):
        raise SystemExit(f"latest.json is missing the {descriptor['platform']} updater payload.")
    expected_tar_url = f"{base_url}/{updater_tar}"
    if platform.get("url") != expected_tar_url:
        raise SystemExit(
            f"latest.json {descriptor['platform']} URL mismatch: expected {expected_tar_url}, got {platform.get('url')}"
        )
    if platform.get("signature", "").strip() != (asset_dir / updater_sig).read_text().strip():
        raise SystemExit(f"latest.json signature does not match {updater_sig}")

if setup.get("app_version") != version:
    raise SystemExit(f"setup-manifest.json app_version mismatch: expected {version}, got {setup.get('app_version')}")
if setup.get("release_tag") != tag:
    raise SystemExit(f"setup-manifest.json release_tag mismatch: expected {tag}, got {setup.get('release_tag')}")
if int(setup.get("pyannote_compat_level", expected_pyannote_compat_level)) != expected_pyannote_compat_level:
    raise SystemExit(
        "setup-manifest.json pyannote_compat_level mismatch: "
        f"expected {expected_pyannote_compat_level}, got {setup.get('pyannote_compat_level')}"
    )

def ensure_setup_descriptor(key: str, expected_name: str) -> dict:
    descriptor = setup.get(key)
    if not isinstance(descriptor, dict):
        raise SystemExit(f"setup-manifest.json is missing descriptor '{key}'")
    if descriptor.get("name") != expected_name:
        raise SystemExit(
            f"setup-manifest.json {key}.name mismatch: expected {expected_name}, got {descriptor.get('name')}"
        )
    checksum = descriptor.get("sha256", "").strip().lower()
    if not checksum:
        raise SystemExit(f"setup-manifest.json {key}.sha256 is missing")
    actual = sha256(expected_name)
    if checksum != actual:
        raise SystemExit(
            f"setup-manifest.json {key}.sha256 mismatch for {expected_name}: expected {checksum}, got {actual}"
        )
    expected_size = descriptor.get("size_bytes")
    if expected_size != file_size(expected_name):
        raise SystemExit(
            f"setup-manifest.json {key}.size_bytes mismatch for {expected_name}: expected {expected_size}, got {file_size(expected_name)}"
        )
    expected_expanded_size = descriptor.get("expanded_size_bytes")
    if expected_expanded_size != expanded_size(expected_name):
        raise SystemExit(
            f"setup-manifest.json {key}.expanded_size_bytes mismatch for {expected_name}: expected {expected_expanded_size}, got {expanded_size(expected_name)}"
        )
    return descriptor

def ensure_setup_arch_descriptor(key: str, target: str, expected_name: str) -> dict:
    descriptors = setup.get(key)
    if not isinstance(descriptors, dict):
        raise SystemExit(f"setup-manifest.json is missing descriptor map '{key}'")
    descriptor = descriptors.get(target)
    if not isinstance(descriptor, dict):
        raise SystemExit(f"setup-manifest.json is missing {key}.{target}")
    if descriptor.get("name") != expected_name:
        raise SystemExit(
            f"setup-manifest.json {key}.{target}.name mismatch: expected {expected_name}, got {descriptor.get('name')}"
        )
    checksum = descriptor.get("sha256", "").strip().lower()
    if checksum != sha256(expected_name):
        raise SystemExit(f"setup-manifest.json {key}.{target}.sha256 mismatch for {expected_name}")
    if descriptor.get("size_bytes") != file_size(expected_name):
        raise SystemExit(f"setup-manifest.json {key}.{target}.size_bytes mismatch for {expected_name}")
    if descriptor.get("expanded_size_bytes") != expanded_size(expected_name):
        raise SystemExit(f"setup-manifest.json {key}.{target}.expanded_size_bytes mismatch for {expected_name}")
    return descriptor

runtime_manifest_descriptor = ensure_setup_descriptor("runtime_manifest", "runtime-manifest.json")
pyannote_manifest_descriptor = ensure_setup_descriptor("pyannote_manifest", "pyannote-manifest.json")
pyannote_model_descriptor = ensure_setup_descriptor(
    "pyannote_model_asset",
    "pyannote-model-community-1.zip",
)

if runtime.get("app_version") != version:
    raise SystemExit(
        f"runtime-manifest.json app_version mismatch: expected {version}, got {runtime.get('app_version')}"
    )
if pyannote.get("app_version") != version:
    raise SystemExit(
        f"pyannote-manifest.json app_version mismatch: expected {version}, got {pyannote.get('app_version')}"
    )
if int(pyannote.get("compat_level", expected_pyannote_compat_level)) != expected_pyannote_compat_level:
    raise SystemExit(
        "pyannote-manifest.json compat_level mismatch: "
        f"expected {expected_pyannote_compat_level}, got {pyannote.get('compat_level')}"
    )

runtime_assets = {asset.get("kind"): asset for asset in runtime.get("assets", [])}
pyannote_assets = {asset.get("kind"): asset for asset in pyannote.get("assets", [])}
for arch, descriptor in architectures.items():
    runtime_asset_descriptor = ensure_setup_arch_descriptor(
        "runtime_assets", descriptor["target"], descriptor["runtime_name"]
    )
    pyannote_runtime_descriptor = ensure_setup_arch_descriptor(
        "pyannote_runtime_assets", descriptor["target"], descriptor["pyannote_name"]
    )
    runtime_asset = runtime_assets.get(descriptor["runtime_kind"])
    if not isinstance(runtime_asset, dict):
        raise SystemExit(f"runtime-manifest.json is missing {descriptor['runtime_kind']}")
    if any(runtime_asset.get(field) != runtime_asset_descriptor.get(field) for field in ("name", "sha256", "size_bytes", "expanded_size_bytes")):
        raise SystemExit(f"runtime-manifest.json {arch} asset does not match setup-manifest.json")
    pyannote_runtime = pyannote_assets.get(descriptor["pyannote_kind"])
    if not isinstance(pyannote_runtime, dict):
        raise SystemExit(f"pyannote-manifest.json is missing {descriptor['pyannote_kind']}")
    if any(pyannote_runtime.get(field) != pyannote_runtime_descriptor.get(field) for field in ("name", "sha256", "size_bytes", "expanded_size_bytes")):
        raise SystemExit(f"pyannote-manifest.json {arch} runtime does not match setup-manifest.json")

pyannote_model = pyannote_assets.get("pyannote_model")
if not isinstance(pyannote_model, dict):
    raise SystemExit("pyannote-manifest.json is missing pyannote_model")
if pyannote_model.get("name") != pyannote_model_descriptor["name"]:
    raise SystemExit("pyannote-manifest.json model asset name does not match setup-manifest.json")
if pyannote_model.get("sha256", "").strip().lower() != pyannote_model_descriptor["sha256"].strip().lower():
    raise SystemExit("pyannote-manifest.json model checksum does not match setup-manifest.json")
if pyannote_model.get("size_bytes") != pyannote_model_descriptor.get("size_bytes"):
    raise SystemExit("pyannote-manifest.json model size does not match setup-manifest.json")
if pyannote_model.get("expanded_size_bytes") != pyannote_model_descriptor.get("expanded_size_bytes"):
    raise SystemExit("pyannote-manifest.json model expanded size does not match setup-manifest.json")

print(f"Distribution readiness passed for {tag} from {base_url}")
PY

RUNTIME_SMOKE_DIR="$TEMP_DIR/runtime-smoke"
mkdir -p "$RUNTIME_SMOKE_DIR"
/usr/bin/ditto -x -k "$TEMP_DIR/speech-runtime-macos-$RELEASE_ARCH.zip" "$RUNTIME_SMOKE_DIR"

for binary in ffmpeg whisper-cli whisper-stream parakeet-cli; do
  candidate="$RUNTIME_SMOKE_DIR/runtime/bin/$binary"
  if [[ ! -x "$candidate" ]]; then
    echo "Remote speech runtime is missing executable: $candidate" >&2
    exit 1
  fi
  architectures=$(lipo -archs "$candidate")
  if [[ " $architectures " != *" $RELEASE_ARCH "* ]]; then
    echo "Remote speech runtime $binary is not $RELEASE_ARCH-native: $architectures" >&2
    exit 1
  fi
  if otool -L "$candidate" | tail -n +2 | awk '{print $1}' | grep -Eq '^(/opt/homebrew|/usr/local)'; then
    echo "Remote speech runtime $binary links against a host-managed path." >&2
    exit 1
  fi
done

PYANNOTE_SMOKE_DIR="$TEMP_DIR/pyannote-smoke"
mkdir -p "$PYANNOTE_SMOKE_DIR"
/usr/bin/ditto -x -k "$TEMP_DIR/pyannote-runtime-macos-$RELEASE_ARCH.zip" "$PYANNOTE_SMOKE_DIR"

PATH="/usr/bin:/bin" \
PYANNOTE_RUNTIME_ROOT="$PYANNOTE_SMOKE_DIR/python" \
PYTHONHOME="$PYANNOTE_SMOKE_DIR/python" \
PYTHONPATH="$PYANNOTE_SMOKE_DIR/python/lib/python3.11:$PYANNOTE_SMOKE_DIR/python/lib/python3.11/lib-dynload:$PYANNOTE_SMOKE_DIR/python/lib/python3.11/site-packages" \
PYTHONNOUSERSITE="1" \
"$PYANNOTE_SMOKE_DIR/python/bin/python3" - <<'PY'
import os
import pathlib
import subprocess

root = pathlib.Path(os.environ["PYANNOTE_RUNTIME_ROOT"])
host_prefixes = ("/opt/homebrew", "/usr/local", "/Library/Frameworks")


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
    # See setup_bundled_pyannote.sh: the line after `cmd LC_RPATH` is
    # `cmdsize NN`, not `path ...`, so the previous parser missed every
    # rpath and let host-managed Homebrew rpaths slip into the release.
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


def iter_runtime_native_binaries() -> list[pathlib.Path]:
    binaries: list[pathlib.Path] = []
    seen: set[pathlib.Path] = set()
    search_roots = []
    for version_dir in sorted((root / "lib").glob("python3.*")):
        for relative in ("lib-dynload", "site-packages"):
            candidate = version_dir / relative
            if candidate.is_dir():
                search_roots.append(candidate)
    embedded_dir = root / "lib" / "embedded-dylibs"
    if embedded_dir.is_dir():
        search_roots.append(embedded_dir)

    for search_root in search_roots:
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
                f"Remote pyannote runtime still links a native module against a host path: {binary} -> {dep}"
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
                f"Remote pyannote runtime still exposes a host LC_RPATH: {binary} -> {rpath}"
            )

torchcodec_dir = root / "lib" / "python3.11" / "site-packages" / "torchcodec"
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
                    f"Remote pyannote runtime still links torchcodec against a host path: {binary} -> {dep}"
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
                    f"Remote pyannote runtime still exposes a host LC_RPATH: {binary} -> {rpath}"
                )

    ffmpeg_roots = (
        torchcodec_dir / ".dylibs",
        root / "lib" / "embedded-dylibs",
        root / "lib",
    )
    for family in (
        "libavutil",
        "libavcodec",
        "libavformat",
        "libavdevice",
        "libavfilter",
        "libswscale",
        "libswresample",
    ):
        matches = [
            candidate
            for ffmpeg_root in ffmpeg_roots
            if ffmpeg_root.is_dir()
            for candidate in ffmpeg_root.glob(f"{family}*.dylib")
            if candidate.exists() or candidate.is_symlink()
        ]
        if not matches:
            raise SystemExit(
                f"Remote pyannote runtime is missing bundled TorchCodec FFmpeg library family: {family}"
            )

import collections.abc
import ctypes
import csv
import encodings
import ssl
import sqlite3
import traceback
import types
import torch
import torchcodec
from pyannote.audio import Pipeline
print("Remote pyannote runtime smoke test passed")
PY

echo "Distribution readiness checks passed for $TAG"
