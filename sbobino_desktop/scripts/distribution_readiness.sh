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

RELEASE_API_URL="https://api.github.com/repos/$REPO_SLUG/releases/tags/$TAG"

python3 - "$RELEASE_API_URL" <<'PY'
import json
import sys
import urllib.request

url = sys.argv[1]
request = urllib.request.Request(url, headers={"User-Agent": "sbobino-distribution-readiness"})
with urllib.request.urlopen(request) as response:
    release = json.load(response)
if release.get("draft", False):
    raise SystemExit("distribution_readiness.sh cannot validate draft releases.")
PY

ASSETS=(
  "Sbobino_${VERSION}_aarch64.dmg"
  "Sbobino.app.tar.gz"
  "Sbobino.app.tar.gz.sig"
  "latest.json"
  "setup-manifest.json"
  "runtime-manifest.json"
  "speech-runtime-macos-aarch64.zip"
  "pyannote-manifest.json"
  "pyannote-runtime-macos-aarch64.zip"
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

python3 - "$VERSION" "$TAG" "$BASE_URL" "$TEMP_DIR" <<'PY'
import hashlib
import json
import pathlib
import sys

version, tag, base_url, asset_dir_raw = sys.argv[1:5]
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

platform = latest.get("platforms", {}).get("darwin-aarch64")
if not isinstance(platform, dict):
    raise SystemExit("latest.json is missing the darwin-aarch64 updater payload.")

expected_tar_url = f"{base_url}/Sbobino.app.tar.gz"
if platform.get("url") != expected_tar_url:
    raise SystemExit(
        f"latest.json tarball URL mismatch: expected {expected_tar_url}, got {platform.get('url')}"
    )

expected_signature = (asset_dir / "Sbobino.app.tar.gz.sig").read_text().strip()
if platform.get("signature", "").strip() != expected_signature:
    raise SystemExit("latest.json signature does not match Sbobino.app.tar.gz.sig")

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

runtime_manifest_descriptor = ensure_setup_descriptor("runtime_manifest", "runtime-manifest.json")
runtime_asset_descriptor = ensure_setup_descriptor("runtime_asset", "speech-runtime-macos-aarch64.zip")
pyannote_manifest_descriptor = ensure_setup_descriptor("pyannote_manifest", "pyannote-manifest.json")
pyannote_runtime_descriptor = ensure_setup_descriptor(
    "pyannote_runtime_asset",
    "pyannote-runtime-macos-aarch64.zip",
)
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
runtime_asset = runtime_assets.get("speech_runtime_macos_aarch64")
if not isinstance(runtime_asset, dict):
    raise SystemExit("runtime-manifest.json is missing speech_runtime_macos_aarch64")
if runtime_asset.get("name") != runtime_asset_descriptor["name"]:
    raise SystemExit("runtime-manifest.json runtime asset name does not match setup-manifest.json")
if runtime_asset.get("sha256", "").strip().lower() != runtime_asset_descriptor["sha256"].strip().lower():
    raise SystemExit("runtime-manifest.json runtime asset checksum does not match setup-manifest.json")
if runtime_asset.get("size_bytes") != runtime_asset_descriptor.get("size_bytes"):
    raise SystemExit("runtime-manifest.json runtime asset size does not match setup-manifest.json")
if runtime_asset.get("expanded_size_bytes") != runtime_asset_descriptor.get("expanded_size_bytes"):
    raise SystemExit("runtime-manifest.json runtime expanded size does not match setup-manifest.json")

pyannote_assets = {asset.get("kind"): asset for asset in pyannote.get("assets", [])}
pyannote_runtime = pyannote_assets.get("pyannote_runtime_macos_aarch64")
pyannote_model = pyannote_assets.get("pyannote_model")
if not isinstance(pyannote_runtime, dict):
    raise SystemExit("pyannote-manifest.json is missing pyannote_runtime_macos_aarch64")
if not isinstance(pyannote_model, dict):
    raise SystemExit("pyannote-manifest.json is missing pyannote_model")
if pyannote_runtime.get("name") != pyannote_runtime_descriptor["name"]:
    raise SystemExit("pyannote-manifest.json runtime asset name does not match setup-manifest.json")
if pyannote_runtime.get("sha256", "").strip().lower() != pyannote_runtime_descriptor["sha256"].strip().lower():
    raise SystemExit("pyannote-manifest.json runtime checksum does not match setup-manifest.json")
if pyannote_runtime.get("size_bytes") != pyannote_runtime_descriptor.get("size_bytes"):
    raise SystemExit("pyannote-manifest.json runtime size does not match setup-manifest.json")
if pyannote_runtime.get("expanded_size_bytes") != pyannote_runtime_descriptor.get("expanded_size_bytes"):
    raise SystemExit("pyannote-manifest.json runtime expanded size does not match setup-manifest.json")
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

PYANNOTE_SMOKE_DIR="$TEMP_DIR/pyannote-smoke"
mkdir -p "$PYANNOTE_SMOKE_DIR"
/usr/bin/ditto -x -k "$TEMP_DIR/pyannote-runtime-macos-aarch64.zip" "$PYANNOTE_SMOKE_DIR"

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
                f"Remote pyannote runtime is missing bundled TorchCodec FFmpeg library: {torchcodec_dir / '.dylibs' / name}"
            )

import collections.abc
import ctypes
import csv
import encodings
import traceback
import types
import torch
from pyannote.audio import Pipeline
print("Remote pyannote runtime smoke test passed")
PY

echo "Distribution readiness checks passed for $TAG"
