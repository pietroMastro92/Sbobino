#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
  echo "Usage: $0 <tauri_conf_path> [candidate_pubkey]" >&2
  exit 1
fi

TAURI_CONF_PATH=$1
CANDIDATE_PUBKEY=${2:-}
ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
EXPECTED_PUBKEY_PATH="$ROOT_DIR/scripts/updater-public-key.txt"

if [[ ! -f "$TAURI_CONF_PATH" ]]; then
  echo "Tauri config not found: $TAURI_CONF_PATH" >&2
  exit 1
fi

if [[ ! -f "$EXPECTED_PUBKEY_PATH" ]]; then
  echo "Expected updater public key file not found: $EXPECTED_PUBKEY_PATH" >&2
  exit 1
fi

EXPECTED_PUBKEY=$(tr -d '\n\r' < "$EXPECTED_PUBKEY_PATH")
CONFIG_PUBKEY=$(python3 - "$TAURI_CONF_PATH" <<'PY'
import json
import pathlib
import sys

path = pathlib.Path(sys.argv[1])
data = json.loads(path.read_text())
print(data.get("plugins", {}).get("updater", {}).get("pubkey", ""))
PY
)
CONFIG_PUBKEY=$(printf '%s' "$CONFIG_PUBKEY" | tr -d '\n\r')
CANDIDATE_PUBKEY=$(printf '%s' "$CANDIDATE_PUBKEY" | tr -d '\n\r')

if [[ -z "$EXPECTED_PUBKEY" ]]; then
  echo "Expected updater public key is empty." >&2
  exit 1
fi

if [[ "$CONFIG_PUBKEY" != "$EXPECTED_PUBKEY" ]]; then
  echo "Updater public key mismatch in $TAURI_CONF_PATH." >&2
  echo "The app updater key is part of the installed-client compatibility contract." >&2
  echo "Restore the historical public key or cut a manual reinstall-only release." >&2
  exit 1
fi

if [[ -n "$CANDIDATE_PUBKEY" && "$CANDIDATE_PUBKEY" != "$EXPECTED_PUBKEY" ]]; then
  echo "Provided updater public key does not match the historical public key." >&2
  echo "Release builds must use the private key paired with scripts/updater-public-key.txt." >&2
  echo "Otherwise existing installations cannot verify and apply updates." >&2
  exit 1
fi

echo "Updater public key verified."
