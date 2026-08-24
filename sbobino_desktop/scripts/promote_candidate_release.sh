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

TAG_COMMIT_SHA=$(gh api "repos/$REPO_SLUG/commits/$TAG" --jq '.sha')
if [[ ! "$TAG_COMMIT_SHA" =~ ^[0-9a-fA-F]{40}$ ]]; then
  echo "Stable promotion blocked: could not resolve the candidate tag to one commit." >&2
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
    # Operational update/setup manifests are JSON too, but they are not proof
    # channels and are required by every installable candidate.
    "latest.json",
    "setup-manifest.json",
    "runtime-manifest.json",
    "pyannote-manifest.json",
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
    details = ["Stable promotion blocked: public JSON assets must be the reviewed seven proofs plus four operational manifests."]
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

python3 - <<'PY' "$TMP_DIR" "$VERSION" "$TAG" "$TAG_COMMIT_SHA" "$REPO_SLUG"
import math
import json
import pathlib
import re
import sys

report_dir = pathlib.Path(sys.argv[1])
version = sys.argv[2]
tag = sys.argv[3]
tag_commit_sha = sys.argv[4].lower()
repo_slug = sys.argv[5]

def load_json(path: pathlib.Path, label: str) -> dict:
    if not path.is_file():
        raise SystemExit(f"Stable promotion blocked: could not download {label}.")
    return json.loads(path.read_text(encoding="utf-8"))

def validate_revision(source: dict, label: str) -> None:
    if str(source.get("commit_sha", "")).strip().lower() != tag_commit_sha:
        raise SystemExit(f"Stable promotion blocked: {label} commit_sha does not match the candidate tag.")
    if str(source.get("repo_slug", "")).strip().lower() != repo_slug.lower():
        raise SystemExit(f"Stable promotion blocked: {label} repo_slug mismatch.")

readiness = load_json(report_dir / "release-readiness-proof.json", "release-readiness-proof.json")
validate_revision(readiness, "release-readiness-proof.json")
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
validate_revision(distribution, "distribution-readiness-proof.json")
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

def validate_quality_gate(key: str, label: str, source: dict | None = None) -> dict:
    report = (source or distribution).get(key)
    if not isinstance(report, dict):
        raise SystemExit(f"Stable promotion blocked: distribution proof is missing {label} results.")
    if int(report.get("schema_version", 0)) != 1:
        raise SystemExit(f"Stable promotion blocked: {label} report has unsupported schema_version.")
    if str(report.get("status", "")).strip().lower() != "passed":
        raise SystemExit(f"Stable promotion blocked: {label} report did not pass.")
    validate_revision(report, label)
    if report.get("version") != version or report.get("release_tag") != tag:
        raise SystemExit(f"Stable promotion blocked: {label} report version/tag mismatch.")
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

def validate_live_recovery(source: dict, label: str) -> None:
    recovery = source.get("backlog_recovery")
    if not isinstance(recovery, dict):
        raise SystemExit(f"Stable promotion blocked: {label} is missing backlog recovery evidence.")
    if str(recovery.get("status", "")).strip().lower() != "passed":
        raise SystemExit(f"Stable promotion blocked: {label} backlog recovery did not pass.")
    if recovery.get("live_mode") != "backlog-recovery":
        raise SystemExit(f"Stable promotion blocked: {label} recovery mode provenance is invalid.")
    if recovery.get("backlog_recovery_expected") is not True:
        raise SystemExit(f"Stable promotion blocked: {label} recovery expectation provenance is missing.")
    if recovery.get("preflight_rejection_expected") is not False or recovery.get("preflight_rejected") is not False:
        raise SystemExit(f"Stable promotion blocked: {label} recovery was confused with preflight rejection.")
    captured = recovery.get("captured_audio_frames")
    saved = recovery.get("saved_audio_frames")
    if not isinstance(captured, int) or captured <= 0 or saved != captured:
        raise SystemExit(
            f"Stable promotion blocked: {label} backlog recovery did not preserve every captured frame."
        )
    if int(recovery.get("dropped_samples", -1)) != 0:
        raise SystemExit(f"Stable promotion blocked: {label} backlog recovery dropped audio.")
    reaction = recovery.get("backlog_reaction_seconds")
    if not isinstance(reaction, (int, float)) or not math.isfinite(float(reaction)) or float(reaction) > 0.05:
        raise SystemExit(f"Stable promotion blocked: {label} backlog recovery reacted too late.")

def validate_live_duration(source: dict, label: str) -> None:
    duration = source.get("duration_seconds")
    if not isinstance(duration, (int, float)) or not math.isfinite(float(duration)) or float(duration) < 900:
        raise SystemExit(f"Stable promotion blocked: {label} did not attest a 900-second live run.")

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
if distribution["live_latency"].get("live_mode") != "realtime" or distribution["live_latency"].get("realtime_capable") is not True:
    raise SystemExit("Stable promotion blocked: ARM64 live proof must demonstrate realtime transcription.")
validate_live_recovery(distribution["live_latency"], "live-latency")
validate_live_duration(distribution["live_latency"], "live-latency")
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
intel_live_metrics = validate_quality_gate(
    "live_latency", "Intel live-latency", intel_distribution
)
validate_revision(intel_distribution, "intel-distribution-readiness-proof.json")
if intel_distribution["live_latency"].get("live_mode") != "realtime" or intel_distribution["live_latency"].get("realtime_capable") is not True:
    raise SystemExit("Stable promotion blocked: Intel live proof must demonstrate realtime transcription.")
validate_live_recovery(intel_distribution["live_latency"], "Intel live-latency")
validate_live_duration(intel_distribution["live_latency"], "Intel live-latency")
for metric, maximum in (
    ("first_preview_seconds", 2.0),
    ("preview_latency_p95_seconds", 2.0),
    ("backlog_p95_seconds", 2.0),
    ("finalization_seconds", 2.0),
    ("rss_growth_mib", 256.0),
):
    value = intel_live_metrics.get(metric)
    if not isinstance(value, (int, float)) or not math.isfinite(float(value)) or float(value) > maximum:
        raise SystemExit(
            f"Stable promotion blocked: Intel live-latency {metric} exceeds the release threshold."
        )
for metric in ("dropped_samples", "missing_segments", "duplicate_segments"):
    if int(intel_live_metrics.get(metric, -1)) != 0:
        raise SystemExit(f"Stable promotion blocked: Intel live-latency {metric} is non-zero.")
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
validate_revision(windows_distribution, "windows-distribution-readiness-proof.json")
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
windows_live_metrics = validate_quality_gate(
    "live_latency", "Windows live-latency", windows_distribution
)
windows_live = windows_distribution["live_latency"]
validate_live_recovery(windows_live, "Windows live-latency")
validate_live_duration(windows_live, "Windows live-latency")
windows_live_mode = windows_live.get("live_mode")
if windows_live_mode not in {"realtime", "preflight-rejected-incompatible-cpu"}:
    raise SystemExit("Stable promotion blocked: Windows live proof has an unknown mode.")
if windows_live_mode == "preflight-rejected-incompatible-cpu":
    preflight = windows_live.get("preflight")
    requested_duration = windows_live.get("requested_duration_seconds")
    captured_duration = windows_live.get("captured_duration_seconds")
    if windows_live.get("realtime_capable") is not False:
        raise SystemExit("Stable promotion blocked: Windows CPU preflight did not attest realtime incompatibility.")
    if windows_live.get("preflight_rejected") is not True or not isinstance(preflight, dict):
        raise SystemExit("Stable promotion blocked: Windows CPU preflight rejection evidence is missing.")
    if str(preflight.get("status", "")).strip().lower() != "rejected":
        raise SystemExit("Stable promotion blocked: Windows CPU preflight status is not rejected.")
    inference_ms = preflight.get("inference_ms")
    budget_ms = preflight.get("budget_ms")
    if (
        not isinstance(inference_ms, (int, float))
        or not isinstance(budget_ms, (int, float))
        or not math.isfinite(float(inference_ms))
        or not math.isfinite(float(budget_ms))
        or float(budget_ms) <= 0
        or float(inference_ms) <= float(budget_ms)
    ):
        raise SystemExit("Stable promotion blocked: Windows CPU preflight did not exceed its measured realtime budget.")
    if (
        not isinstance(requested_duration, (int, float))
        or not math.isfinite(float(requested_duration))
        or float(requested_duration) < 900
    ):
        raise SystemExit("Stable promotion blocked: Windows CPU preflight did not use the 900-second release profile.")
    if not isinstance(captured_duration, (int, float)) or float(captured_duration) != 0.0:
        raise SystemExit("Stable promotion blocked: Windows CPU preflight started capture before rejection.")
else:
    if windows_live.get("realtime_capable") is not True:
        raise SystemExit("Stable promotion blocked: Windows realtime proof did not attest capability.")
    for metric, maximum in (
        ("first_preview_seconds", 2.0),
        ("preview_latency_p95_seconds", 2.0),
        ("backlog_p95_seconds", 2.0),
        ("finalization_seconds", 2.0),
        ("rss_growth_mib", 256.0),
    ):
        value = windows_live_metrics.get(metric)
        if not isinstance(value, (int, float)) or not math.isfinite(float(value)) or float(value) > maximum:
            raise SystemExit(
                f"Stable promotion blocked: Windows live-latency {metric} exceeds the release threshold."
            )
for metric in ("dropped_samples", "missing_segments", "duplicate_segments"):
    if int(windows_live_metrics.get(metric, -1)) != 0:
        raise SystemExit(f"Stable promotion blocked: Windows live-latency {metric} is non-zero.")

windows_gui = load_json(
    report_dir / "windows-gui-smoke-report.json",
    "windows-gui-smoke-report.json",
)
validate_revision(windows_gui, "windows-gui-smoke-report.json")
if windows_gui.get("version") != version or windows_gui.get("release_tag") != tag:
    raise SystemExit("Stable promotion blocked: Windows GUI smoke version/tag mismatch.")
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
validate_revision(portability, "portability-smoke-report.json")
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
validate_revision(intel_portability, "intel-portability-smoke-report.json")
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
validate_revision(intel_pyannote, "Intel CPU/Pyannote smoke")
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
