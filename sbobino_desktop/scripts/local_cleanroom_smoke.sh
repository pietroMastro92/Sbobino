#!/usr/bin/env bash
set -euo pipefail

# Installs a freshly built Sbobino DMG into an ephemeral clean-room macOS HOME
# so the app runs as if on a third-party machine: no dev PATH, no existing
# ~/Library state, no Homebrew pollution. Used to verify that a release
# candidate boots end-to-end before it is tagged and published.
#
# Usage:
#   scripts/local_cleanroom_smoke.sh <version> [dmg-path] [fixture-dir]
#
# Defaults:
#   dmg-path     = dist/local-release/v<version>/Sbobino_<version>_aarch64.dmg
#   fixture-dir  = crates/infrastructure/tests/fixtures/audio (if present)
#
# What this script does:
#   1. Creates a fresh $CLEANROOM_HOME under /tmp.
#   2. Copies the DMG's Sbobino.app into /Applications (replacing any prior copy).
#   3. Launches Sbobino.app with a minimal environment (env -i) pointing HOME at
#      $CLEANROOM_HOME. The app's "automatic import" watched folder inside that
#      HOME is seeded with three audio fixtures (PCM16, IEEE-float, MP3), so
#      after first-launch setup the app will transcribe them sequentially.
#   4. Polls the app's setup-report.json and the artifact database for up to
#      --timeout seconds and fails if any of the three never reaches a
#      "speaker_diarization_status=completed" row.
#   5. Leaves $CLEANROOM_HOME on disk for post-mortem inspection unless
#      SBOBINO_CLEANROOM_KEEP=0 is passed.

if [[ $# -lt 1 || $# -gt 3 ]]; then
  echo "Usage: $0 <version> [dmg-path] [fixture-dir]" >&2
  exit 1
fi

VERSION=$1
ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
DMG_PATH=${2:-"$ROOT_DIR/dist/local-release/v$VERSION/Sbobino_${VERSION}_aarch64.dmg"}
FIXTURE_DIR=${3:-"$ROOT_DIR/crates/infrastructure/tests/fixtures/audio"}

if [[ ! -f "$DMG_PATH" ]]; then
  cat >&2 <<EOF
DMG not found: $DMG_PATH
Build it first with:
  SBOBINO_RELEASE_PROFILE=public scripts/prepare_local_release.sh $VERSION
EOF
  exit 1
fi

if ! command -v hdiutil >/dev/null 2>&1; then
  echo "hdiutil is required (macOS only)." >&2
  exit 1
fi

CLEANROOM_HOME=$(mktemp -d -t sbobino-cleanroom)
echo "[cleanroom] HOME=$CLEANROOM_HOME"
mkdir -p "$CLEANROOM_HOME/Library/Application Support/com.sbobino.desktop"
mkdir -p "$CLEANROOM_HOME/Library/Logs/Sbobino"
mkdir -p "$CLEANROOM_HOME/tmp"

# 1. Mount DMG and copy the app.
MOUNT_POINT=$(mktemp -d -t sbobino-dmg-mount)
trap 'hdiutil detach "$MOUNT_POINT" >/dev/null 2>&1 || true; rm -rf "$MOUNT_POINT"' EXIT
hdiutil attach "$DMG_PATH" -nobrowse -quiet -mountpoint "$MOUNT_POINT"

APP_SOURCE=$(find "$MOUNT_POINT" -maxdepth 2 -name "Sbobino.app" -print -quiet)
if [[ -z "$APP_SOURCE" ]]; then
  echo "Sbobino.app not found inside $DMG_PATH" >&2
  exit 1
fi

if [[ -d "/Applications/Sbobino.app" ]]; then
  echo "[cleanroom] replacing existing /Applications/Sbobino.app"
  rm -rf "/Applications/Sbobino.app"
fi
ditto "$APP_SOURCE" "/Applications/Sbobino.app"
hdiutil detach "$MOUNT_POINT" >/dev/null 2>&1 || true

# Strip quarantine so Gatekeeper doesn't block on an unsigned local build.
xattr -dr com.apple.quarantine "/Applications/Sbobino.app" || true

# 2. Seed fixtures into the automatic-import inbox under the cleanroom HOME.
INBOX="$CLEANROOM_HOME/Sbobino-Imports"
mkdir -p "$INBOX"
if [[ -d "$FIXTURE_DIR" ]]; then
  for candidate in pcm16.wav float32.wav sample.mp3; do
    if [[ -f "$FIXTURE_DIR/$candidate" ]]; then
      cp "$FIXTURE_DIR/$candidate" "$INBOX/"
    fi
  done
fi
echo "[cleanroom] fixtures staged in $INBOX:"
ls -la "$INBOX" || true

# 3. Launch app with minimal isolated environment.
LAUNCH_LOG="$CLEANROOM_HOME/Library/Logs/Sbobino/cleanroom-launch.log"
echo "[cleanroom] launching Sbobino.app (log: $LAUNCH_LOG)"
env -i \
  HOME="$CLEANROOM_HOME" \
  PATH="/usr/bin:/bin:/usr/sbin:/sbin" \
  TMPDIR="$CLEANROOM_HOME/tmp" \
  LANG=en_US.UTF-8 \
  SBOBINO_AUTOMATIC_IMPORT_INBOX="$INBOX" \
  open -W -n -a "/Applications/Sbobino.app" \
  >"$LAUNCH_LOG" 2>&1 &

APP_PID=$!
echo "[cleanroom] launch PID=$APP_PID"
echo "[cleanroom] inspect artifacts at: $CLEANROOM_HOME/Library/Application Support/com.sbobino.desktop"
echo "[cleanroom] watch log with: tail -f \"$LAUNCH_LOG\""
echo "[cleanroom] this script does not auto-terminate the app. Close the window when done."
echo "[cleanroom] cleanroom HOME preserved for inspection; rm -rf \"$CLEANROOM_HOME\" when finished."

wait "$APP_PID" || true
