#!/usr/bin/env python3
"""Score frozen ASR spike outputs. References are read only in this process."""

import argparse
import hashlib
import html
import json
import re
import unicodedata
from pathlib import Path

from asr_spike import CORRECT, DIRECT, stitch


WORD = re.compile(r"[^\W_]+(?:['’][^\W_]+)*", re.UNICODE)
PUNCTUATION = re.compile(r"[.,!?;:]")


def words(text):
    return WORD.findall(unicodedata.normalize("NFKC", html.unescape(text)).casefold().replace("’", "'"))


def edits(reference, hypothesis):
    """Return Levenshtein counts with substitutions preferred to ins+del ties."""
    try:
        from rapidfuzz.distance import Levenshtein
        operations = Levenshtein.editops(reference, hypothesis)
        counts = {"substitutions": 0, "deletions": 0, "insertions": 0}
        for operation in operations:
            counts[{"replace": "substitutions", "delete": "deletions", "insert": "insertions"}[operation.tag]] += 1
        return counts, len(operations)
    except ImportError:
        # Keep the scorer dependency-free for small local checks. The benchmark
        # environment installs rapidfuzz so long talks use its native editops.
        pass
    previous = [(j, 0, 0, j) for j in range(len(hypothesis) + 1)]
    for i, a in enumerate(reference, 1):
        row = [(i, 0, i, 0)]
        for j, b in enumerate(hypothesis, 1):
            diagonal, deletion, insertion = previous[j - 1], previous[j], row[j - 1]
            sub = int(a != b)
            row.append(min((diagonal[0] + sub, diagonal[1] + sub, diagonal[2], diagonal[3]),
                           (deletion[0] + 1, deletion[1], deletion[2] + 1, deletion[3]),
                           (insertion[0] + 1, insertion[1], insertion[2], insertion[3] + 1),
                           key=lambda item: item[0]))
        previous = row
    cost, substitutions, deletions, insertions = previous[-1]
    return {"substitutions": substitutions, "deletions": deletions, "insertions": insertions}, cost


def distance(reference, hypothesis):
    try:
        from rapidfuzz.distance import Levenshtein
        return Levenshtein.distance(reference, hypothesis)
    except ImportError:
        # The standalone scorer still works without packages on short clips.
        return edits(reference, hypothesis)[1]


def measure(reference, hypothesis, entities=()):
    ref_words, hyp_words = words(reference), words(hypothesis)
    counts, word_errors = edits(ref_words, hyp_words)
    ref_chars = "".join(ref_words)
    hyp_chars = "".join(hyp_words)
    char_errors = distance(ref_chars, hyp_chars)
    ref_punct = PUNCTUATION.findall(reference)
    hyp_punct = PUNCTUATION.findall(hypothesis)
    punct_errors = distance(ref_punct, hyp_punct)
    ref_numbers = [token for token in ref_words if any(ch.isdigit() for ch in token)]
    hyp_numbers = [token for token in hyp_words if any(ch.isdigit() for ch in token)]
    number_errors = distance(ref_numbers, hyp_numbers)
    entity_scores = {}
    for entity in entities:
        value = entity["text"] if isinstance(entity, dict) else entity
        needle = words(value)
        def present(haystack):
            return any(haystack[i:i + len(needle)] == needle for i in range(len(haystack) - len(needle) + 1))
        entity_scores[value] = present(hyp_words) if needle else False
    return {
        "reference_words": len(ref_words), "hypothesis_words": len(hyp_words),
        "word_errors": word_errors, "wer": word_errors / len(ref_words) if ref_words else None,
        "reference_chars": len(ref_chars), "char_errors": char_errors,
        "cer": char_errors / len(ref_chars) if ref_chars else None,
        "punctuation_errors": punct_errors,
        "number_errors": number_errors,
        "possible_hallucination_on_silence": not ref_words and bool(hyp_words),
        "entity_correct": entity_scores,
        **{key: counts.get(key, 0) for key in ("substitutions", "deletions", "insertions")},
    }


def load_samples(path):
    raw = json.loads(Path(path).read_text())
    samples = raw["samples"] if isinstance(raw, dict) else raw
    if len({item["id"] for item in samples}) != len(samples):
        raise ValueError(f"duplicate sample ID in {path}")
    for item in samples:
        if "reference_sha256" in item and "reference" in item:
            actual = hashlib.sha256(item["reference"].encode()).hexdigest()
            if actual != item["reference_sha256"]:
                raise ValueError(f"reference checksum differs: {item['id']}")
    return {item["id"]: item for item in samples}


def validated_run(path, manifest):
    result = json.loads(Path(path).read_text())
    expected = hashlib.sha256(Path(manifest).read_bytes()).hexdigest()
    if result.get("configuration", {}).get("manifest_sha256") != expected:
        raise ValueError(f"run manifest checksum differs: {path}")
    samples = result["samples"]
    if len({item["sample_id"] for item in samples}) != len(samples):
        raise ValueError(f"duplicate run sample ID in {path}")
    return result


def timed_entities(reference):
    intervals = {}
    for entity in reference.get("entities", []):
        value = entity["text"] if isinstance(entity, dict) else entity
        needle = words(value)
        def contains(segment):
            tokens = words(segment["text"])
            return any(tokens[i:i + len(needle)] == needle for i in range(len(tokens) - len(needle) + 1))
        intervals[value] = [
            {"start_seconds": segment["start"], "end_seconds": segment["end"]}
            for segment in reference.get("segments", [])
            if contains(segment)
        ] if needle else []
    return intervals


def evaluation_text(sample, reference, output, overlap):
    """Use the same annotated audio span for AMI quality and paired comparisons."""
    segments = reference.get("segments", [])
    if sample.get("corpus") != "ami" or not segments:
        return reference["reference"], output["text"], None
    annotated_start = min(segment["start"] for segment in segments)
    annotated_end = max(segment["end"] for segment in segments)
    if annotated_start <= 1 and annotated_end >= sample["duration_seconds"] - 1:
        return reference["reference"], output["text"], None
    chunks = [chunk for chunk in output["chunks"]
              if chunk["start"] >= annotated_start and chunk["end"] <= annotated_end]
    crosses = lambda boundary: any(segment["start"] < boundary < segment["end"] for segment in segments)
    # ponytail: whole chunks bounded by annotation gaps; word timestamps could recover excluded edges.
    while chunks and crosses(chunks[0]["start"]):
        chunks = chunks[1:]
    while chunks and crosses(chunks[-1]["end"]):
        chunks = chunks[:-1]
    if not chunks:
        return None, None, None
    span = {"start_seconds": chunks[0]["start"], "end_seconds": chunks[-1]["end"]}
    reference_text = " ".join(segment["text"] for segment in segments
                              if segment["start"] >= span["start_seconds"] and
                              segment["end"] <= span["end_seconds"])
    if not reference_text.strip():
        return None, None, None
    return reference_text, stitch(chunks, overlap)[0], span


def score_run(manifest, references, run):
    samples = load_samples(manifest)
    gold = load_samples(references)
    results = validated_run(run, manifest)
    outputs = results["samples"] if isinstance(results, dict) else results
    scored = []
    for output in outputs:
        sample = samples[output["sample_id"]]
        instructions = (CORRECT, DIRECT.format(language={"it": "italiano", "en": "inglese"}.get(
            sample.get("language"), sample.get("language"))))
        echoes = [{"start_seconds": chunk["start"], "end_seconds": chunk["end"]}
                  for chunk in output.get("chunks", [])
                  if chunk.get("raw_text", "").strip() in instructions]
        if output.get("status") != "ok":
            scored.append({"sample_id": output["sample_id"], "status": output.get("status", "failed"),
                           "error": output.get("error"), "prompt_echo_intervals": echoes})
            continue
        reference = gold.get(output["sample_id"])
        if reference is None or "reference" not in reference:
            # Stability-only samples (for example the 90-minute sequence) are
            # intentionally absent from references.json and must not affect WER.
            scored.append({"sample_id": output["sample_id"], "status": "unscored",
                           "reason": "reference_missing", "corpus": sample.get("corpus"),
                           "language": sample.get("language"),
                           "duration_seconds": sample.get("duration_seconds"),
                           "prompt_echo_intervals": echoes,
                           "output_status": "ok"})
            continue
        reference_text, output_text, scored_interval = evaluation_text(
            sample, reference, output, results["configuration"].get("overlap", 2))
        if reference_text is None:
            scored.append({"sample_id": output["sample_id"], "status": "unscored",
                           "reason": "no_reference_safe_span", "corpus": sample.get("corpus"),
                           "language": sample.get("language"), "output_status": "ok",
                           "prompt_echo_intervals": echoes})
            continue
        row = measure(reference_text, output_text, reference.get("entities", []))
        row["entity_reference_intervals"] = timed_entities(reference)
        row["entity_local_correct"] = {}
        for entity, intervals in row["entity_reference_intervals"].items():
            if not intervals:
                continue
            needle = words(entity)
            nearby = [chunk for chunk in output["chunks"] if any(
                chunk["start"] <= interval["end_seconds"] + 2 and
                chunk["end"] >= interval["start_seconds"] - 2 for interval in intervals)]
            row["entity_local_correct"][entity] = any(
                any(tokens[i:i + len(needle)] == needle for i in range(len(tokens) - len(needle) + 1))
                for tokens in (words(chunk["raw_text"]) for chunk in nearby))
        row.update(sample_id=output["sample_id"], corpus=sample.get("corpus"), language=sample.get("language"),
                   reference_status=reference.get("reference_status"),
                   duration_seconds=sample.get("duration_seconds"), status="ok",
                   scored_interval=scored_interval,
                   prompt_echo_intervals=echoes,
                   inference_seconds=output.get("inference_seconds"), rtf=output.get("rtf"),
                   boundary_findings=output.get("boundary_findings", []))
        scored.append(row)
    successful = [item for item in scored if item["status"] == "ok"]
    total_words = sum(item["reference_words"] for item in successful)
    italian = [item for item in successful if item["language"] == "it"]
    total_chars = sum(item["reference_chars"] for item in successful)
    def totals(rows):
        return {key: sum(item[key] for item in rows) for key in
                ("word_errors", "char_errors", "substitutions", "deletions", "insertions",
                 "punctuation_errors", "number_errors")}
    entities = [correct for item in successful for correct in item["entity_correct"].values()]
    local_entities = [correct for item in successful for correct in item["entity_local_correct"].values()]
    by_corpus = {}
    for corpus in sorted({item["corpus"] for item in successful}):
        rows = [item for item in successful if item["corpus"] == corpus]
        words_total = sum(item["reference_words"] for item in rows)
        by_corpus[corpus] = {"sample_count": len(rows),
                             "word_errors": sum(item["word_errors"] for item in rows),
                             "reference_words": words_total,
                             "micro_wer": sum(item["word_errors"] for item in rows) / words_total if words_total else None}
    return {"run": str(run), "sample_count": len(scored), "success_count": len(successful),
            "quality_sample_count": len(successful),
            "unscored_count": sum(item["status"] == "unscored" for item in scored),
            "unscored_sample_ids": [item["sample_id"] for item in scored if item["status"] == "unscored"],
            "micro_wer": sum(item["word_errors"] for item in successful) / total_words if total_words else None,
            "micro_cer": sum(item["char_errors"] for item in successful) / total_chars if total_chars else None,
            "italian_micro_wer": (sum(item["word_errors"] for item in italian) / sum(item["reference_words"] for item in italian)) if italian and sum(item["reference_words"] for item in italian) else None,
            "totals": totals(successful), "by_corpus": by_corpus,
            "entity_correct_count": sum(entities), "entity_total": len(entities),
            "entity_local_correct_count": sum(local_entities), "entity_local_total": len(local_entities),
            "possible_silence_hallucinations": sum(item["possible_hallucination_on_silence"] for item in successful),
            "prompt_echo_chunk_count": sum(len(item.get("prompt_echo_intervals", [])) for item in scored),
            "startup_seconds": results.get("startup_seconds") if isinstance(results, dict) else None,
            "peak_process_rss_bytes": results.get("peak_process_rss_bytes") if isinstance(results, dict) else None,
            "mlx_peak_gpu_allocation_bytes": results.get("mlx_peak_gpu_allocation_bytes") if isinstance(results, dict) else None,
            "mps_observed_driver_allocation_bytes": results.get("mps_observed_driver_allocation_bytes") if isinstance(results, dict) else None,
            "configuration": results.get("configuration") if isinstance(results, dict) else None,
            "samples": scored}


def compare_runs(manifest, references, baseline_path, corrected_path, text_only_path=None):
    samples = load_samples(manifest)
    gold = load_samples(references)
    baseline_run = validated_run(baseline_path, manifest)
    corrected_run = validated_run(corrected_path, manifest)
    baseline_sha = hashlib.sha256(Path(baseline_path).read_bytes()).hexdigest()
    def source_matches(payload):
        config = payload["configuration"]
        recorded_sha = config.get("source_run_sha256")
        return (recorded_sha == baseline_sha if recorded_sha else
                Path(config.get("source_run", "")).resolve() == Path(baseline_path).resolve())
    if not source_matches(corrected_run):
        raise ValueError("correction was produced from a different baseline run")
    def outputs(path):
        payload = validated_run(path, manifest)
        if not source_matches(payload):
            raise ValueError("text-only control was produced from a different baseline run")
        return {item["sample_id"]: item for item in payload["samples"]}
    baseline = {item["sample_id"]: item for item in baseline_run["samples"]}
    corrected = {item["sample_id"]: item for item in corrected_run["samples"]}
    text_only = outputs(text_only_path) if text_only_path else {}
    records = []
    unscored_sample_ids = []
    failed_correction_sample_ids = []
    fallback_word_errors = fallback_reference_words = 0
    for sample_id, after in corrected.items():
        before = baseline.get(sample_id)
        if not before or before["status"] != "ok":
            continue
        reference_record = gold.get(sample_id)
        if reference_record is None or "reference" not in reference_record:
            unscored_sample_ids.append(sample_id)
            continue
        overlap = baseline_run["configuration"].get("overlap", 2)
        reference, before_text, span = evaluation_text(samples[sample_id], reference_record, before, overlap)
        if reference is None:
            unscored_sample_ids.append(sample_id)
            continue
        old = measure(reference, before_text)
        fallback_reference_words += old["reference_words"]
        if after["status"] != "ok":
            failed_correction_sample_ids.append(sample_id)
            fallback_word_errors += old["word_errors"]
            continue
        changes = []
        if len(before["chunks"]) != len(after["chunks"]):
            raise ValueError(f"mismatched correction chunk count: {sample_id}")
        for original, revision in zip(before["chunks"], after["chunks"]):
            if (original["start"], original["end"]) != (revision["start"], revision["end"]):
                raise ValueError(f"mismatched correction audio window: {sample_id}")
            if original["raw_text"] != revision["raw_text"]:
                changes.append({"start_seconds": original["start"], "end_seconds": original["end"],
                                "before": original["raw_text"], "after": revision["raw_text"],
                                "lexical_change": words(original["raw_text"]) != words(revision["raw_text"])})
        _, after_text, after_span = evaluation_text(samples[sample_id], reference_record, after, overlap)
        if after_span != span:
            raise ValueError(f"mismatched correction scoring span: {sample_id}")
        new = measure(reference, after_text)
        fallback_word_errors += new["word_errors"]
        difference = old["word_errors"] - new["word_errors"]
        text_control = text_only.get(sample_id)
        control_errors = None
        if text_control and text_control.get("status") == "ok":
            if [(chunk["start"], chunk["end"]) for chunk in text_control["chunks"]] != [
                    (chunk["start"], chunk["end"]) for chunk in before["chunks"]]:
                raise ValueError(f"mismatched text-only control windows: {sample_id}")
            _, control_text, control_span = evaluation_text(samples[sample_id], reference_record, text_control, overlap)
            if control_span != span:
                raise ValueError(f"mismatched text-only scoring span: {sample_id}")
            control_errors = measure(reference, control_text)["word_errors"]
        records.append({"sample_id": sample_id, "audio_path": samples[sample_id]["audio_path"],
                        "corpus": samples[sample_id]["corpus"], "language": samples[sample_id]["language"],
                        "scored_interval": span,
                        "baseline_word_errors": old["word_errors"], "corrected_word_errors": new["word_errors"],
                        "asr_plus_gemma_inference_seconds": before.get("inference_seconds", 0) + after.get("inference_seconds", 0),
                        "net_fixed_errors": difference,
                        "net_class": "helpful" if difference > 0 else "harmful" if difference < 0 else "neutral",
                        "text_only_word_errors": control_errors, "changes": changes,
                        "review_note": "Net WER is automatic; listen to each changed interval to classify individual edits."})
    return {"baseline": str(baseline_path), "corrected": str(corrected_path),
            "source_pairing": "sha256" if corrected_run["configuration"].get("source_run_sha256") else "legacy_path_only",
            "text_only": str(text_only_path) if text_only_path else None,
            "asr_plus_gemma_elapsed_seconds": (
                baseline_run["elapsed_seconds"] + corrected_run["elapsed_seconds"]
                if baseline_run.get("elapsed_seconds") is not None and
                corrected_run.get("elapsed_seconds") is not None and
                set(baseline) == set(corrected) else None),
            "paired_recordings": len(records),
            "missing_corrected_sample_ids": sorted(set(baseline) - set(corrected)),
            "extra_corrected_sample_ids": sorted(set(corrected) - set(baseline)),
            "net_fixed_errors": sum(item["net_fixed_errors"] for item in records),
            "helpful_recordings": sum(item["net_class"] == "helpful" for item in records),
            "harmful_recordings": sum(item["net_class"] == "harmful" for item in records),
            "text_only_paired_recordings": sum(item["text_only_word_errors"] is not None for item in records),
            "failed_correction_sample_ids": failed_correction_sample_ids,
            "fallback_word_errors": fallback_word_errors,
            "fallback_reference_words": fallback_reference_words,
            "fallback_micro_wer": (fallback_word_errors / fallback_reference_words
                                   if fallback_reference_words else None),
            "fallback_policy": "Simulation: preserve original ASR transcript when correction status is not ok.",
            "unscored_sample_ids": unscored_sample_ids,
            "unscored_count": len(unscored_sample_ids),
            "records": records}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--references", required=True)
    parser.add_argument("--run", required=True)
    parser.add_argument("--baseline", help="baseline run for paired audio-aware correction analysis")
    parser.add_argument("--text-only", help="matched text-only Gemma control run")
    parser.add_argument("--out", required=True)
    args = parser.parse_args()
    report = score_run(args.manifest, args.references, args.run)
    if args.baseline:
        report["comparison"] = compare_runs(args.manifest, args.references, args.baseline, args.run, args.text_only)
    report["scorer_sha256"] = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    report["manifest_sha256"] = hashlib.sha256(Path(args.manifest).read_bytes()).hexdigest()
    report["references_sha256"] = hashlib.sha256(Path(args.references).read_bytes()).hexdigest()
    Path(args.out).write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ("sample_count", "success_count", "micro_wer", "italian_micro_wer")}))


if __name__ == "__main__":
    main()
