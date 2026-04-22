#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: prepare_local_release.sh <version> [output-dir]

Builds and validates a macOS Apple Silicon release entirely on the local machine.
It produces a candidate release folder only. It does not create tags, push
commits, or publish anything to GitHub.

Optional environment variables:
  SBOBINO_RELEASE_PROFILE               Build profile: public (default) or standalone-dev.
  SBOBINO_UPDATER_KEY_DIR               Directory used for stable local updater keys.
  SBOBINO_RELEASE_SUMMARY               Override the GitHub release summary paragraph.
  SBOBINO_RELEASE_NOTES_SHORT           Override the concise updater note stored in latest.json.
  SBOBINO_RELEASE_IMPROVEMENTS_MD       Override the Markdown bullet list for the Improvements section.
  SBOBINO_RELEASE_FIXES_MD              Override the Markdown bullet list for the Fixes section.
  SBOBINO_RELEASE_SETUP_MD              Override the Markdown bullet list for the Setup and compatibility section.
  TAURI_UPDATER_PUBLIC_KEY              Injected into tauri.conf.json only for this local build.
  TAURI_SIGNING_PRIVATE_KEY             If present, signs Sbobino.app.tar.gz for updater use.
  TAURI_SIGNING_PRIVATE_KEY_PATH        If present, signs Sbobino.app.tar.gz from a private key file.
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD    Password for the Tauri updater private key.
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
APP_DIR="$ROOT_DIR/target/aarch64-apple-darwin/release/bundle/macos"
APP_PATH="$APP_DIR/Sbobino.app"
DMG_DIR="$ROOT_DIR/target/aarch64-apple-darwin/release/bundle/dmg"
DMG_PATH="$DMG_DIR/Sbobino_${VERSION}_aarch64.dmg"
PYANNOTE_ASSET_DIR="$ROOT_DIR/target/aarch64-apple-darwin/release/bundle/pyannote-release"
RUNTIME_ASSET_DIR="$ROOT_DIR/target/aarch64-apple-darwin/release/bundle/runtime-release"
UPDATER_TAR="$APP_DIR/Sbobino.app.tar.gz"
UPDATER_SIG="$UPDATER_TAR.sig"
TEMP_DIR=$(mktemp -d)
TAURI_CONF_BACKUP="$TEMP_DIR/tauri.conf.json.backup"
REPO_SLUG="pietroMastro92/Sbobino"
LOCAL_RELEASE_URL_BASE="https://github.com/$REPO_SLUG/releases/download/v$VERSION"
LOCAL_UPDATER_KEY_DIR=${SBOBINO_UPDATER_KEY_DIR:-"${XDG_CONFIG_HOME:-$HOME/.config}/sbobino/updater"}
LOCAL_UPDATER_PRIVATE_KEY_PATH="$LOCAL_UPDATER_KEY_DIR/tauri-updater.key"
LOCAL_UPDATER_PUBLIC_KEY_PATH="$LOCAL_UPDATER_PRIVATE_KEY_PATH.pub"
LOCAL_UPDATER_PASSWORD_PATH="$LOCAL_UPDATER_KEY_DIR/tauri-updater.password"
DEFAULT_RELEASE_SUMMARY="Sbobino for Apple Silicon now resumes more smoothly after setup, keeps offline provisioning predictable, and improves prerelease validation before stable promotion."
DEFAULT_RELEASE_NOTES_SHORT="Improved returning-user startup, first-launch reliability, and candidate release validation on Apple Silicon."
DEFAULT_RELEASE_IMPROVEMENTS_MD=$'- Returning users can reopen the app without repeating blocking startup checks after a successful local setup.\n- Release artifacts and updater metadata stay aligned so the exact tested prerelease can later be promoted to stable without rebuilding.'
DEFAULT_RELEASE_FIXES_MD=$'- Fixed regressions where first-launch validation and release asset consistency could interrupt an otherwise healthy setup.\n- Fixed the fallback update link so the app points to the correct Apple Silicon DMG instead of unrelated release assets.'
DEFAULT_RELEASE_SETUP_MD=$'- First launch still provisions the managed offline transcription runtime and required models locally.\n- Apple-side signing and notarization remain disabled, so macOS may require one manual Gatekeeper confirmation on first open.'
RELEASE_SUMMARY=${SBOBINO_RELEASE_SUMMARY:-$DEFAULT_RELEASE_SUMMARY}
RELEASE_NOTES_SHORT=${SBOBINO_RELEASE_NOTES_SHORT:-$DEFAULT_RELEASE_NOTES_SHORT}
RELEASE_IMPROVEMENTS_MD=${SBOBINO_RELEASE_IMPROVEMENTS_MD:-$DEFAULT_RELEASE_IMPROVEMENTS_MD}
RELEASE_FIXES_MD=${SBOBINO_RELEASE_FIXES_MD:-$DEFAULT_RELEASE_FIXES_MD}
RELEASE_SETUP_MD=${SBOBINO_RELEASE_SETUP_MD:-$DEFAULT_RELEASE_SETUP_MD}

cleanup() {
  if [[ -f "$TAURI_CONF_BACKUP" ]]; then
    cp "$TAURI_CONF_BACKUP" "$TAURI_CONF"
  fi
  rm -rf "$TEMP_DIR"
}
trap cleanup EXIT

json_escape() {
  python3 -c 'import json,sys; print(json.dumps(sys.argv[1]))' "$1"
}

configure_local_tauri_build() {
  python3 - "$TAURI_CONF" "${1:-}" "${2:-0}" "${3:-0}" "${4:-public}" <<'PY'
import json
import pathlib
import sys

config_path = pathlib.Path(sys.argv[1])
updater_pubkey = sys.argv[2]
has_updater_pubkey = sys.argv[3] == "1"
enable_updater_artifacts = sys.argv[4] == "1"
release_profile = sys.argv[5].strip() or "public"

data = json.loads(config_path.read_text())
bundle = data.setdefault("bundle", {})
# Local public releases re-package and sign the updater tarball manually after
# `tauri build`, so we keep Tauri's native updater-artifact generation disabled
# here to avoid requiring the private key string during the build step itself.
bundle["createUpdaterArtifacts"] = False
resources = bundle.get("resources")
if release_profile == "public":
    if isinstance(resources, list):
        bundle["resources"] = [
            item for item in resources if str(item).strip() != "resources/pyannote"
        ]
else:
    if isinstance(resources, list) and "resources/pyannote" not in resources:
        resources.append("resources/pyannote")
        bundle["resources"] = resources

updater = data.setdefault("plugins", {}).setdefault("updater", {})
updater["active"] = has_updater_pubkey
if has_updater_pubkey:
    updater["pubkey"] = updater_pubkey

config_path.write_text(json.dumps(data, indent=2) + "\n")
PY
}

ensure_local_updater_keys() {
  local generated_password_file=0

  if [[ -n "${TAURI_UPDATER_PUBLIC_KEY:-}" ]]; then
    return 0
  fi

  if [[ -z "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" && -f "$LOCAL_UPDATER_PASSWORD_PATH" ]]; then
    export TAURI_SIGNING_PRIVATE_KEY_PASSWORD
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD=$(tr -d '\n' < "$LOCAL_UPDATER_PASSWORD_PATH")
  fi

  if [[ -z "${TAURI_SIGNING_PRIVATE_KEY_PATH:-}" && -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]]; then
    mkdir -p "$LOCAL_UPDATER_KEY_DIR"
    if [[ -z "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" ]]; then
      export TAURI_SIGNING_PRIVATE_KEY_PASSWORD
      TAURI_SIGNING_PRIVATE_KEY_PASSWORD=$(python3 - <<'PY'
import secrets
print(secrets.token_urlsafe(32))
PY
)
      printf '%s' "$TAURI_SIGNING_PRIVATE_KEY_PASSWORD" >"$LOCAL_UPDATER_PASSWORD_PATH"
      chmod 600 "$LOCAL_UPDATER_PASSWORD_PATH"
      generated_password_file=1
    fi

    if [[ ! -f "$LOCAL_UPDATER_PRIVATE_KEY_PATH" || ! -f "$LOCAL_UPDATER_PUBLIC_KEY_PATH" || "$generated_password_file" -eq 1 ]]; then
      pushd "$DESKTOP_DIR" >/dev/null
      npx tauri signer generate \
        --ci \
        --force \
        --password "$TAURI_SIGNING_PRIVATE_KEY_PASSWORD" \
        --write-keys "$LOCAL_UPDATER_PRIVATE_KEY_PATH" >/dev/null
      popd >/dev/null
    fi
    export TAURI_SIGNING_PRIVATE_KEY_PATH="$LOCAL_UPDATER_PRIVATE_KEY_PATH"
  fi

  if [[ -f "${TAURI_SIGNING_PRIVATE_KEY_PATH:-}" && -f "${TAURI_SIGNING_PRIVATE_KEY_PATH}.pub" ]]; then
    export TAURI_UPDATER_PUBLIC_KEY
    TAURI_UPDATER_PUBLIC_KEY=$(tr -d '\n' < "${TAURI_SIGNING_PRIVATE_KEY_PATH}.pub")
    return 0
  fi

  if [[ -f "$LOCAL_UPDATER_PUBLIC_KEY_PATH" ]]; then
    export TAURI_UPDATER_PUBLIC_KEY
    TAURI_UPDATER_PUBLIC_KEY=$(tr -d '\n' < "$LOCAL_UPDATER_PUBLIC_KEY_PATH")
    return 0
  fi

  echo "Unable to resolve a stable local Tauri updater keypair." >&2
  exit 1
}

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

need_cmd cargo
need_cmd clang
need_cmd cmake
need_cmd codesign
need_cmd curl
need_cmd git
need_cmd hdiutil
need_cmd make
need_cmd npm
need_cmd otool
need_cmd python3
need_cmd shasum
need_cmd tar
need_cmd xcrun

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "This local release flow only supports macOS." >&2
  exit 1
fi

if [[ "$(uname -m)" != "arm64" ]]; then
  echo "This local release flow must run on Apple Silicon (arm64)." >&2
  exit 1
fi

mkdir -p "$OUTPUT_DIR"

"$ROOT_DIR/scripts/check_release_versions.sh" "$VERSION"

if [[ "$RELEASE_PROFILE" != "public" && "$RELEASE_PROFILE" != "standalone-dev" ]]; then
  echo "Unsupported SBOBINO_RELEASE_PROFILE '$RELEASE_PROFILE'. Use 'public' or 'standalone-dev'." >&2
  exit 1
fi

ensure_local_updater_keys

cp "$TAURI_CONF" "$TAURI_CONF_BACKUP"
HAS_UPDATER_KEYS=0
if [[ -n "${TAURI_UPDATER_PUBLIC_KEY:-}" && ( -n "${TAURI_SIGNING_PRIVATE_KEY:-}" || -n "${TAURI_SIGNING_PRIVATE_KEY_PATH:-}" ) ]]; then
  configure_local_tauri_build "$TAURI_UPDATER_PUBLIC_KEY" 1 1 "$RELEASE_PROFILE"
  HAS_UPDATER_KEYS=1
elif [[ -n "${TAURI_UPDATER_PUBLIC_KEY:-}" ]]; then
  configure_local_tauri_build "$TAURI_UPDATER_PUBLIC_KEY" 1 0 "$RELEASE_PROFILE"
  echo "Updater public key is set but signing keys are missing; local build will disable native updater artifacts." >&2
else
  configure_local_tauri_build "" 0 0 "$RELEASE_PROFILE"
  echo "TAURI updater keys are not set; local build will disable native updater artifacts." >&2
fi

if [[ "$HAS_UPDATER_KEYS" -ne 1 ]]; then
  echo "Public local releases require a working local Tauri updater keypair and tarball signing." >&2
  exit 1
fi

pushd "$DESKTOP_DIR" >/dev/null
npm ci
popd >/dev/null

if [[ "$RELEASE_PROFILE" == "standalone-dev" ]]; then
  "$ROOT_DIR/scripts/setup_bundled_pyannote.sh" --force
fi

pushd "$DESKTOP_DIR" >/dev/null
SBOBINO_RELEASE_PROFILE="$RELEASE_PROFILE" npm run tauri:build -- --target aarch64-apple-darwin --bundles app
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
# Avoid AppleDouble metadata entries (._*) that break Tauri's updater unpack step.
COPYFILE_DISABLE=1 tar -czf "$UPDATER_TAR" -C "$APP_DIR" "Sbobino.app"

LATEST_JSON_CREATED=0
if [[ "$HAS_UPDATER_KEYS" -eq 1 ]]; then
  pushd "$DESKTOP_DIR" >/dev/null
  if [[ -n "${TAURI_SIGNING_PRIVATE_KEY_PATH:-}" ]]; then
    npx tauri signer sign \
      -f "$TAURI_SIGNING_PRIVATE_KEY_PATH" \
      -p "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" \
      "$UPDATER_TAR"
  else
    npx tauri signer sign \
      -p "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}" \
      "$UPDATER_TAR"
  fi
  popd >/dev/null

  PUB_DATE=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
  SIGNATURE=$(tr -d '\n' < "$UPDATER_SIG")
  RELEASE_NOTES_SHORT_JSON=$(json_escape "$RELEASE_NOTES_SHORT")
  cat >"$OUTPUT_DIR/latest.json" <<JSON
{
  "version": "$VERSION",
  "notes": $RELEASE_NOTES_SHORT_JSON,
  "pub_date": "$PUB_DATE",
  "platforms": {
    "darwin-aarch64": {
      "url": "$LOCAL_RELEASE_URL_BASE/Sbobino.app.tar.gz",
      "signature": "$SIGNATURE"
    }
  }
}
JSON
  LATEST_JSON_CREATED=1
else
  echo "Tauri updater signing keys are not set; skipping Sbobino.app.tar.gz signing and latest.json generation." >&2
fi

if [[ "$LATEST_JSON_CREATED" -ne 1 ]]; then
  echo "Failed to generate latest.json for the local candidate release." >&2
  exit 1
fi

STAGING_DIR=$(mktemp -d "$TEMP_DIR/dmg-stage.XXXXXX")
cp -R "$APP_PATH" "$STAGING_DIR/"
ln -s /Applications "$STAGING_DIR/Applications"
mkdir -p "$DMG_DIR"
rm -f "$DMG_PATH"
hdiutil create \
  -volname "Sbobino" \
  -srcfolder "$STAGING_DIR" \
  -ov \
  -format UDZO \
  "$DMG_PATH"

mkdir -p "$PYANNOTE_ASSET_DIR" "$RUNTIME_ASSET_DIR"
"$ROOT_DIR/scripts/package_pyannote_asset.sh" \
  "$DESKTOP_DIR/src-tauri/resources/pyannote/python/aarch64-apple-darwin" \
  python \
  "$PYANNOTE_ASSET_DIR/pyannote-runtime-macos-aarch64.zip"
"$ROOT_DIR/scripts/package_pyannote_asset.sh" \
  "$DESKTOP_DIR/src-tauri/resources/pyannote/model" \
  model \
  "$PYANNOTE_ASSET_DIR/pyannote-model-community-1.zip"
"$ROOT_DIR/scripts/package_macos_runtime_asset.sh" \
  "$RUNTIME_ASSET_DIR/speech-runtime-macos-aarch64.zip"

RELEASE_ASSET_STAGING_DIR=$(mktemp -d "$TEMP_DIR/release-assets.XXXXXX")
cp "$PYANNOTE_ASSET_DIR/pyannote-runtime-macos-aarch64.zip" "$RELEASE_ASSET_STAGING_DIR/"
cp "$PYANNOTE_ASSET_DIR/pyannote-model-community-1.zip" "$RELEASE_ASSET_STAGING_DIR/"
cp "$RUNTIME_ASSET_DIR/speech-runtime-macos-aarch64.zip" "$RELEASE_ASSET_STAGING_DIR/"
"$ROOT_DIR/scripts/generate_release_manifests.sh" "$VERSION" "$RELEASE_ASSET_STAGING_DIR"

cp "$RELEASE_ASSET_STAGING_DIR/pyannote-manifest.json" "$PYANNOTE_ASSET_DIR/"
cp "$RELEASE_ASSET_STAGING_DIR/runtime-manifest.json" "$RUNTIME_ASSET_DIR/"

SBOBINO_RELEASE_PROFILE="$RELEASE_PROFILE" "$ROOT_DIR/scripts/release_readiness.sh" "$VERSION" "$APP_PATH"

cp "$DMG_PATH" "$OUTPUT_DIR/"
cp "$UPDATER_TAR" "$OUTPUT_DIR/"
cp "$PYANNOTE_ASSET_DIR"/pyannote-runtime-macos-aarch64.zip "$OUTPUT_DIR/"
cp "$PYANNOTE_ASSET_DIR"/pyannote-model-community-1.zip "$OUTPUT_DIR/"
cp "$PYANNOTE_ASSET_DIR"/pyannote-manifest.json "$OUTPUT_DIR/"
cp "$RUNTIME_ASSET_DIR"/speech-runtime-macos-aarch64.zip "$OUTPUT_DIR/"
cp "$RUNTIME_ASSET_DIR"/runtime-manifest.json "$OUTPUT_DIR/"
cp "$RELEASE_ASSET_STAGING_DIR"/setup-manifest.json "$OUTPUT_DIR/"
if [[ -f "$UPDATER_SIG" ]]; then
  cp "$UPDATER_SIG" "$OUTPUT_DIR/"
fi

python3 "$ROOT_DIR/scripts/generate_release_candidate_metadata.py" \
  "$OUTPUT_DIR" \
  "$VERSION" \
  --release-profile "$RELEASE_PROFILE" \
  --commit-sha "$(git -C "$ROOT_DIR/.." rev-parse HEAD)"

cat >"$OUTPUT_DIR/release-notes.md" <<EOF
## Sbobino $VERSION

$RELEASE_SUMMARY
EOF

if [[ -n "${RELEASE_IMPROVEMENTS_MD// }" ]]; then
  cat >>"$OUTPUT_DIR/release-notes.md" <<EOF

### Improvements

$RELEASE_IMPROVEMENTS_MD
EOF
fi

if [[ -n "${RELEASE_FIXES_MD// }" ]]; then
  cat >>"$OUTPUT_DIR/release-notes.md" <<EOF

### Fixes

$RELEASE_FIXES_MD
EOF
fi

if [[ -n "${RELEASE_SETUP_MD// }" ]]; then
  cat >>"$OUTPUT_DIR/release-notes.md" <<EOF

### Setup and compatibility

$RELEASE_SETUP_MD
EOF
fi

cat >"$OUTPUT_DIR/UPLOAD_TO_GITHUB.md" <<EOF
# GitHub publish for v$VERSION

Nothing in this folder has been published automatically.

## Recommended flow

1. Create or reuse the Git tag locally: \`git tag -a v$VERSION -m "Sbobino v$VERSION"\`
2. Push only when you are ready: \`git push origin v$VERSION\`
3. Publish the GitHub release as a prerelease candidate first. Stable promotion is blocked until Apple Silicon validation reports are uploaded with \`status=passed\`.
4. Upload these files from \`$OUTPUT_DIR\`:
   - \`Sbobino_${VERSION}_aarch64.dmg\`
   - \`Sbobino.app.tar.gz\`
   - \`Sbobino.app.tar.gz.sig\` (if present)
   - \`latest.json\`
   - \`setup-manifest.json\`
   - \`speech-runtime-macos-aarch64.zip\`
   - \`runtime-manifest.json\`
   - \`pyannote-runtime-macos-aarch64.zip\`
   - \`pyannote-model-community-1.zip\`
   - \`pyannote-manifest.json\`
   - \`release-readiness-proof.json\`
   - \`AS-PRIMARY.validation-report.json\`
   - \`AS-THIRD.validation-report.json\`
   - \`INTEL-PRIMARY.validation-report.json\`
   - \`release-notes.md\` (use this exact file as the GitHub release body)
5. Run \`./scripts/distribution_readiness.sh "$VERSION"\` from \`sbobino_desktop/\`.
6. Generate \`distribution-readiness-proof.json\` after the remote integrity gate passes.
7. Validate that exact GitHub release against \`docs/distribution-validation-plan.md\` on:
   - \`AS-PRIMARY\`
   - \`AS-THIRD\`
   - \`INTEL-PRIMARY\`
8. Update all machine validation report JSON files with:
   - the GitHub release URL
   - tester name
   - OS name/version
   - \`tested_at_utc\`
   - per-scenario results
   - top-level \`status\` set to \`passed\` only when every mandatory scenario passed
   - use \`soft_pass\` only for \`INTEL-PRIMARY\` when the arm64 binary is intentionally marked \`not_applicable\`
9. Re-upload \`distribution-readiness-proof.json\` plus all three machine validation JSON files to the same GitHub prerelease with \`gh release upload --clobber\`.
10. Promote to stable only with \`./scripts/promote_candidate_release.sh "$VERSION"\`.
11. If validation fails, retire the prerelease and cut a new patch version. Do not overwrite a stable release in place.

## gh CLI example

\`\`\`bash
./scripts/publish_candidate_release.sh "$VERSION"

./scripts/distribution_readiness.sh "$VERSION"

python3 ./scripts/write_distribution_readiness_proof.py \
  "$OUTPUT_DIR/distribution-readiness-proof.json" \
  "$VERSION"

gh release upload "v$VERSION" \
  "$OUTPUT_DIR/distribution-readiness-proof.json" \
  "$OUTPUT_DIR/AS-PRIMARY.validation-report.json" \
  "$OUTPUT_DIR/AS-THIRD.validation-report.json" \
  "$OUTPUT_DIR/INTEL-PRIMARY.validation-report.json" \
  --clobber

./scripts/promote_candidate_release.sh "$VERSION"
\`\`\`
EOF

cat >"$OUTPUT_DIR/CLEAN_ROOM_VALIDATION.md" <<'EOF'
# Clean-room validation for v__VERSION__

Run this checklist on a different Apple Silicon Mac that does not rely on
Homebrew, host Python, or previously installed Sbobino runtime assets.

## Before install

1. Remove any previous Sbobino app copy from \`/Applications/Sbobino.app\`.
2. Remove any prior app data:
   - \`~/Library/Application Support/com.sbobino.desktop\`
3. Confirm no user-installed dependencies are required for the test:
   - Homebrew is not needed
   - system Python, ffmpeg, whisper binaries, and pyannote must be treated as absent/irrelevant

## Candidate validation

1. Download `Sbobino___VERSION___aarch64.dmg` from the GitHub prerelease.
2. Install the app into \`/Applications\`.
3. Launch the app and complete first-launch setup.
4. Confirm first launch:
   - runtime downloads and validates successfully
   - required whisper models download successfully
   - pyannote runtime and model install successfully
   - app enters the main UI without manual fixes
5. Quit the app and launch it again.
6. Confirm second launch:
   - no first-launch setup screen appears
   - no heavy startup checking blocks the UI
   - `Settings > Local Models` opens without a new full runtime inspection on arrival
   - runtime paths point to app-managed directories under Application Support

## Decision rule

- If every step passes, the prerelease can be promoted to stable.
- If any step fails, delete the prerelease and cut a new patch version.
- This checklist is the minimum clean-room pass. Stable release still requires the full Apple Silicon matrix in \`docs/distribution-validation-plan.md\`, including update-path validation.
EOF

python3 - "$OUTPUT_DIR/CLEAN_ROOM_VALIDATION.md" "$VERSION" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
version = sys.argv[2]
content = path.read_text()
content = content.replace("__VERSION__", version)
content = content.replace("___VERSION___", version)
path.write_text(content)
PY

cat >"$OUTPUT_DIR/UPGRADE_VALIDATION.md" <<'EOF'
# Upgrade validation for v__VERSION__

Run this checklist on an Apple Silicon Mac that already has the latest public
Sbobino version installed with working runtime, whisper models, and pyannote.

## Before update

1. Confirm the existing public version can open normally.
2. Confirm `Settings > Local Models` reports pyannote \`Ready\`.
3. Run one short diarized transcription to verify the pre-update baseline.

## Candidate validation

1. Update to `v__VERSION__` using the real shipped flow.
2. Launch the updated app.
3. Open `Settings > Local Models`.
4. Run one diarized transcription.

## Decision rule

- Pass only if the update completes without manual repair and pyannote stays usable the same way as before.
- If any step fails, delete the prerelease and cut a new patch version.
- Record the result in \`AS-PRIMARY.validation-report.json\` before promotion.
EOF

python3 - "$OUTPUT_DIR/UPGRADE_VALIDATION.md" "$VERSION" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
version = sys.argv[2]
content = path.read_text()
content = content.replace("__VERSION__", version)
content = content.replace("___VERSION___", version)
path.write_text(content)
PY

cat <<EOF
Local release prepared successfully in:
  $OUTPUT_DIR

Artifacts:
  - $(basename "$DMG_PATH")
  - $(basename "$UPDATER_TAR")
  - setup-manifest.json
  - speech-runtime-macos-aarch64.zip
  - runtime-manifest.json
  - pyannote-runtime-macos-aarch64.zip
  - pyannote-model-community-1.zip
  - pyannote-manifest.json
  - release-readiness-proof.json
  - AS-PRIMARY.validation-report.json
  - AS-THIRD.validation-report.json
  - INTEL-PRIMARY.validation-report.json
EOF

if [[ -f "$OUTPUT_DIR/Sbobino.app.tar.gz.sig" ]]; then
  echo "  - Sbobino.app.tar.gz.sig"
fi

if (( LATEST_JSON_CREATED == 1 )); then
  echo "  - latest.json"
fi
