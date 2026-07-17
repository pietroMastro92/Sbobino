#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: clean_release_validation_state.sh <machine-class>

Removes Sbobino app/runtime state from a dedicated release validation VM before
preflight. This is intentionally scoped to Sbobino paths only.
EOF
}

if [[ $# -ne 1 ]]; then
  usage
  exit 1
fi

MACHINE_CLASS=$1
case "$MACHINE_CLASS" in
  AS-THIRD|INTEL-PRIMARY)
    ;;
  *)
    echo "Refusing to clean release validation state for unsupported machine class: $MACHINE_CLASS" >&2
    exit 1
    ;;
esac

APP_ID=${SBOBINO_APP_ID:-com.sbobino.desktop}
APP_PATH=${SBOBINO_VALIDATION_APP_PATH:-/Applications/Sbobino.app}
DATA_DIR=${SBOBINO_VALIDATION_DATA_DIR:-"$HOME/Library/Application Support/$APP_ID"}
CACHE_DIR="$HOME/Library/Caches/$APP_ID"
LOG_DIR="$HOME/Library/Logs/$APP_ID"

safe_rm_rf() {
  local path=$1
  case "$path" in
    /Applications/Sbobino.app|"$HOME/Library/Application Support/$APP_ID"|"$HOME/Library/Caches/$APP_ID"|"$HOME/Library/Logs/$APP_ID"|/tmp/sbobino-*)
      if [[ -e "$path" || -L "$path" ]]; then
        rm -rf "$path"
        echo "removed: $path"
      fi
      ;;
    *)
      echo "Refusing to remove unexpected path: $path" >&2
      exit 1
      ;;
  esac
}

osascript -e 'tell application "Sbobino" to quit' >/dev/null 2>&1 || true
pkill -f "/Applications/Sbobino.app/Contents/MacOS/Sbobino" >/dev/null 2>&1 || true

safe_rm_rf "$APP_PATH"
safe_rm_rf "$DATA_DIR"
safe_rm_rf "$CACHE_DIR"
safe_rm_rf "$LOG_DIR"

for path in /tmp/sbobino-*; do
  [[ -e "$path" || -L "$path" ]] || continue
  safe_rm_rf "$path"
done

python3 - <<'PY'
import shutil
free = shutil.disk_usage("/").free / (1024 ** 3)
print(f"system_free_gb_after_cleanup={free:.1f}")
PY
