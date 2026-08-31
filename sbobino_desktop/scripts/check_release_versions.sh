#!/usr/bin/env bash
set -euo pipefail

if [[ $# -gt 1 ]]; then
  echo "Usage: $0 [expected-version]" >&2
  exit 1
fi

EXPECTED_VERSION=${1:-}
RELEASE_PROFILE=${SBOBINO_RELEASE_PROFILE:-public}
ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
PACKAGE_JSON="$ROOT_DIR/apps/desktop/package.json"
TAURI_CONF="$ROOT_DIR/apps/desktop/src-tauri/tauri.conf.json"
CARGO_TOML="$ROOT_DIR/apps/desktop/src-tauri/Cargo.toml"
DOMAIN_CARGO_TOML="$ROOT_DIR/crates/domain/Cargo.toml"
APPLICATION_CARGO_TOML="$ROOT_DIR/crates/application/Cargo.toml"
INFRASTRUCTURE_CARGO_TOML="$ROOT_DIR/crates/infrastructure/Cargo.toml"
CARGO_LOCK="$ROOT_DIR/Cargo.lock"

if [[ -n "$EXPECTED_VERSION" && ! "$EXPECTED_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Expected release version must be semantic MAJOR.MINOR.PATCH (got '$EXPECTED_VERSION')." >&2
  exit 1
fi

PACKAGE_VERSION=$(node -p "JSON.parse(require('fs').readFileSync(process.argv[1], 'utf8')).version" "$PACKAGE_JSON")
TAURI_VERSION=$(node -p "JSON.parse(require('fs').readFileSync(process.argv[1], 'utf8')).version" "$TAURI_CONF")
CARGO_VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$CARGO_TOML" | head -n 1)
DOMAIN_VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$DOMAIN_CARGO_TOML" | head -n 1)
APPLICATION_VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$APPLICATION_CARGO_TOML" | head -n 1)
INFRASTRUCTURE_VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$INFRASTRUCTURE_CARGO_TOML" | head -n 1)

if [[ -z "$PACKAGE_VERSION" || -z "$TAURI_VERSION" || -z "$CARGO_VERSION" || -z "$DOMAIN_VERSION" || -z "$APPLICATION_VERSION" || -z "$INFRASTRUCTURE_VERSION" ]]; then
  echo "Unable to determine one or more app versions." >&2
  exit 1
fi

if [[ ! -f "$CARGO_LOCK" ]]; then
  echo "Cargo.lock is missing; release version coherence cannot be verified." >&2
  exit 1
fi

python3 - "$CARGO_LOCK" "$PACKAGE_VERSION" <<'PY'
import pathlib
import sys

try:
    import tomllib
except ModuleNotFoundError as error:
    raise SystemExit("Python 3.11+ is required to validate Cargo.lock release versions") from error

lock_path = pathlib.Path(sys.argv[1])
expected = sys.argv[2]
document = tomllib.loads(lock_path.read_text(encoding="utf-8"))
packages = document.get("package", [])
required = {
    "sbobino-application",
    "sbobino-desktop",
    "sbobino-domain",
    "sbobino-infrastructure",
}
versions = {
    package.get("name"): package.get("version")
    for package in packages
    if isinstance(package, dict) and package.get("name") in required
}
missing = sorted(required - versions.keys())
mismatched = sorted(
    f"{name}={versions[name]}"
    for name in required & versions.keys()
    if versions[name] != expected
)
if missing or mismatched:
    details = []
    if missing:
        details.append("missing=" + ",".join(missing))
    if mismatched:
        details.append("mismatched=" + ",".join(mismatched))
    raise SystemExit("Cargo.lock release package versions are not coherent: " + "; ".join(details))
PY

if [[ "$PACKAGE_VERSION" != "$TAURI_VERSION" || "$PACKAGE_VERSION" != "$CARGO_VERSION" || "$PACKAGE_VERSION" != "$DOMAIN_VERSION" || "$PACKAGE_VERSION" != "$APPLICATION_VERSION" || "$PACKAGE_VERSION" != "$INFRASTRUCTURE_VERSION" ]]; then
  echo "Version mismatch detected:" >&2
  echo "  package.json:     $PACKAGE_VERSION" >&2
  echo "  tauri.conf.json:  $TAURI_VERSION" >&2
  echo "  Cargo.toml:       $CARGO_VERSION" >&2
  echo "  domain:           $DOMAIN_VERSION" >&2
  echo "  application:      $APPLICATION_VERSION" >&2
  echo "  infrastructure:   $INFRASTRUCTURE_VERSION" >&2
  exit 1
fi

if [[ -n "$EXPECTED_VERSION" && "$PACKAGE_VERSION" != "$EXPECTED_VERSION" ]]; then
  echo "Expected version '$EXPECTED_VERSION' but found '$PACKAGE_VERSION'." >&2
  exit 1
fi

if [[ "$RELEASE_PROFILE" != "standalone-dev" ]]; then
  "$ROOT_DIR/scripts/check_updater_public_key.sh" "$TAURI_CONF"
fi

echo "Version coherence verified:"
echo "  package.json:     $PACKAGE_VERSION"
echo "  tauri.conf.json:  $TAURI_VERSION"
echo "  Cargo.toml:       $CARGO_VERSION"
echo "  domain:           $DOMAIN_VERSION"
echo "  application:      $APPLICATION_VERSION"
echo "  infrastructure:   $INFRASTRUCTURE_VERSION"
echo "  Cargo.lock:        $PACKAGE_VERSION"
