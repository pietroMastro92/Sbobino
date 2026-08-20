#!/usr/bin/env python3
"""Evaluate an ASR timeline against a private or redistributable reference.

Both inputs use the same small JSON contract:
{
  "duration_seconds": 12.3,
  "segments": [
    {"start_seconds": 0.0, "end_seconds": 2.0,
     "language_code": "it", "text": "ciao mondo"}
  ]
}

The evaluator deliberately has no third-party dependencies so it can run in
release jobs and against private fixtures kept outside the repository.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
import unicodedata
from pathlib import Path
from typing import Any, Iterable, Sequence


SERVICE_TAG = re.compile(r"(?:\x1b\[[0-?]*[ -/]*[@-~]|<\/?[a-z]{2}(?:-[A-Z]{2})?>|fallback CPU-safe mode)", re.IGNORECASE)
TOKEN = re.compile(r"[^\W\d_]+(?:['’][^\W\d_]+)?", re.UNICODE)


def normalize_text(value: str) -> str:
    value = unicodedata.normalize("NFKC", value).casefold().replace("’", "'")
    return " ".join(TOKEN.findall(value))


def edit_distance(left: Sequence[str], right: Sequence[str]) -> int:
    if len(left) > len(right):
        left, right = right, left
    previous = list(range(len(left) + 1))
    for row, right_item in enumerate(right, 1):
        current = [row]
        for column, left_item in enumerate(left, 1):
            current.append(
                min(
                    current[-1] + 1,
                    previous[column] + 1,
                    previous[column - 1] + (left_item != right_item),
                )
            )
        previous = current
    return previous[-1]


def error_rate(reference: Sequence[str], hypothesis: Sequence[str]) -> float:
    if not reference:
        return 0.0 if not hypothesis else 1.0
    return edit_distance(reference, hypothesis) / len(reference)


def load_timeline(path: Path) -> dict[str, Any]:
    payload = json.loads(path.read_text(encoding="utf-8"))
    segments = payload.get("segments")
    if not isinstance(segments, list):
        raise ValueError(f"{path}: 'segments' must be an array")
    return payload


def segment_text(segments: Iterable[dict[str, Any]]) -> str:
    return " ".join(str(segment.get("text") or "").strip() for segment in segments).strip()


def timestamp_findings(segments: Sequence[dict[str, Any]], duration: float) -> list[str]:
    findings: list[str] = []
    last_end = 0.0
    for index, segment in enumerate(segments):
        start = float(segment.get("start_seconds", 0.0))
        end = float(segment.get("end_seconds", start))
        if not (0.0 <= start <= end <= duration + 0.25):
            findings.append(f"segment {index} has invalid bounds {start:.3f}..{end:.3f}")
        if start + 0.01 < last_end:
            findings.append(f"segment {index} is non-monotonic ({start:.3f} < {last_end:.3f})")
        last_end = max(last_end, end)
    return findings


def largest_uncovered_reference_interval(
    reference: Sequence[dict[str, Any]], hypothesis: Sequence[dict[str, Any]]
) -> float:
    """Measure missing reference speech, without treating real silence as loss."""
    hypothesis_intervals = sorted(
        (float(segment.get("start_seconds", 0.0)), float(segment.get("end_seconds", 0.0)))
        for segment in hypothesis
        if normalize_text(str(segment.get("text") or ""))
    )
    largest = 0.0
    for segment in reference:
        if not normalize_text(str(segment.get("text") or "")):
            continue
        start = float(segment.get("start_seconds", 0.0))
        end = float(segment.get("end_seconds", start))
        cursor = start
        for covered_start, covered_end in hypothesis_intervals:
            if covered_end <= cursor or covered_start >= end:
                continue
            largest = max(largest, max(0.0, min(covered_start, end) - cursor))
            cursor = max(cursor, min(covered_end, end))
            if cursor >= end:
                break
        largest = max(largest, max(0.0, end - cursor))
    return largest


def language_boundaries(segments: Sequence[dict[str, Any]]) -> list[tuple[float, str]]:
    boundaries: list[tuple[float, str]] = []
    previous: str | None = None
    for segment in segments:
        language = str(segment.get("language_code") or "und").split("-", 1)[0].lower()
        if language != previous:
            boundaries.append((float(segment.get("start_seconds", 0.0)), language))
            previous = language
    return boundaries


def match_language_boundaries(
    reference: Sequence[tuple[float, str]],
    hypothesis: Sequence[tuple[float, str]],
    tolerance: float,
) -> list[str]:
    findings: list[str] = []
    for at, language in reference:
        if language == "und":
            continue
        if not any(other_language == language and abs(other_at - at) <= tolerance for other_at, other_language in hypothesis):
            findings.append(f"missing language transition to {language} near {at:.3f}s")
    return findings


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def evaluate(reference: dict[str, Any], hypothesis: dict[str, Any], args: argparse.Namespace) -> dict[str, Any]:
    ref_segments = reference["segments"]
    hyp_segments = hypothesis["segments"]
    duration = float(reference.get("duration_seconds") or hypothesis.get("duration_seconds") or 0.0)
    ref_text = normalize_text(segment_text(ref_segments))
    hyp_text = normalize_text(segment_text(hyp_segments))
    wer = error_rate(ref_text.split(), hyp_text.split())
    cer = error_rate(list(ref_text.replace(" ", "")), list(hyp_text.replace(" ", "")))
    timestamp_errors = timestamp_findings(hyp_segments, duration)
    largest_gap = largest_uncovered_reference_interval(ref_segments, hyp_segments)
    language_errors = match_language_boundaries(
        language_boundaries(ref_segments),
        language_boundaries(hyp_segments),
        args.language_tolerance_seconds,
    )
    raw_hypothesis = segment_text(hyp_segments)
    technical_tags = SERVICE_TAG.findall(raw_hypothesis)
    failures = list(timestamp_errors) + list(language_errors)
    if args.require_reviewed_reference and reference.get("review_status") != "reviewed":
        failures.append("reference is not marked review_status=reviewed")
    if wer > args.max_wer:
        failures.append(f"WER {wer:.4f} exceeds {args.max_wer:.4f}")
    if cer > args.max_cer:
        failures.append(f"CER {cer:.4f} exceeds {args.max_cer:.4f}")
    if largest_gap > args.max_gap_seconds:
        failures.append(f"largest uncovered interval {largest_gap:.3f}s exceeds {args.max_gap_seconds:.3f}s")
    if technical_tags:
        failures.append("hypothesis contains technical/service tags")
    return {
        "schema_version": 1,
        "status": "passed" if not failures else "failed",
        "metrics": {
            "wer": round(wer, 6),
            "cer": round(cer, 6),
            "largest_uncovered_seconds": round(largest_gap, 6),
            "reference_words": len(ref_text.split()),
            "hypothesis_words": len(hyp_text.split()),
            "reference_language_boundaries": language_boundaries(ref_segments),
            "hypothesis_language_boundaries": language_boundaries(hyp_segments),
        },
        "failures": failures,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("reference", type=Path)
    parser.add_argument("hypothesis", type=Path)
    parser.add_argument("--audio", type=Path)
    parser.add_argument("--report", type=Path)
    parser.add_argument("--max-wer", type=float, default=0.35)
    parser.add_argument("--max-cer", type=float, default=0.25)
    parser.add_argument("--max-gap-seconds", type=float, default=2.0)
    parser.add_argument("--language-tolerance-seconds", type=float, default=2.0)
    parser.add_argument("--require-reviewed-reference", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    reference = load_timeline(args.reference)
    hypothesis = load_timeline(args.hypothesis)
    if args.audio:
        expected = str(reference.get("audio_sha256") or "").lower()
        actual = file_sha256(args.audio)
        if expected and expected != actual:
            raise SystemExit(f"reference audio SHA-256 mismatch: expected {expected}, got {actual}")
    report = evaluate(reference, hypothesis, args)
    body = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(body, encoding="utf-8")
    else:
        sys.stdout.write(body)
    return 0 if report["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
