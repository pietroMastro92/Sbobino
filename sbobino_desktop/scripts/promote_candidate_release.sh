#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: promote_candidate_release.sh <version> [repo-slug]

Promotes a previously validated GitHub prerelease candidate to stable and
keeps the latest two stable releases available for rollback by default.

Set SBOBINO_PROMOTION_DRY_RUN=1 to validate the public proof assets without
changing the release state.
EOF
}

if [[ $# -lt 1 || $# -gt 2 ]]; then
  usage
  exit 1
fi

VERSION=$1
REPO_SLUG=${2:-pietroMastro92/Sbobino}
TAG="v$VERSION"

need_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Missing required command: $1" >&2
    exit 1
  fi
}

need_cmd gh
need_cmd python3

RELEASE_JSON=$(gh release view "$TAG" --repo "$REPO_SLUG" --json assets,isPrerelease,name,tagName,url)
if [[ -z "$RELEASE_JSON" ]]; then
  echo "Release $TAG was not found in $REPO_SLUG." >&2
  exit 1
fi

IS_PRERELEASE=$(python3 - <<'PY' "$RELEASE_JSON"
import json, sys
print("1" if json.loads(sys.argv[1]).get("isPrerelease") else "0")
PY
)

if [[ "$IS_PRERELEASE" != "1" ]]; then
  echo "Release $TAG is already stable. Only validated prereleases can be promoted." >&2
  exit 1
fi

python3 - <<'PY' "$RELEASE_JSON" "$VERSION"
import json
import pathlib
import sys

release = json.loads(sys.argv[1])
version = sys.argv[2]
expected_json_assets = {
    "release-readiness-proof.json",
    "distribution-readiness-proof.json",
    "intel-distribution-readiness-proof.json",
    "windows-distribution-readiness-proof.json",
    "windows-gui-smoke-report.json",
    "portability-smoke-report.json",
    "intel-portability-smoke-report.json",
}
required_non_json_assets = {"release-notes.md"}
present_assets = {
    asset.get("name", "").strip()
    for asset in release.get("assets", [])
    if isinstance(asset, dict)
}
present_json_assets = {
    name for name in present_assets if pathlib.PurePosixPath(name).suffix.lower() == ".json"
}
missing_json = sorted(expected_json_assets - present_json_assets)
unexpected_json = sorted(present_json_assets - expected_json_assets)
missing_non_json = sorted(required_non_json_assets - present_assets)
if missing_json or unexpected_json:
    details = ["Stable promotion blocked: public JSON proof assets must be exactly the reviewed seven names."]
    if missing_json:
        details.append("missing=" + ",".join(missing_json))
    if unexpected_json:
        details.append("unexpected=" + ",".join(unexpected_json))
    raise SystemExit(" ".join(details))
if missing_non_json:
    raise SystemExit(
        "Stable promotion blocked: missing required release metadata assets: "
        + ", ".join(missing_non_json)
    )
if release.get("tagName") != f"v{version}":
    raise SystemExit("Release tag does not match the requested version.")
PY

TMP_DIR=$(mktemp -d)
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

gh release download "$TAG" \
  --repo "$REPO_SLUG" \
  --dir "$TMP_DIR" \
  --pattern "release-readiness-proof.json" \
  --pattern "distribution-readiness-proof.json" \
  --pattern "intel-distribution-readiness-proof.json" \
  --pattern "windows-distribution-readiness-proof.json" \
  --pattern "windows-gui-smoke-report.json" \
  --pattern "portability-smoke-report.json" \
  --pattern "intel-portability-smoke-report.json" \
  --pattern "release-notes.md"

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
"$ROOT_DIR/scripts/check_release_notes.sh" \
  "$VERSION" \
  "$TMP_DIR/release-notes.md"

python3 - <<'PY' "$TMP_DIR" "$VERSION" "$TAG"
import math
import json
import pathlib
import re
import sys

report_dir = pathlib.Path(sys.argv[1])
version = sys.argv[2]
tag = sys.argv[3]

def load_json(path: pathlib.Path, label: str) -> dict:
    if not path.is_file():
        raise SystemExit(f"Stable promotion blocked: could not download {label}.")
    return json.loads(path.read_text(encoding="utf-8"))

readiness = load_json(report_dir / "release-readiness-proof.json", "release-readiness-proof.json")
if readiness.get("version") != version:
    raise SystemExit("Stable promotion blocked: release-readiness-proof.json version mismatch.")
if str(readiness.get("status", "")).strip().lower() != "passed":
    raise SystemExit("Stable promotion blocked: release-readiness-proof.json is not marked passed.")
if str(readiness.get("gate", "")).strip() != "release_readiness.sh":
    raise SystemExit("Stable promotion blocked: release-readiness-proof.json gate mismatch.")

distribution = load_json(
    report_dir / "distribution-readiness-proof.json",
    "distribution-readiness-proof.json",
)
if int(distribution.get("schema_version", 0)) != 1:
    raise SystemExit(
        "Stable promotion blocked: distribution-readiness-proof.json has unsupported schema_version."
    )
if distribution.get("version") != version:
    raise SystemExit("Stable promotion blocked: distribution-readiness-proof.json version mismatch.")
if distribution.get("release_tag") != tag:
    raise SystemExit("Stable promotion blocked: distribution-readiness-proof.json release_tag mismatch.")
if str(distribution.get("status", "")).strip().lower() != "passed":
    raise SystemExit("Stable promotion blocked: distribution-readiness-proof.json is not marked passed.")
if str(distribution.get("gate", "")).strip() != "distribution_readiness.sh":
    raise SystemExit("Stable promotion blocked: distribution-readiness-proof.json gate mismatch.")

def validate_quality_gate(key: str, label: str) -> dict:
    report = distribution.get(key)
    if not isinstance(report, dict):
        raise SystemExit(f"Stable promotion blocked: distribution proof is missing {label} results.")
    if int(report.get("schema_version", 0)) != 1:
        raise SystemExit(f"Stable promotion blocked: {label} report has unsupported schema_version.")
    if str(report.get("status", "")).strip().lower() != "passed":
        raise SystemExit(f"Stable promotion blocked: {label} report did not pass.")
    if report.get("evidence_class") != "hosted-packaged-engine":
        raise SystemExit(
            f"Stable promotion blocked: {label} is not real hosted packaged-engine evidence."
        )
    if report.get("real_engine") is not True or report.get("real_harness") is not True:
        raise SystemExit(
            f"Stable promotion blocked: {label} did not execute a real packaged engine/harness."
        )
    if not str(report.get("runner", "")).strip().startswith("github-hosted "):
        raise SystemExit(f"Stable promotion blocked: {label} is not from a hosted runner.")
    if not str(report.get("engine", "")).strip() or str(report.get("engine")).strip().lower() == "fixture":
        raise SystemExit(f"Stable promotion blocked: {label} is missing a packaged engine identity.")
    if not str(report.get("harness", "")).strip():
        raise SystemExit(f"Stable promotion blocked: {label} report is missing harness identity.")
    if not re.fullmatch(r"[0-9a-fA-F]{64}", str(report.get("input_audio_sha256", "")).strip()):
        raise SystemExit(f"Stable promotion blocked: {label} report is missing an input audio hash.")
    runtime_hashes = report.get("runtime_artifact_sha256")
    if not isinstance(runtime_hashes, dict) or not runtime_hashes or any(
        not re.fullmatch(r"[0-9a-fA-F]{64}", str(value).strip())
        for value in runtime_hashes.values()
    ):
        raise SystemExit(f"Stable promotion blocked: {label} report is missing packaged runtime hashes.")
    failures = report.get("failures")
    if not isinstance(failures, list) or failures:
        raise SystemExit(f"Stable promotion blocked: {label} report contains failures.")
    metrics = report.get("metrics")
    if not isinstance(metrics, dict):
        raise SystemExit(f"Stable promotion blocked: {label} report is missing metrics.")
    return metrics

asr_metrics = validate_quality_gate("asr_reference", "ASR reference")
for metric, maximum in (
    ("wer", 0.35),
    ("cer", 0.25),
    ("largest_uncovered_seconds", 2.0),
):
    value = asr_metrics.get(metric)
    if not isinstance(value, (int, float)) or not math.isfinite(float(value)) or float(value) > maximum:
        raise SystemExit(
            f"Stable promotion blocked: ASR reference {metric} exceeds the release threshold."
        )

live_metrics = validate_quality_gate("live_latency", "live-latency")
for metric, maximum in (
    ("first_preview_seconds", 2.0),
    ("preview_latency_p95_seconds", 2.0),
    ("backlog_p95_seconds", 2.0),
    ("finalization_seconds", 2.0),
    ("rss_growth_mib", 256.0),
):
    value = live_metrics.get(metric)
    if not isinstance(value, (int, float)) or not math.isfinite(float(value)) or float(value) > maximum:
        raise SystemExit(
            f"Stable promotion blocked: live-latency {metric} exceeds the release threshold."
        )
for metric in ("dropped_samples", "missing_segments", "duplicate_segments"):
    if int(live_metrics.get(metric, -1)) != 0:
        raise SystemExit(f"Stable promotion blocked: live-latency {metric} is non-zero.")

intel_distribution = load_json(
    report_dir / "intel-distribution-readiness-proof.json",
    "intel-distribution-readiness-proof.json",
)
if int(intel_distribution.get("schema_version", 0)) != 1:
    raise SystemExit(
        "Stable promotion blocked: intel-distribution-readiness-proof.json has unsupported schema_version."
    )
if intel_distribution.get("version") != version or intel_distribution.get("release_tag") != tag:
    raise SystemExit("Stable promotion blocked: Intel distribution readiness proof version mismatch.")
if str(intel_distribution.get("status", "")).strip().lower() != "passed":
    raise SystemExit("Stable promotion blocked: Intel distribution readiness proof is not marked passed.")
if str(intel_distribution.get("gate", "")).strip() != "distribution_readiness.sh":
    raise SystemExit("Stable promotion blocked: Intel distribution readiness proof gate mismatch.")

windows_distribution = load_json(
    report_dir / "windows-distribution-readiness-proof.json",
    "windows-distribution-readiness-proof.json",
)
if int(windows_distribution.get("schema_version", 0)) != 1:
    raise SystemExit(
        "Stable promotion blocked: windows-distribution-readiness-proof.json has unsupported schema_version."
    )
if windows_distribution.get("version") != version or windows_distribution.get("release_tag") != tag:
    raise SystemExit("Stable promotion blocked: Windows distribution readiness proof version mismatch.")
if str(windows_distribution.get("status", "")).strip().lower() != "passed":
    raise SystemExit("Stable promotion blocked: Windows distribution readiness proof is not marked passed.")
if windows_distribution.get("platform") != "windows" or windows_distribution.get("architecture") != "x86_64":
    raise SystemExit("Stable promotion blocked: Windows distribution readiness proof target mismatch.")

windows_gui = load_json(
    report_dir / "windows-gui-smoke-report.json",
    "windows-gui-smoke-report.json",
)
if int(windows_gui.get("schema_version", 0)) != 1:
    raise SystemExit("Stable promotion blocked: Windows GUI smoke report schema mismatch.")
if str(windows_gui.get("status", "")).strip().lower() != "passed":
    raise SystemExit("Stable promotion blocked: Windows GUI smoke report is not marked passed.")
if windows_gui.get("platform") != "windows":
    raise SystemExit("Stable promotion blocked: Windows GUI smoke report platform mismatch.")
if int(windows_gui.get("visible_console_windows", -1)) != 0:
    raise SystemExit("Stable promotion blocked: Windows GUI smoke detected visible console windows.")
if int(windows_gui.get("main_window_count", -1)) != 1:
    raise SystemExit("Stable promotion blocked: Windows GUI smoke did not find exactly one main window.")
if windows_gui.get("main_window_opaque") is not True:
    raise SystemExit("Stable promotion blocked: Windows main window is not opaque.")
if windows_gui.get("macos_transparency_preserved") is not True:
    raise SystemExit("Stable promotion blocked: shared macOS transparency contract changed.")
if set(windows_gui.get("helper_probes") or []) != {"ffmpeg", "whisper", "parakeet", "python"}:
    raise SystemExit("Stable promotion blocked: Windows GUI smoke did not exercise every real helper.")

for field in (
    "visible_console_windows",
    "main_window_count",
    "main_window_opaque",
    "macos_transparency_preserved",
):
    if windows_distribution.get(field) != windows_gui.get(field):
        raise SystemExit(
            f"Stable promotion blocked: Windows distribution proof disagrees with GUI smoke for {field}."
        )

portability = load_json(
    report_dir / "portability-smoke-report.json",
    "portability-smoke-report.json",
)
if int(portability.get("schema_version", 0)) != 1:
    raise SystemExit(
        "Stable promotion blocked: portability-smoke-report.json has unsupported schema_version."
    )
if portability.get("version") != version:
    raise SystemExit("Stable promotion blocked: portability-smoke-report.json version mismatch.")
if portability.get("release_tag") != tag:
    raise SystemExit("Stable promotion blocked: portability-smoke-report.json release_tag mismatch.")
if str(portability.get("status", "")).strip().lower() != "passed":
    raise SystemExit("Stable promotion blocked: portability-smoke-report.json is not marked passed.")

intel_portability = load_json(
    report_dir / "intel-portability-smoke-report.json",
    "intel-portability-smoke-report.json",
)
if int(intel_portability.get("schema_version", 0)) != 1:
    raise SystemExit(
        "Stable promotion blocked: intel-portability-smoke-report.json has unsupported schema_version."
    )
if intel_portability.get("version") != version or intel_portability.get("release_tag") != tag:
    raise SystemExit("Stable promotion blocked: Intel portability smoke report version mismatch.")
if str(intel_portability.get("status", "")).strip().lower() != "passed":
    raise SystemExit("Stable promotion blocked: Intel portability smoke report is not marked passed.")

intel_pyannote = intel_distribution.get("intel_pyannote_parakeet_smoke")
if not isinstance(intel_pyannote, dict):
    raise SystemExit(
        "Stable promotion blocked: Intel distribution proof is missing the CPU/Pyannote smoke fields."
    )
if int(intel_pyannote.get("schema_version", 0)) != 1:
    raise SystemExit(
        "Stable promotion blocked: Intel CPU/Pyannote smoke fields have unsupported schema_version."
    )
if str(intel_pyannote.get("status", "")).strip().lower() != "passed":
    raise SystemExit("Stable promotion blocked: Intel CPU/Pyannote smoke did not pass.")
if intel_pyannote.get("runner") != "github-hosted macos-15-intel":
    raise SystemExit("Stable promotion blocked: Intel smoke did not use macos-15-intel.")
if intel_pyannote.get("machine_class") != "HOSTED-CLEANROOM-STANDARD":
    raise SystemExit("Stable promotion blocked: Intel smoke machine class is not standard hosted.")
if float(intel_pyannote.get("parakeet_duration_seconds", 0)) <= 60:
    raise SystemExit("Stable promotion blocked: Intel Parakeet smoke did not exceed 60 seconds of audio.")
if str(intel_pyannote.get("parakeet_compute_device", "")).strip().lower() != "cpu":
    raise SystemExit("Stable promotion blocked: Intel Parakeet smoke was not CPU-only.")
if str(intel_pyannote.get("parakeet_language", "")).strip().lower() != "auto":
    raise SystemExit("Stable promotion blocked: Intel Parakeet smoke was not automatic-language mode.")
if intel_pyannote.get("pyannote_deep_smoke") is not True:
    raise SystemExit("Stable promotion blocked: Intel Pyannote deep smoke did not pass.")
PY

if [[ "${SBOBINO_PROMOTION_DRY_RUN:-0}" == "1" ]]; then
  cat <<EOF
Candidate promotion proof gate passed (dry run):
  repo: $REPO_SLUG
  tag:  $TAG
EOF
  exit 0
fi

gh release edit "$TAG" --repo "$REPO_SLUG" --prerelease=false --latest

STABLE_RELEASE_RETENTION=${SBOBINO_STABLE_RELEASE_RETENTION:-2}
if ! [[ "$STABLE_RELEASE_RETENTION" =~ ^[0-9]+$ ]] || [[ "$STABLE_RELEASE_RETENTION" -lt 1 ]]; then
  echo "SBOBINO_STABLE_RELEASE_RETENTION must be a positive integer." >&2
  exit 1
fi

RELEASE_LIST_JSON=$(gh release list --repo "$REPO_SLUG" --exclude-pre-releases --limit 100 --json tagName,publishedAt,isLatest)

STABLE_TAGS_TO_DELETE=$(python3 - <<'PY' "$RELEASE_LIST_JSON" "$TAG" "$STABLE_RELEASE_RETENTION"
import json
import re
import sys

releases = json.loads(sys.argv[1])
current_tag = sys.argv[2]
retention = int(sys.argv[3])

def version_key(tag: str) -> tuple[int, ...]:
    match = re.fullmatch(r"v?(\d+(?:\.\d+)*)", tag.strip())
    if not match:
        return ()
    return tuple(int(part) for part in match.group(1).split("."))

stable = []
for index, release in enumerate(releases):
    tag = str(release.get("tagName", "")).strip()
    if not tag:
        continue
    stable.append(
        {
            "tag": tag,
            "index": index,
            "version": version_key(tag),
            "published_at": str(release.get("publishedAt", "")),
        }
    )

current = next((release for release in stable if release["tag"] == current_tag), None)
if current is None:
    raise SystemExit(f"Stable retention blocked: promoted tag {current_tag} is not listed as stable.")

stable.sort(
    key=lambda release: (
        release["version"],
        release["published_at"],
        -release["index"],
    ),
    reverse=True,
)

keep = {current_tag}
for release in stable:
    if len(keep) >= retention:
        break
    keep.add(release["tag"])

for release in stable:
    if release["tag"] not in keep:
        print(release["tag"])
PY
)

if [[ -n "${STABLE_TAGS_TO_DELETE// }" ]]; then
  while IFS= read -r stable_tag; do
    [[ -z "$stable_tag" ]] && continue
    gh release delete "$stable_tag" --repo "$REPO_SLUG" --yes --cleanup-tag
  done <<<"$STABLE_TAGS_TO_DELETE"
fi

cat <<EOF
Candidate promoted to stable:
  repo: $REPO_SLUG
  tag:  $TAG

Stable release retention:
  kept:    newest $STABLE_RELEASE_RETENTION stable release(s), including $TAG
  deleted: ${STABLE_TAGS_TO_DELETE:-none}
EOF
