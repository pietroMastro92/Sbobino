#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: publish_candidate_release.sh <version> [repo-slug] [asset-dir]

Creates a fresh GitHub prerelease candidate and uploads the full Sbobino asset set.
This command refuses to reuse an existing release for the same version.
It also refuses to publish if pre-release readiness proof or validation templates are missing or invalid.
EOF
}

if [[ $# -lt 1 || $# -gt 3 ]]; then
  usage
  exit 1
fi

VERSION=$1
REPO_SLUG=${2:-pietroMastro92/Sbobino}
ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
ASSET_DIR=${3:-"$ROOT_DIR/dist/local-release/v$VERSION"}
TAG="v$VERSION"

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

need_cmd gh
need_cmd git

if [[ ! -d "$ASSET_DIR" ]]; then
  echo "Candidate asset directory not found: $ASSET_DIR" >&2
  exit 1
fi

required_assets=(
  "Sbobino_${VERSION}_aarch64.dmg"
  "Sbobino_${VERSION}_x86_64.dmg"
  "Sbobino_${VERSION}_aarch64.app.tar.gz"
  "Sbobino_${VERSION}_aarch64.app.tar.gz.sig"
  "Sbobino_${VERSION}_x86_64.app.tar.gz"
  "Sbobino_${VERSION}_x86_64.app.tar.gz.sig"
  "Sbobino_${VERSION}_windows_x86_64-setup.exe"
  "Sbobino_${VERSION}_windows_x86_64.nsis.zip"
  "Sbobino_${VERSION}_windows_x86_64.nsis.zip.sig"
  "latest.json"
  "setup-manifest.json"
  "runtime-manifest.json"
  "speech-runtime-macos-aarch64.zip"
  "speech-runtime-macos-x86_64.zip"
  "speech-runtime-windows-x86_64.zip"
  "pyannote-manifest.json"
  "pyannote-runtime-macos-aarch64.zip"
  "pyannote-runtime-macos-x86_64.zip"
  "pyannote-runtime-windows-x86_64.zip"
  "pyannote-model-community-1.zip"
  "release-notes.md"
  "release-readiness-proof.json"
)

for asset in "${required_assets[@]}"; do
  if [[ ! -f "$ASSET_DIR/$asset" ]]; then
    echo "Missing required candidate asset: $ASSET_DIR/$asset" >&2
    exit 1
  fi
done

python3 - <<'PY' "$VERSION" "$ASSET_DIR"
import hashlib
import json
import pathlib
import sys

version = sys.argv[1]
asset_dir = pathlib.Path(sys.argv[2])
proof_path = asset_dir / "release-readiness-proof.json"
if not proof_path.is_file():
    raise SystemExit(
        "Missing release-readiness-proof.json. Run ./scripts/prepare_local_release.sh first."
    )

proof = json.loads(proof_path.read_text(encoding="utf-8"))
if proof.get("version") != version:
    raise SystemExit(
        f"Readiness proof version mismatch: expected {version}, got {proof.get('version')}"
    )
if str(proof.get("status", "")).strip().lower() != "passed":
    raise SystemExit("Readiness proof does not report a passed state.")
if proof.get("gate") != "release_readiness.sh":
    raise SystemExit("Readiness proof was not produced by release_readiness.sh.")

checksums = proof.get("sha256")
if not isinstance(checksums, dict):
    raise SystemExit("Readiness proof is missing sha256 checksums.")

for name, expected in checksums.items():
    path = asset_dir / name
    if not path.is_file():
        raise SystemExit(f"Readiness proof references missing asset: {name}")
    actual = hashlib.sha256(path.read_bytes()).hexdigest()
    if actual.lower() != str(expected).strip().lower():
        raise SystemExit(f"Asset checksum changed after readiness validation: {name}")

latest = json.loads((asset_dir / "latest.json").read_text(encoding="utf-8"))
setup = json.loads((asset_dir / "setup-manifest.json").read_text(encoding="utf-8"))
runtime = json.loads((asset_dir / "runtime-manifest.json").read_text(encoding="utf-8"))
pyannote = json.loads((asset_dir / "pyannote-manifest.json").read_text(encoding="utf-8"))

expected_tag = f"v{version}"
if latest.get("version") != version:
    raise SystemExit("latest.json version does not match requested release version.")
for arch, target in (("aarch64", "darwin-aarch64"), ("x86_64", "darwin-x86_64")):
    platform = latest.get("platforms", {}).get(target)
    if not isinstance(platform, dict):
        raise SystemExit(f"latest.json is missing {target} updater metadata.")
    updater_tar = f"Sbobino_{version}_{arch}.app.tar.gz"
    if not str(platform.get("url", "")).endswith(f"/{updater_tar}"):
        raise SystemExit(f"latest.json {target} updater URL is not architecture-matched.")
    if str(platform.get("signature", "")).strip() != (asset_dir / f"{updater_tar}.sig").read_text().strip():
        raise SystemExit(f"latest.json {target} updater signature mismatch.")
windows_updater = f"Sbobino_{version}_windows_x86_64.nsis.zip"
windows_platform = latest.get("platforms", {}).get("windows-x86_64")
if not isinstance(windows_platform, dict):
    raise SystemExit("latest.json is missing windows-x86_64 updater metadata.")
if not str(windows_platform.get("url", "")).endswith(f"/{windows_updater}"):
    raise SystemExit("latest.json windows-x86_64 updater URL is not architecture-matched.")
if str(windows_platform.get("signature", "")).strip() != (asset_dir / f"{windows_updater}.sig").read_text().strip():
    raise SystemExit("latest.json windows-x86_64 updater signature mismatch.")
if setup.get("app_version") != version or setup.get("release_tag") != expected_tag:
    raise SystemExit("setup-manifest.json does not match requested release version/tag.")
if runtime.get("app_version") != version:
    raise SystemExit("runtime-manifest.json version does not match requested release version.")
if pyannote.get("app_version") != version:
    raise SystemExit("pyannote-manifest.json version does not match requested release version.")

setup_level = int(setup.get("pyannote_compat_level", 1))
pyannote_level = int(pyannote.get("compat_level", 1))
if setup_level != pyannote_level:
    raise SystemExit("setup and pyannote compatibility levels are inconsistent.")

runtime_assets = {
    asset.get("kind"): asset
    for asset in runtime.get("assets", [])
    if isinstance(asset, dict)
}
pyannote_assets = {
    asset.get("kind"): asset
    for asset in pyannote.get("assets", [])
    if isinstance(asset, dict)
}

pyannote_model = pyannote_assets.get("pyannote_model")
if not isinstance(pyannote_model, dict):
    raise SystemExit("pyannote-manifest.json missing pyannote model asset.")

def assert_descriptor_matches_asset(descriptor: dict, release_asset: dict, label: str) -> None:
    if descriptor.get("name") != release_asset.get("name"):
        raise SystemExit(f"{label} name mismatch between setup and release manifest.")
    if str(descriptor.get("sha256", "")).strip().lower() != str(
        release_asset.get("sha256", "")
    ).strip().lower():
        raise SystemExit(f"{label} checksum mismatch between setup and release manifest.")

for arch, target, runtime_kind, pyannote_kind in (
    ("aarch64", "aarch64-apple-darwin", "speech_runtime_macos_aarch64", "pyannote_runtime_macos_aarch64"),
    ("x86_64", "x86_64-apple-darwin", "speech_runtime_macos_x86_64", "pyannote_runtime_macos_x86_64"),
    ("windows-x86_64", "x86_64-pc-windows-msvc", "speech_runtime_windows_x86_64", "pyannote_runtime_windows_x86_64"),
):
    runtime_release = runtime_assets.get(runtime_kind)
    pyannote_runtime = pyannote_assets.get(pyannote_kind)
    if not isinstance(runtime_release, dict) or not isinstance(pyannote_runtime, dict):
        raise SystemExit(f"release manifests are missing {arch} runtime assets.")
    runtime_descriptor = setup.get("runtime_assets", {}).get(target)
    pyannote_descriptor = setup.get("pyannote_runtime_assets", {}).get(target)
    if not isinstance(runtime_descriptor, dict) or not isinstance(pyannote_descriptor, dict):
        raise SystemExit(f"setup-manifest.json is missing {arch} runtime descriptors.")
    assert_descriptor_matches_asset(runtime_descriptor, runtime_release, f"{arch} runtime asset")
    assert_descriptor_matches_asset(pyannote_descriptor, pyannote_runtime, f"{arch} pyannote runtime asset")

model_descriptor = setup.get("pyannote_model_asset")
if not isinstance(model_descriptor, dict):
    raise SystemExit("setup-manifest.json is missing the pyannote model descriptor.")
assert_descriptor_matches_asset(model_descriptor, pyannote_model, "pyannote model asset")

PY

if gh release view "$TAG" --repo "$REPO_SLUG" >/dev/null 2>&1; then
  echo "Release $TAG already exists in $REPO_SLUG. Candidate versions must be fresh patch releases." >&2
  exit 1
fi

if ! git rev-parse "$TAG" >/dev/null 2>&1; then
  echo "Local git tag $TAG does not exist. Create it before publishing the candidate." >&2
  exit 1
fi

gh release create "$TAG" \
  --repo "$REPO_SLUG" \
  --title "$TAG" \
  --notes-file "$ASSET_DIR/release-notes.md" \
  --prerelease

gh release upload "$TAG" \
  "$ASSET_DIR/Sbobino_${VERSION}_aarch64.dmg" \
  "$ASSET_DIR/Sbobino_${VERSION}_x86_64.dmg" \
  "$ASSET_DIR/Sbobino_${VERSION}_aarch64.app.tar.gz" \
  "$ASSET_DIR/Sbobino_${VERSION}_aarch64.app.tar.gz.sig" \
  "$ASSET_DIR/Sbobino_${VERSION}_x86_64.app.tar.gz" \
  "$ASSET_DIR/Sbobino_${VERSION}_x86_64.app.tar.gz.sig" \
  "$ASSET_DIR/Sbobino_${VERSION}_windows_x86_64-setup.exe" \
  "$ASSET_DIR/Sbobino_${VERSION}_windows_x86_64.nsis.zip" \
  "$ASSET_DIR/Sbobino_${VERSION}_windows_x86_64.nsis.zip.sig" \
  "$ASSET_DIR/latest.json" \
  "$ASSET_DIR/setup-manifest.json" \
  "$ASSET_DIR/speech-runtime-macos-aarch64.zip" \
  "$ASSET_DIR/speech-runtime-macos-x86_64.zip" \
  "$ASSET_DIR/speech-runtime-windows-x86_64.zip" \
  "$ASSET_DIR/runtime-manifest.json" \
  "$ASSET_DIR/pyannote-runtime-macos-aarch64.zip" \
  "$ASSET_DIR/pyannote-runtime-macos-x86_64.zip" \
  "$ASSET_DIR/pyannote-runtime-windows-x86_64.zip" \
  "$ASSET_DIR/pyannote-model-community-1.zip" \
  "$ASSET_DIR/pyannote-manifest.json" \
  "$ASSET_DIR/release-readiness-proof.json" \
  --repo "$REPO_SLUG"

cat <<EOF
Prerelease candidate published successfully:
  repo: $REPO_SLUG
  tag:  $TAG

Next required steps:
  1. ./scripts/distribution_readiness.sh "$VERSION" "$REPO_SLUG"
  2. Run the hosted ARM64, Intel, and Windows validation matrix (automated in release.yml)
  3. Upload every hosted validation proof asset
  4. ./scripts/promote_candidate_release.sh "$VERSION" "$REPO_SLUG"
EOF
