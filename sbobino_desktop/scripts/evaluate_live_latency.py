#!/usr/bin/env python3
"""Validate the platform-neutral Sbobino live-latency report contract."""

from __future__ import annotations

import argparse
import json
import math
import statistics
import sys
from pathlib import Path
from typing import Any, Sequence


def percentile(values: Sequence[float], percentile_value: float) -> float:
    if not values:
        return math.inf
    ordered = sorted(values)
    rank = (len(ordered) - 1) * percentile_value
    lower = math.floor(rank)
    upper = math.ceil(rank)
    if lower == upper:
        return ordered[lower]
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (rank - lower)


def evaluate(payload: dict[str, Any], max_latency: float, max_rss_growth_mib: float) -> dict[str, Any]:
    samples = payload.get("samples") or []
    failures: list[str] = []
    latency = []
    backlog = []
    last_captured = 0.0
    last_processed = 0.0
    for index, sample in enumerate(samples):
        captured = float(sample.get("captured_seconds", 0.0))
        processed = float(sample.get("processed_seconds", 0.0))
        reported_backlog = float(sample.get("backlog_seconds", max(0.0, captured - processed)))
        if captured + 1e-6 < last_captured or processed + 1e-6 < last_processed:
            failures.append(f"sample {index} has non-monotonic audio cursors")
        last_captured = max(last_captured, captured)
        last_processed = max(last_processed, processed)
        backlog.append(reported_backlog)
        if sample.get("preview_latency_seconds") is not None:
            latency.append(float(sample["preview_latency_seconds"]))

    p95_latency = percentile(latency, 0.95)
    p95_backlog = percentile(backlog, 0.95)
    first_preview = float(payload.get("first_preview_seconds", math.inf))
    finalization = float(payload.get("finalization_seconds", math.inf))
    rss = [float(value) for value in payload.get("rss_samples_mib") or []]
    rss_growth = max(0.0, (max(rss[-5:]) if rss else 0.0) - (statistics.median(rss[:5]) if rss else 0.0))

    if not samples:
        failures.append("report has no telemetry samples")
    if not latency:
        failures.append("report has no preview-latency samples")
    if first_preview > max_latency:
        failures.append(f"first preview {first_preview:.3f}s exceeds {max_latency:.3f}s")
    if p95_latency > max_latency:
        failures.append(f"preview P95 {p95_latency:.3f}s exceeds {max_latency:.3f}s")
    if p95_backlog > max_latency:
        failures.append(f"backlog P95 {p95_backlog:.3f}s exceeds {max_latency:.3f}s")
    if finalization > max_latency:
        failures.append(f"finalization {finalization:.3f}s exceeds {max_latency:.3f}s")
    for name in ("dropped_samples", "missing_segments", "duplicate_segments"):
        if int(payload.get(name, 0)) != 0:
            failures.append(f"{name} must be zero")
    if rss_growth > max_rss_growth_mib:
        failures.append(f"RSS growth {rss_growth:.1f} MiB exceeds {max_rss_growth_mib:.1f} MiB")

    return {
        "schema_version": 1,
        "status": "passed" if not failures else "failed",
        "engine": payload.get("engine"),
        "platform": payload.get("platform"),
        "metrics": {
            "first_preview_seconds": first_preview if math.isfinite(first_preview) else None,
            "preview_latency_p95_seconds": round(p95_latency, 6) if math.isfinite(p95_latency) else None,
            "backlog_p95_seconds": round(p95_backlog, 6),
            "finalization_seconds": finalization if math.isfinite(finalization) else None,
            "rss_growth_mib": round(rss_growth, 3),
        },
        "failures": failures,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("input", type=Path)
    parser.add_argument("--report", type=Path)
    parser.add_argument("--max-latency-seconds", type=float, default=2.0)
    parser.add_argument("--max-rss-growth-mib", type=float, default=256.0)
    args = parser.parse_args()
    payload = json.loads(args.input.read_text(encoding="utf-8"))
    report = evaluate(payload, args.max_latency_seconds, args.max_rss_growth_mib)
    body = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(body, encoding="utf-8")
    else:
        sys.stdout.write(body)
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
