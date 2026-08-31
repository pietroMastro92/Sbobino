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

VERSION=${1#v}
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
  if [[ ! -s "$ASSET_DIR/$asset" ]]; then
    echo "Missing required candidate asset: $ASSET_DIR/$asset" >&2
    exit 1
  fi
done

if ! TAG_COMMIT_SHA=$(git rev-parse "$TAG^{commit}" 2>/dev/null); then
  echo "Local git tag $TAG does not exist. Create it before publishing the candidate." >&2
  exit 1
fi

PREVIOUS_NOTES_REF=${SBOBINO_RELEASE_NOTES_PREVIOUS_REF:-}
if [[ -z "$PREVIOUS_NOTES_REF" ]]; then
  PREVIOUS_NOTES_REF=$(git -C "$ROOT_DIR/.." describe --tags --abbrev=0 HEAD^ 2>/dev/null || true)
fi
if [[ -z "$PREVIOUS_NOTES_REF" ]]; then
  echo "Unable to determine previous release ref for release-notes.md." >&2
  exit 1
fi
"$ROOT_DIR/scripts/check_release_notes.sh" \
  "$VERSION" \
  "$ASSET_DIR/release-notes.md" \
  "$PREVIOUS_NOTES_REF"

CANONICAL_NOTES="$ROOT_DIR/docs/release-notes/v$VERSION.md"
if [[ ! -f "$CANONICAL_NOTES" ]]; then
  echo "Versioned release notes are missing from the checkout: $CANONICAL_NOTES" >&2
  exit 1
fi
if ! cmp -s "$CANONICAL_NOTES" "$ASSET_DIR/release-notes.md"; then
  echo "Candidate release-notes.md must be generated from $CANONICAL_NOTES." >&2
  exit 1
fi

python3 - <<'PY' "$VERSION" "$ASSET_DIR" "$REPO_SLUG" "$TAG_COMMIT_SHA"
import hashlib
import json
import pathlib
import re
import sys
import zipfile

version = sys.argv[1]
asset_dir = pathlib.Path(sys.argv[2])
repo_slug = sys.argv[3]
tag_commit_sha = sys.argv[4].strip().lower()
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
if proof.get("repo_slug") != repo_slug:
    raise SystemExit("Readiness proof repository does not match the requested repository.")
if not re.fullmatch(r"[0-9a-fA-F]{40}", str(proof.get("commit_sha", ""))):
    raise SystemExit("Readiness proof is missing a full commit SHA.")
if proof["commit_sha"].strip().lower() != tag_commit_sha:
    raise SystemExit(
        "Readiness proof commit does not match the candidate tag commit: "
        f"expected {tag_commit_sha}, got {proof['commit_sha']}"
    )

required_assets = [
    f"Sbobino_{version}_aarch64.dmg",
    f"Sbobino_{version}_x86_64.dmg",
    f"Sbobino_{version}_aarch64.app.tar.gz",
    f"Sbobino_{version}_aarch64.app.tar.gz.sig",
    f"Sbobino_{version}_x86_64.app.tar.gz",
    f"Sbobino_{version}_x86_64.app.tar.gz.sig",
    f"Sbobino_{version}_windows_x86_64-setup.exe",
    f"Sbobino_{version}_windows_x86_64.nsis.zip",
    f"Sbobino_{version}_windows_x86_64.nsis.zip.sig",
    "latest.json",
    "setup-manifest.json",
    "runtime-manifest.json",
    "speech-runtime-macos-aarch64.zip",
    "speech-runtime-macos-x86_64.zip",
    "speech-runtime-windows-x86_64.zip",
    "pyannote-manifest.json",
    "pyannote-runtime-macos-aarch64.zip",
    "pyannote-runtime-macos-x86_64.zip",
    "pyannote-runtime-windows-x86_64.zip",
    "pyannote-model-community-1.zip",
    "release-notes.md",
]
if proof.get("required_assets") != required_assets:
    raise SystemExit("Readiness proof required asset set does not match this candidate.")
if proof.get("optional_assets") != []:
    raise SystemExit("Readiness proof must not make candidate assets optional.")

checksums = proof.get("sha256")
if not isinstance(checksums, dict):
    raise SystemExit("Readiness proof is missing sha256 checksums.")
if set(checksums) != set(required_assets):
    raise SystemExit("Readiness proof must checksum every required candidate asset exactly once.")

for name in required_assets:
    expected = checksums[name]
    if not re.fullmatch(r"[0-9a-fA-F]{64}", str(expected).strip()):
        raise SystemExit(f"Readiness proof has an invalid SHA-256 checksum: {name}")
    path = asset_dir / name
    if not path.is_file() or path.stat().st_size <= 0:
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
release_base = f"https://github.com/{repo_slug}/releases/download/{expected_tag}"
for arch, target in (("aarch64", "darwin-aarch64"), ("x86_64", "darwin-x86_64")):
    platform = latest.get("platforms", {}).get(target)
    if not isinstance(platform, dict):
        raise SystemExit(f"latest.json is missing {target} updater metadata.")
    updater_tar = f"Sbobino_{version}_{arch}.app.tar.gz"
    if platform.get("url") != f"{release_base}/{updater_tar}":
        raise SystemExit(f"latest.json {target} updater URL is not architecture-matched.")
    if str(platform.get("signature", "")).strip() != (asset_dir / f"{updater_tar}.sig").read_text().strip():
        raise SystemExit(f"latest.json {target} updater signature mismatch.")
windows_updater = f"Sbobino_{version}_windows_x86_64.nsis.zip"
windows_platform = latest.get("platforms", {}).get("windows-x86_64")
if not isinstance(windows_platform, dict):
    raise SystemExit("latest.json is missing windows-x86_64 updater metadata.")
if windows_platform.get("url") != f"{release_base}/{windows_updater}":
    raise SystemExit("latest.json windows-x86_64 updater URL is not architecture-matched.")
if str(windows_platform.get("signature", "")).strip() != (asset_dir / f"{windows_updater}.sig").read_text().strip():
    raise SystemExit("latest.json windows-x86_64 updater signature mismatch.")
if setup.get("app_version") != version or setup.get("release_tag") != expected_tag:
    raise SystemExit("setup-manifest.json does not match requested release version/tag.")
if runtime.get("app_version") != version:
    raise SystemExit("runtime-manifest.json version does not match requested release version.")
if pyannote.get("app_version") != version:
    raise SystemExit("pyannote-manifest.json version does not match requested release version.")

try:
    setup_level = int(setup["pyannote_compat_level"])
    pyannote_level = int(pyannote["compat_level"])
except (KeyError, TypeError, ValueError) as error:
    raise SystemExit("setup and pyannote manifests must declare integer compatibility levels.") from error
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

if set(runtime_assets) != {
    "speech_runtime_macos_aarch64",
    "speech_runtime_macos_x86_64",
    "speech_runtime_windows_x86_64",
}:
    raise SystemExit("runtime-manifest.json must contain exactly the three packaged speech runtimes.")
if set(pyannote_assets) != {
    "pyannote_runtime_macos_aarch64",
    "pyannote_runtime_macos_x86_64",
    "pyannote_runtime_windows_x86_64",
    "pyannote_model",
}:
    raise SystemExit("pyannote-manifest.json must contain exactly the three runtimes and model.")

def assert_descriptor_matches_asset(descriptor: dict, release_asset: dict, label: str) -> None:
    if descriptor.get("name") != release_asset.get("name"):
        raise SystemExit(f"{label} name mismatch between setup and release manifest.")
    expected_sha = str(release_asset.get("sha256", "")).strip().lower()
    if not re.fullmatch(r"[0-9a-f]{64}", expected_sha):
        raise SystemExit(f"{label} has an invalid release-manifest checksum.")
    if str(descriptor.get("sha256", "")).strip().lower() != expected_sha:
        raise SystemExit(f"{label} checksum mismatch between setup and release manifest.")
    path = asset_dir / str(release_asset["name"])
    if release_asset.get("size_bytes") != path.stat().st_size:
        raise SystemExit(f"{label} size mismatch between manifest and packaged asset.")
    if expected_sha != str(checksums.get(path.name, "")).strip().lower():
        raise SystemExit(f"{label} checksum does not match readiness proof.")

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

for manifest_name, descriptor_key in (
    ("runtime-manifest.json", "runtime_manifest"),
    ("pyannote-manifest.json", "pyannote_manifest"),
):
    descriptor = setup.get(descriptor_key)
    if not isinstance(descriptor, dict):
        raise SystemExit(f"setup-manifest.json is missing {descriptor_key}.")
    manifest_path = asset_dir / manifest_name
    manifest_sha = hashlib.sha256(manifest_path.read_bytes()).hexdigest()
    if descriptor.get("name") != manifest_name or str(descriptor.get("sha256", "")).lower() != manifest_sha:
        raise SystemExit(f"setup-manifest.json {descriptor_key} does not match {manifest_name}.")
    if descriptor.get("size_bytes") != manifest_path.stat().st_size:
        raise SystemExit(f"setup-manifest.json {descriptor_key} size does not match {manifest_name}.")


def zip_members(path: pathlib.Path) -> set[str]:
    try:
        with zipfile.ZipFile(path) as archive:
            names = {
                name.replace("\\", "/").lstrip("./")
                for name in archive.namelist()
                if name and not name.endswith("/")
            }
    except (OSError, zipfile.BadZipFile) as error:
        raise SystemExit(f"Candidate asset is not a readable ZIP archive: {path}: {error}") from error
    if not names:
        raise SystemExit(f"Candidate asset ZIP is empty: {path}")
    return names


def require_member(filename: str, member: str) -> None:
    if member not in zip_members(asset_dir / filename):
        raise SystemExit(f"Candidate asset {filename} is missing packaged member {member}.")


for filename in ("speech-runtime-macos-aarch64.zip", "speech-runtime-macos-x86_64.zip"):
    for binary in ("ffmpeg", "whisper-cli", "whisper-stream", "parakeet-cli", "parakeet-batch-json"):
        require_member(filename, f"runtime/bin/{binary}")
for binary in ("ffmpeg.exe", "whisper-cli.exe", "whisper-stream.exe", "parakeet-cli.exe", "parakeet-batch-json.exe"):
    require_member("speech-runtime-windows-x86_64.zip", f"runtime/bin/{binary}")
for filename in ("pyannote-runtime-macos-aarch64.zip", "pyannote-runtime-macos-x86_64.zip"):
    require_member(filename, "python/bin/python3")
require_member("pyannote-runtime-windows-x86_64.zip", "python/python.exe")
require_member("pyannote-model-community-1.zip", "model/config.yaml")

PY

if gh release view "$TAG" --repo "$REPO_SLUG" >/dev/null 2>&1; then
  echo "Release $TAG already exists in $REPO_SLUG. Candidate versions must be fresh patch releases." >&2
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
  "$ASSET_DIR/release-notes.md" \
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
