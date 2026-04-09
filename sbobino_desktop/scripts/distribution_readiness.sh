#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "Usage: $0 <version> [repo-slug]" >&2
  exit 1
fi

VERSION=$1
REPO_SLUG=${2:-pietroMastro92/sbobino_tauri}
TAG="v$VERSION"
BASE_URL="https://github.com/$REPO_SLUG/releases/download/$TAG"
TEMP_DIR=$(mktemp -d)

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
  local url="$BASE_URL/$asset_name"

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

def load_json(name: str):
    return json.loads((asset_dir / name).read_text())

latest = load_json("latest.json")
setup = load_json("setup-manifest.json")
runtime = load_json("runtime-manifest.json")
pyannote = load_json("pyannote-manifest.json")

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

runtime_assets = {asset.get("kind"): asset for asset in runtime.get("assets", [])}
runtime_asset = runtime_assets.get("speech_runtime_macos_aarch64")
if not isinstance(runtime_asset, dict):
    raise SystemExit("runtime-manifest.json is missing speech_runtime_macos_aarch64")
if runtime_asset.get("name") != runtime_asset_descriptor["name"]:
    raise SystemExit("runtime-manifest.json runtime asset name does not match setup-manifest.json")
if runtime_asset.get("sha256", "").strip().lower() != runtime_asset_descriptor["sha256"].strip().lower():
    raise SystemExit("runtime-manifest.json runtime asset checksum does not match setup-manifest.json")

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
if pyannote_model.get("name") != pyannote_model_descriptor["name"]:
    raise SystemExit("pyannote-manifest.json model asset name does not match setup-manifest.json")
if pyannote_model.get("sha256", "").strip().lower() != pyannote_model_descriptor["sha256"].strip().lower():
    raise SystemExit("pyannote-manifest.json model checksum does not match setup-manifest.json")

print(f"Distribution readiness passed for {tag} from {base_url}")
PY

echo "Distribution readiness checks passed for $TAG"
