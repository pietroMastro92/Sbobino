#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: prepare_local_release.sh <version> [output-dir]

Build one native macOS release staging directory for the current host
architecture. Run it once on Apple Silicon and once on an Intel Mac with the
same output directory; the second run assembles the publishable dual-arch
candidate automatically. This command only prepares local files: it does not
tag, push, or publish a GitHub release.

Required for public releases:
  TAURI_UPDATER_PUBLIC_KEY
  TAURI_SIGNING_PRIVATE_KEY or TAURI_SIGNING_PRIVATE_KEY_PATH
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD (when the key is password-protected)

Optional environment variables:
  SBOBINO_RELEASE_PROFILE=public|standalone-dev (default: public)
  SBOBINO_RELEASE_REPOSITORY=<owner/repository> (default: pietroMastro92/Sbobino)
EOF
}

if [[ $# -lt 1 || $# -gt 2 ]]; then
  usage
  exit 1
fi

VERSION=$1
ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
DESKTOP_DIR="$ROOT_DIR/apps/desktop"
TAURI_CONF="$DESKTOP_DIR/src-tauri/tauri.conf.json"
OUTPUT_DIR=${2:-"$ROOT_DIR/dist/local-release/v$VERSION"}
RELEASE_PROFILE=${SBOBINO_RELEASE_PROFILE:-public}

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This local release flow only supports macOS." >&2
  exit 1
fi

case "$(uname -m)" in
  arm64)
    TARGET_TRIPLE=aarch64-apple-darwin
    RELEASE_ARCH=aarch64
    ;;
  x86_64)
    TARGET_TRIPLE=x86_64-apple-darwin
    RELEASE_ARCH=x86_64
    ;;
  *)
    echo "Unsupported macOS architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

if [[ "$RELEASE_PROFILE" != "public" && "$RELEASE_PROFILE" != "standalone-dev" ]]; then
  echo "Unsupported SBOBINO_RELEASE_PROFILE '$RELEASE_PROFILE'." >&2
  exit 1
fi

if [[ -z "${TAURI_UPDATER_PUBLIC_KEY:-}" || ( -z "${TAURI_SIGNING_PRIVATE_KEY:-}" && -z "${TAURI_SIGNING_PRIVATE_KEY_PATH:-}" ) ]]; then
  echo "A local candidate requires Tauri updater public and private signing keys." >&2
  exit 1
fi

if [[ "$RELEASE_PROFILE" == "public" ]]; then
  "$ROOT_DIR/scripts/check_updater_public_key.sh" "$TAURI_CONF" "$TAURI_UPDATER_PUBLIC_KEY"
fi

for command in cargo codesign hdiutil npm python3 tar xcrun; do
  if ! command -v "$command" >/dev/null 2>&1; then
    echo "Missing required command: $command" >&2
    exit 1
  fi
done

STAGING_DIR="$OUTPUT_DIR/staging/$RELEASE_ARCH"
TARGET_DIR="$ROOT_DIR/target/$TARGET_TRIPLE/release/bundle"
APP_DIR="$TARGET_DIR/macos"
APP_PATH="$APP_DIR/Sbobino.app"
UPDATER_TAR="$APP_DIR/Sbobino.app.tar.gz"
UPDATER_SIG="$UPDATER_TAR.sig"
RUNTIME_DIR="$TARGET_DIR/runtime-release"
PYANNOTE_DIR="$TARGET_DIR/pyannote-release"
TEMP_DIR=$(mktemp -d)
TAURI_CONF_BACKUP="$TEMP_DIR/tauri.conf.json.backup"

cleanup() {
  if [[ -f "$TAURI_CONF_BACKUP" ]]; then
    cp "$TAURI_CONF_BACKUP" "$TAURI_CONF"
  fi
  rm -rf "$TEMP_DIR"
}
trap cleanup EXIT

configure_tauri_for_local_release() {
  python3 - "$TAURI_CONF" "$TAURI_UPDATER_PUBLIC_KEY" "$RELEASE_PROFILE" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
pubkey = sys.argv[2]
profile = sys.argv[3]
data = json.loads(path.read_text())
bundle = data.setdefault("bundle", {})
# The updater archive is signed below, after codesigning the completed app.
bundle["createUpdaterArtifacts"] = False
resources = bundle.get("resources", [])
if profile == "public":
    bundle["resources"] = [item for item in resources if str(item).strip() != "resources/pyannote"]
elif "resources/pyannote" not in resources:
    resources.append("resources/pyannote")
    bundle["resources"] = resources
updater = data.setdefault("plugins", {}).setdefault("updater", {})
updater["active"] = True
updater["pubkey"] = pubkey
path.write_text(json.dumps(data, indent=2) + "\n")
PY
}

mkdir -p "$STAGING_DIR" "$RUNTIME_DIR" "$PYANNOTE_DIR"
"$ROOT_DIR/scripts/check_release_versions.sh" "$VERSION"
cp "$TAURI_CONF" "$TAURI_CONF_BACKUP"
configure_tauri_for_local_release

pushd "$DESKTOP_DIR" >/dev/null
npm ci
if [[ "$RELEASE_PROFILE" == "standalone-dev" ]]; then
  "$ROOT_DIR/scripts/setup_bundled_pyannote.sh" --force
fi
SBOBINO_RELEASE_PROFILE="$RELEASE_PROFILE" npm run tauri:build -- --target "$TARGET_TRIPLE" --bundles app
popd >/dev/null

if [[ "$RELEASE_PROFILE" == "public" ]]; then
  "$ROOT_DIR/scripts/setup_bundled_pyannote.sh" --force
fi

if [[ ! -d "$APP_PATH" ]]; then
  echo "Expected built app at '$APP_PATH', but it was not created." >&2
  exit 1
fi

codesign --force --deep --sign - "$APP_PATH"
rm -f "$UPDATER_TAR" "$UPDATER_SIG"
COPYFILE_DISABLE=1 tar -czf "$UPDATER_TAR" -C "$APP_DIR" "Sbobino.app"

pushd "$DESKTOP_DIR" >/dev/null
if [[ -n "${TAURI_SIGNING_PRIVATE_KEY_PATH:-}" ]]; then
  npx tauri signer sign -f "$TAURI_SIGNING_PRIVATE_KEY_PATH" -p "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" "$UPDATER_TAR"
else
  npx tauri signer sign -p "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" "$UPDATER_TAR"
fi
popd >/dev/null

DMG_STAGE="$TEMP_DIR/dmg"
mkdir -p "$DMG_STAGE"
cp -R "$APP_PATH" "$DMG_STAGE/"
ln -s /Applications "$DMG_STAGE/Applications"
hdiutil create -volname "Sbobino" -srcfolder "$DMG_STAGE" -ov -format UDZO \
  "$STAGING_DIR/Sbobino_${VERSION}_${RELEASE_ARCH}.dmg"

"$ROOT_DIR/scripts/package_macos_runtime_asset.sh" \
  "$RUNTIME_DIR/speech-runtime-macos-$RELEASE_ARCH.zip"
"$ROOT_DIR/scripts/package_pyannote_asset.sh" \
  "$DESKTOP_DIR/src-tauri/resources/pyannote/python/$TARGET_TRIPLE" \
  python \
  "$PYANNOTE_DIR/pyannote-runtime-macos-$RELEASE_ARCH.zip"
"$ROOT_DIR/scripts/package_pyannote_asset.sh" \
  "$DESKTOP_DIR/src-tauri/resources/pyannote/model" \
  model \
  "$PYANNOTE_DIR/pyannote-model-community-1.zip"

SBOBINO_RELEASE_PROFILE="$RELEASE_PROFILE" \
SBOBINO_RELEASE_TARGET_TRIPLE="$TARGET_TRIPLE" \
  "$ROOT_DIR/scripts/release_readiness.sh" "$VERSION" "$APP_PATH"

cp "$UPDATER_TAR" "$STAGING_DIR/Sbobino_${VERSION}_${RELEASE_ARCH}.app.tar.gz"
cp "$UPDATER_SIG" "$STAGING_DIR/Sbobino_${VERSION}_${RELEASE_ARCH}.app.tar.gz.sig"
cp "$RUNTIME_DIR/speech-runtime-macos-$RELEASE_ARCH.zip" "$STAGING_DIR/"
cp "$PYANNOTE_DIR/pyannote-runtime-macos-$RELEASE_ARCH.zip" "$STAGING_DIR/"
cp "$PYANNOTE_DIR/pyannote-model-community-1.zip" "$STAGING_DIR/"

OTHER_ARCH=aarch64
if [[ "$RELEASE_ARCH" == "aarch64" ]]; then
  OTHER_ARCH=x86_64
fi

WINDOWS_STAGING="$OUTPUT_DIR/staging/windows-x86_64"
if [[ -f "$OUTPUT_DIR/staging/$OTHER_ARCH/Sbobino_${VERSION}_${OTHER_ARCH}.dmg" \
  && -f "$WINDOWS_STAGING/Sbobino_${VERSION}_windows_x86_64-setup.exe" ]]; then
  "$ROOT_DIR/scripts/assemble_local_release.sh" \
    "$VERSION" \
    "$OUTPUT_DIR/staging/aarch64" \
    "$OUTPUT_DIR/staging/x86_64" \
    "$WINDOWS_STAGING" \
    "$OUTPUT_DIR"
else
  cat <<EOF
Native $RELEASE_ARCH staging completed in:
  $STAGING_DIR

Final assembly requires native Apple Silicon, macOS Intel, and Windows x86_64
staging. Use the Release Candidate GitHub Actions workflow for the native
Windows build, or place its downloaded staging artifact in:
  $WINDOWS_STAGING
EOF
fi
