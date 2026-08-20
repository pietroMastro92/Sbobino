#!/usr/bin/env python3
"""Run the deterministic ASR and live-latency gates used by candidate releases.

The fixtures are intentionally tiny, reviewed, and checked into the repository.
They are not a recording from a private evaluation corpus.  Keeping the
fixture hashes in ``release_quality_manifest.json`` makes a workflow run fail
closed if a fixture is changed without an explicit review.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace
from typing import Any, Iterable


SCRIPT_DIR = Path(__file__).resolve().parent
DEFAULT_FIXTURE_DIR = SCRIPT_DIR / "fixtures"

# These are the only JSON files that the release workflow may publish as
# public proof assets.  Keep this contract in one place so the workflow test
# and the promotion gate cannot silently drift apart.
PUBLIC_JSON_PROOF_ASSETS = frozenset(
    {
        "release-readiness-proof.json",
        "distribution-readiness-proof.json",
        "intel-distribution-readiness-proof.json",
        "windows-distribution-readiness-proof.json",
        "windows-gui-smoke-report.json",
        "portability-smoke-report.json",
        "intel-portability-smoke-report.json",
    }
)

SYNTHETIC_EVIDENCE_CLASS = "contract-self-test"
REAL_HOSTED_EVIDENCE_CLASS = "hosted-packaged-engine"


def load_json(path: Path) -> dict[str, Any]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(payload, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return payload


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def load_module(name: str, path: Path) -> Any:
    spec = importlib.util.spec_from_file_location(name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"unable to load evaluator {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def validate_public_json_proof_assets(names_or_directory: Iterable[str] | Path) -> None:
    """Require exactly the seven public JSON proof assets.

    A release may also contain installers, archives, signatures, and release
    notes.  Only JSON assets are constrained here: an unexpected public JSON
    file is treated as an unreviewed proof channel and blocks promotion.
    """

    if isinstance(names_or_directory, Path):
        names = [path.name for path in names_or_directory.iterdir() if path.is_file()]
    else:
        names = list(names_or_directory)
    public_json_names = {
        str(name).strip()
        for name in names
        if str(name).strip().lower().endswith(".json")
    }
    missing = sorted(PUBLIC_JSON_PROOF_ASSETS - public_json_names)
    unexpected = sorted(public_json_names - PUBLIC_JSON_PROOF_ASSETS)
    if missing or unexpected:
        details: list[str] = [
            "expected exactly seven public JSON proof assets",
        ]
        if missing:
            details.append("missing=" + ",".join(missing))
        if unexpected:
            details.append("unexpected=" + ",".join(unexpected))
        raise ValueError("; ".join(details))


def validate_real_hosted_quality_report(report: dict[str, Any], label: str) -> None:
    """Validate the evidence metadata required for release ASR/live reports.

    The report must come from a hosted run of the packaged engine and its
    harness.  In particular, a report generated from the repository fixtures
    is deliberately rejected even when its numerical metrics pass.
    """

    if report.get("schema_version") != 1:
        raise ValueError(f"{label} report has unsupported schema_version")
    if report.get("status") != "passed":
        raise ValueError(f"{label} report did not pass")
    if report.get("evidence_class") != REAL_HOSTED_EVIDENCE_CLASS:
        raise ValueError(f"{label} report is not hosted packaged-engine evidence")
    if report.get("real_engine") is not True:
        raise ValueError(f"{label} report did not execute a real packaged engine")
    if report.get("real_harness") is not True:
        raise ValueError(f"{label} report did not execute the release harness")
    runner = str(report.get("runner") or "").strip()
    if not runner.startswith("github-hosted "):
        raise ValueError(f"{label} report is not from a GitHub-hosted runner")
    if not str(report.get("engine") or "").strip() or str(report.get("engine")).strip().lower() == "fixture":
        raise ValueError(f"{label} report is missing a packaged engine identity")
    if not str(report.get("harness") or "").strip():
        raise ValueError(f"{label} report is missing harness identity")
    if not str(report.get("input_audio_sha256") or "").strip():
        raise ValueError(f"{label} report is missing input audio identity")
    artifact_hashes = report.get("runtime_artifact_sha256")
    if not isinstance(artifact_hashes, dict) or not artifact_hashes:
        raise ValueError(f"{label} report is missing packaged runtime hashes")
    if any(
        len(str(value).strip()) != 64
        or any(character not in "0123456789abcdefABCDEF" for character in str(value).strip())
        for value in artifact_hashes.values()
    ):
        raise ValueError(f"{label} report contains invalid packaged runtime hashes")
    failures = report.get("failures")
    if not isinstance(failures, list) or failures:
        raise ValueError(f"{label} report contains failures")
    if not isinstance(report.get("metrics"), dict):
        raise ValueError(f"{label} report is missing metrics")


def run(fixture_dir: Path, output_dir: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    manifest_path = fixture_dir / "release_quality_manifest.json"
    manifest = load_json(manifest_path)
    fixture_id = str(manifest.get("fixture_id") or "").strip()
    fixture_source = str(manifest.get("fixture_source") or "").strip()
    if not fixture_id or fixture_source != "repo-redistributable":
        raise ValueError("release quality manifest must identify a repo-redistributable fixture")

    files = manifest.get("files")
    hashes = manifest.get("sha256")
    if not isinstance(files, dict) or not isinstance(hashes, dict):
        raise ValueError("release quality manifest is missing files or sha256 mappings")

    resolved: dict[str, Path] = {}
    for key in ("asr_reference", "asr_hypothesis", "live_latency"):
        name = str(files.get(key) or "").strip()
        expected = str(hashes.get(name) or "").strip().lower()
        path = fixture_dir / name
        if not name or not path.is_file() or len(expected) != 64:
            raise ValueError(f"release quality fixture mapping is incomplete for {key}")
        actual = sha256(path)
        if actual != expected:
            raise ValueError(f"pinned fixture checksum mismatch for {name}: {actual}")
        resolved[key] = path

    reference = load_json(resolved["asr_reference"])
    hypothesis = load_json(resolved["asr_hypothesis"])
    live = load_json(resolved["live_latency"])
    for payload, label in ((reference, "reference"), (hypothesis, "hypothesis"), (live, "live")):
        if payload.get("fixture_id") != fixture_id or payload.get("fixture_source") != fixture_source:
            raise ValueError(f"{label} fixture metadata does not match the pinned manifest")

    asr_module = load_module("sbobino_evaluate_asr_reference", SCRIPT_DIR / "evaluate_asr_reference.py")
    live_module = load_module("sbobino_evaluate_live_latency", SCRIPT_DIR / "evaluate_live_latency.py")
    asr_args = SimpleNamespace(
        language_tolerance_seconds=2.0,
        require_reviewed_reference=True,
        max_wer=0.35,
        max_cer=0.25,
        max_gap_seconds=2.0,
    )
    asr_report = asr_module.evaluate(reference, hypothesis, asr_args)
    asr_report.update(
        {
            # This job is intentionally a deterministic contract test.  Its
            # output is useful for evaluator coverage but can never be folded
            # into release evidence or satisfy promotion on its own.
            "evidence_class": SYNTHETIC_EVIDENCE_CLASS,
            "real_engine": False,
            "real_harness": False,
            "engine": "fixture",
            "fixture_id": fixture_id,
            "fixture_source": fixture_source,
            "fixture_sha256": {
                key: sha256(path) for key, path in resolved.items()
            },
        }
    )
    live_report = live_module.evaluate(live, max_latency=2.0, max_rss_growth_mib=256.0)
    live_report.setdefault("metrics", {}).update(
        {
            "dropped_samples": int(live.get("dropped_samples", 0)),
            "missing_segments": int(live.get("missing_segments", 0)),
            "duplicate_segments": int(live.get("duplicate_segments", 0)),
        }
    )
    live_report.update(
        {
            "evidence_class": SYNTHETIC_EVIDENCE_CLASS,
            "real_engine": False,
            "real_harness": False,
            "engine": "fixture",
            "fixture_id": fixture_id,
            "fixture_source": fixture_source,
            "fixture_sha256": {"live_latency": sha256(resolved["live_latency"])},
        }
    )

    output_dir.mkdir(parents=True, exist_ok=True)
    (output_dir / "asr-reference-report.json").write_text(
        json.dumps(asr_report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    (output_dir / "live-latency-report.json").write_text(
        json.dumps(live_report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    if asr_report.get("status") != "passed" or live_report.get("status") != "passed":
        raise SystemExit("release ASR/live quality gate failed; candidate remains unpromotable")
    return asr_report, live_report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output-dir",
        type=Path,
        help="write synthetic contract-test reports to this directory",
    )
    parser.add_argument(
        "--validate-public-proof-assets",
        type=Path,
        help="validate the JSON names in a release validation-assets directory",
    )
    parser.add_argument("--fixture-dir", type=Path, default=DEFAULT_FIXTURE_DIR)
    args = parser.parse_args()
    if args.validate_public_proof_assets is not None:
        validate_public_json_proof_assets(args.validate_public_proof_assets.resolve())
        return 0
    if args.output_dir is None:
        parser.error("--output-dir is required unless --validate-public-proof-assets is used")
    run(args.fixture_dir.resolve(), args.output_dir.resolve())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
