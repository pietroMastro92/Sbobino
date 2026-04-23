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
  "Sbobino.app.tar.gz"
  "Sbobino.app.tar.gz.sig"
  "latest.json"
  "setup-manifest.json"
  "runtime-manifest.json"
  "speech-runtime-macos-aarch64.zip"
  "pyannote-manifest.json"
  "pyannote-runtime-macos-aarch64.zip"
  "pyannote-model-community-1.zip"
  "release-notes.md"
  "release-readiness-proof.json"
  "AS-PRIMARY.validation-report.json"
  "AS-THIRD.validation-report.json"
  "INTEL-PRIMARY.validation-report.json"
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

runtime_release = runtime_assets.get("speech_runtime_macos_aarch64")
if not isinstance(runtime_release, dict):
    raise SystemExit("runtime-manifest.json missing speech_runtime_macos_aarch64 asset.")

pyannote_runtime = pyannote_assets.get("pyannote_runtime_macos_aarch64")
pyannote_model = pyannote_assets.get("pyannote_model")
if not isinstance(pyannote_runtime, dict):
    raise SystemExit("pyannote-manifest.json missing pyannote runtime asset.")
if not isinstance(pyannote_model, dict):
    raise SystemExit("pyannote-manifest.json missing pyannote model asset.")

def assert_descriptor_matches_asset(descriptor_key: str, release_asset: dict, label: str) -> None:
    descriptor = setup.get(descriptor_key)
    if not isinstance(descriptor, dict):
        raise SystemExit(f"setup-manifest.json missing descriptor: {descriptor_key}")
    if descriptor.get("name") != release_asset.get("name"):
        raise SystemExit(f"{label} name mismatch between setup and release manifest.")
    if str(descriptor.get("sha256", "")).strip().lower() != str(
        release_asset.get("sha256", "")
    ).strip().lower():
        raise SystemExit(f"{label} checksum mismatch between setup and release manifest.")

assert_descriptor_matches_asset("runtime_asset", runtime_release, "runtime asset")
assert_descriptor_matches_asset("pyannote_runtime_asset", pyannote_runtime, "pyannote runtime asset")
assert_descriptor_matches_asset("pyannote_model_asset", pyannote_model, "pyannote model asset")

expected_reports = {
    "AS-PRIMARY.validation-report.json": {
        "machine_class": "AS-PRIMARY",
        "runner_label": "self-hosted,macos,apple-silicon,as-primary",
        "required_scenarios": [
            "native_upgrade_from_legacy_baseline",
            "warm_restart",
            "functional_diarization_smoke",
        ],
        "optional_scenarios": [],
    },
    "AS-THIRD.validation-report.json": {
        "machine_class": "AS-THIRD",
        "runner_label": "self-hosted,macos,apple-silicon,as-third",
        "required_scenarios": [
            "clean_room_install",
            "warm_restart",
            "functional_diarization_smoke",
        ],
        "optional_scenarios": [],
    },
    "INTEL-PRIMARY.validation-report.json": {
        "machine_class": "INTEL-PRIMARY",
        "runner_label": "self-hosted,macos,x64,intel-primary",
        "required_scenarios": [
            "release_metadata_validation",
            "bootstrap_layer_validation",
        ],
        "optional_scenarios": [
            "arm64_binary_execution",
        ],
    },
}

for name, expectation in expected_reports.items():
    report = json.loads((asset_dir / name).read_text(encoding="utf-8"))
    if int(report.get("schema_version", 0)) != 1:
        raise SystemExit(f"{name} has an unsupported schema_version.")
    if report.get("version") != version:
        raise SystemExit(f"{name} version does not match requested release version.")
    if report.get("release_tag") != expected_tag:
        raise SystemExit(f"{name} release_tag does not match requested release tag.")
    if report.get("machine_class") != expectation["machine_class"]:
        raise SystemExit(f"{name} machine_class is incorrect.")
    if str(report.get("runner_label", "")).strip() != expectation["runner_label"]:
        raise SystemExit(f"{name} runner_label is incorrect.")
    if str(report.get("status", "")).strip().lower() != "pending":
        raise SystemExit(f"{name} must start as pending before candidate validation.")
    scenario_results = report.get("scenario_results")
    if not isinstance(scenario_results, dict):
        raise SystemExit(f"{name} is missing scenario_results.")
    required_scenarios = report.get("required_scenarios")
    if required_scenarios != expectation["required_scenarios"]:
        raise SystemExit(f"{name} required_scenarios do not match the expected matrix.")
    for scenario in expectation["required_scenarios"]:
        if str(scenario_results.get(scenario, "")).strip().lower() != "pending":
            raise SystemExit(f"{name} scenario {scenario} must start as pending.")
    for scenario in expectation["optional_scenarios"]:
        if str(scenario_results.get(scenario, "")).strip().lower() != "pending":
            raise SystemExit(f"{name} scenario {scenario} must start as pending.")
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
  "$ASSET_DIR/Sbobino.app.tar.gz" \
  "$ASSET_DIR/Sbobino.app.tar.gz.sig" \
  "$ASSET_DIR/latest.json" \
  "$ASSET_DIR/setup-manifest.json" \
  "$ASSET_DIR/speech-runtime-macos-aarch64.zip" \
  "$ASSET_DIR/runtime-manifest.json" \
  "$ASSET_DIR/pyannote-runtime-macos-aarch64.zip" \
  "$ASSET_DIR/pyannote-model-community-1.zip" \
  "$ASSET_DIR/pyannote-manifest.json" \
  "$ASSET_DIR/release-readiness-proof.json" \
  "$ASSET_DIR/AS-PRIMARY.validation-report.json" \
  "$ASSET_DIR/AS-THIRD.validation-report.json" \
  "$ASSET_DIR/INTEL-PRIMARY.validation-report.json" \
  --repo "$REPO_SLUG"

cat <<EOF
Prerelease candidate published successfully:
  repo: $REPO_SLUG
  tag:  $TAG

Next required steps:
  1. ./scripts/distribution_readiness.sh "$VERSION" "$REPO_SLUG"
  2. Generate distribution-readiness-proof.json after the remote integrity gate passes
  3. Validate the exact GitHub prerelease on AS-PRIMARY, AS-THIRD, and INTEL-PRIMARY
  4. Re-upload distribution-readiness-proof.json plus the three validation report JSON assets
  5. ./scripts/promote_candidate_release.sh "$VERSION" "$REPO_SLUG"
EOF
