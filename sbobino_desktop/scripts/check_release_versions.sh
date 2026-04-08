#!/usr/bin/env bash
set -euo pipefail

if [[ $# -gt 1 ]]; then
  echo "Usage: $0 [expected-version]" >&2
  exit 1
fi

EXPECTED_VERSION=${1:-}
ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
PACKAGE_JSON="$ROOT_DIR/apps/desktop/package.json"
TAURI_CONF="$ROOT_DIR/apps/desktop/src-tauri/tauri.conf.json"
CARGO_TOML="$ROOT_DIR/apps/desktop/src-tauri/Cargo.toml"
DOMAIN_CARGO_TOML="$ROOT_DIR/crates/domain/Cargo.toml"
APPLICATION_CARGO_TOML="$ROOT_DIR/crates/application/Cargo.toml"
INFRASTRUCTURE_CARGO_TOML="$ROOT_DIR/crates/infrastructure/Cargo.toml"

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

echo "Version coherence verified: $PACKAGE_VERSION"
