#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "Usage: $0 <tauri_conf_path> <updater_pubkey>" >&2
  exit 1
fi

TAURI_CONF_PATH=$1
UPDATER_PUBKEY=$2

if [[ ! -f "$TAURI_CONF_PATH" ]]; then
  echo "Tauri config not found: $TAURI_CONF_PATH" >&2
  exit 1
fi

if [[ -z "$UPDATER_PUBKEY" ]]; then
  echo "Updater public key is empty." >&2
  exit 1
fi

python3 - "$TAURI_CONF_PATH" "$UPDATER_PUBKEY" <<'PY'
import json
import pathlib
import sys

config_path = pathlib.Path(sys.argv[1])
updater_pubkey = sys.argv[2]

data = json.loads(config_path.read_text())
data.setdefault("plugins", {}).setdefault("updater", {})["pubkey"] = updater_pubkey
config_path.write_text(json.dumps(data, indent=2) + "\n")
PY
