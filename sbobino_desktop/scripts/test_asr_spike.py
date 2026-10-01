#!/usr/bin/env python3
"""Small checks for boundaries and literal scoring; run with unittest."""

import unittest
import io
import json
import os
import shutil
import tempfile
import wave
from hashlib import sha256
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from asr_spike import DIRECT, LlamaServer, _parakeet_manifest, _run_parakeet_batch, _run_parakeet_worker, stitch, windows, write_window
from asr_spike_score import compare_runs, edits, load_samples, measure, score_run


class SpikeChecks(unittest.TestCase):
    @unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg unavailable")
    def test_noise_seed_reproduces_audio(self):
        from asr_spike_corpus import _render_perturbation
        with tempfile.TemporaryDirectory() as directory:
            source, first, second, different = (Path(directory) / name for name in
                ("source.wav", "first.wav", "second.wav", "different.wav"))
            write_window(b"\0\0" * 1600, 0, 0.1, source)
            for target, seed in [(first, 17), (second, 17), (different, 18)]:
                _render_perturbation(source, target, "noise", seed=seed)
            self.assertEqual(first.read_bytes(), second.read_bytes())
            self.assertNotEqual(first.read_bytes(), different.read_bytes())

    def test_reference_checksum_rejects_changed_text(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "references.json"
            record = {"id": "a", "reference": "uno", "reference_sha256": sha256(b"uno").hexdigest()}
            path.write_text(json.dumps({"samples": [record]}))
            self.assertEqual(load_samples(path)["a"]["reference"], "uno")
            record["reference"] = "due"
            path.write_text(json.dumps({"samples": [record]}))
            with self.assertRaisesRegex(ValueError, "reference checksum"):
                load_samples(path)

    def test_llama_dry_diagnostic_is_explicit(self):
        server = LlamaServer.__new__(LlamaServer)
        server.url = "http://127.0.0.1:1"
        answer = b'{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}]}'
        for multiplier in (0.0, 0.8):
            with self.subTest(multiplier=multiplier), mock.patch(
                    "asr_spike.urllib.request.urlopen", return_value=io.BytesIO(answer)) as opened:
                self.assertEqual(server.infer(None, "trascrivi", 10, multiplier)["text"], "ok")
                payload = json.loads(opened.call_args.args[0].data)
                self.assertEqual(payload.get("dry_multiplier", 0.0), multiplier)

    def test_parakeet_manifest_offsets_recordings_and_keeps_local_map(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first = {"start": 0, "end": 28, "wav": str(root / "first.wav")}
            second = {"start": 26, "end": 30, "wav": str(root / "second.wav")}
            third = {"start": 0, "end": 4, "wav": str(root / "third.wav")}
            jobs = [
                {"chunks": [first, second], "duration": 30},
                {"chunks": [third], "duration": 4},
            ]
            manifest = root / "parakeet.tsv"
            mapping = _parakeet_manifest(SimpleNamespace(overlap=2), jobs, manifest)
            rows = [line.split("\t") for line in manifest.read_text().splitlines()]

            self.assertEqual([row[0] for row in rows], ["0", "1", "2"])
            self.assertEqual(rows[1][3], rows[0][4])
            self.assertEqual(rows[2][1], "30.000")
            self.assertEqual(rows[2][3], rows[1][4])
            self.assertEqual(mapping, [(jobs[0], 0), (jobs[0], 1), (jobs[1], 0)])
            self.assertEqual([(first["start"], first["end"]), (second["start"], second["end"])],
                             [(0, 28), (26, 30)])

    def test_parakeet_batch_uses_one_worker_per_language_without_cross_recording_stitch(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            samples = []
            for index, language in enumerate(("it", "it", "en")):
                audio = root / f"sample-{index}.wav"
                write_window(b"\0\0" * 32000, 0, 2, audio)
                samples.append({
                    "id": f"sample-{index}", "audio_path": str(audio),
                    "audio_sha256": sha256(audio.read_bytes()).hexdigest(),
                    "duration_seconds": 2, "language": language,
                })
            args = SimpleNamespace(
                window=28, overlap=2, language=None, mode="direct", device="cpu",
                parakeet_threads_supported=False, threads=4, binary="fake", model="fake",
            )
            result = {"samples": []}
            calls = []

            def fake_worker(worker_args, manifest, temp_dir, language, chunk_count):
                rows = [line.split("\t") for line in manifest.read_text().splitlines()]
                calls.append((language, chunk_count, rows))
                return [({"index": index, "result": {"text": f"chunk-{index}"}}, 1.2 if index == 0 else 0.1)
                        for index in range(chunk_count)], 123, "worker ready", None, 0.05

            with mock.patch("asr_spike._run_parakeet_worker", side_effect=fake_worker):
                _run_parakeet_batch(args, samples, {}, result)

            self.assertEqual([(language, count) for language, count, _ in calls],
                             [("it", 2), ("en", 1)])
            self.assertEqual([row["text"] for row in result["samples"]],
                             ["chunk-0", "chunk-1", "chunk-0"])
            self.assertTrue(all(row["status"] == "ok" for row in result["samples"]))
            self.assertEqual(calls[0][2][1][1], "2.000")
            self.assertEqual(calls[1][2][0][1], "0.000")
            self.assertEqual([chunk["start"] for row in result["samples"] for chunk in row["chunks"]],
                             [0.0, 0.0, 0.0])
            self.assertAlmostEqual(result["samples"][0]["chunks"][0]["elapsed_seconds"], 1.15)
            self.assertEqual(result["samples"][0]["chunks"][0]["wall_seconds_including_startup"], 1.2)
            self.assertAlmostEqual(result["samples"][0]["inference_seconds"], 1.15)
            self.assertTrue(result["samples"][0]["runtime"]["warm_timing_available"])

    def test_parakeet_batch_rejects_malformed_transcript(self):
        with tempfile.TemporaryDirectory() as directory:
            audio = Path(directory) / "sample.wav"
            write_window(b"\0\0" * 32000, 0, 2, audio)
            sample = {"id": "sample", "audio_path": str(audio), "audio_sha256": sha256(audio.read_bytes()).hexdigest(),
                      "duration_seconds": 2, "language": "it"}
            args = SimpleNamespace(window=28, overlap=2, language=None, mode="direct", device="cpu")
            for malformed in ({}, {"text": None}, {"text": 123}):
                with self.subTest(malformed=malformed):
                    result = {"samples": []}
                    with mock.patch("asr_spike._run_parakeet_worker", return_value=(
                            [({"index": 0, "result": malformed}, 0.1)], 123, "ready", None, 0.05)):
                        _run_parakeet_batch(args, [sample], {}, result)
                    self.assertEqual(result["samples"][0]["status"], "error")
                    self.assertIn("string text", result["samples"][0]["error"])

    def test_parakeet_worker_rejects_extra_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fake = root / "worker"
            fake.write_text("#!/usr/bin/env python3\nimport json, sys, time\n"
                            "print('SBOBINO_PARAKEET_WORKER {\"phase\":\"model_ready\"}', file=sys.stderr, flush=True)\n"
                            "time.sleep(0.15)\n"
                            "for i in range(2): print(json.dumps({'index': i, 'result': {'text': 'ok'}}), flush=True)\n")
            os.chmod(fake, 0o700)
            manifest = root / "chunks.tsv"
            manifest.write_text("0\t0.000\t2.000\t0.000\t2.000\t/tmp/unused.wav\n")
            args = SimpleNamespace(binary=str(fake), model="unused", parakeet_threads_supported=False,
                                   device="cpu", threads=4)
            lines, _, _, failure, startup = _run_parakeet_worker(args, manifest, root, "it", 1)
            self.assertEqual(len(lines), 2)
            self.assertIsInstance(failure, ValueError)
            self.assertIn("more chunks", str(failure))
            self.assertIsNotNone(startup)

    def test_windows_cover_audio_without_exceeding_gemma_limit(self):
        for overlap in (0, 2):
            chunks = windows(90 * 60 + 0.37, 28, overlap)
            self.assertEqual(chunks[0][0], 0)
            self.assertAlmostEqual(chunks[-1][1], 5400.37)
            self.assertTrue(all(0 < end - start <= 30 for start, end in chunks))
            self.assertTrue(all(right[0] <= left[1] for left, right in zip(chunks, chunks[1:])))

    def test_stitch_preserves_real_repetitions(self):
        text, findings = stitch([
            {"start": 0, "raw_text": "Sì sì, molto molto"},
            {"start": 26, "raw_text": "molto bene"},
        ], 2)
        self.assertEqual(text, "Sì sì, molto molto molto bene")
        self.assertEqual(findings[0]["kind"], "ambiguous_repeat")
        text, findings = stitch([
            {"start": 0, "raw_text": "una prova al confine"},
            {"start": 26, "raw_text": "al confine continua"},
        ], 2)
        self.assertEqual(text, "una prova al confine continua")
        self.assertEqual(findings, [])
        text, findings = stitch([
            {"start": 0, "raw_text": "parola finale"},
            {"start": 26, "raw_text": "inizio nuovo"},
        ], 2)
        self.assertEqual(text, "parola finale inizio nuovo")
        self.assertEqual(findings[0]["kind"], "unaligned_overlap")
        long_phrase = "uno due tre quattro cinque sei sette otto nove"
        text, findings = stitch([
            {"start": 0, "raw_text": "prima " + long_phrase},
            {"start": 26, "raw_text": long_phrase + " dopo"},
        ], 2)
        self.assertEqual(text, "prima " + long_phrase + " " + long_phrase + " dopo")
        self.assertEqual(findings[0]["kind"], "ambiguous_long_overlap")

    def test_final_window_keeps_every_pcm_sample(self):
        frames = 28 * 16000 + 7
        duration = frames / 16000
        bounds = windows(duration, 28, 2)
        self.assertEqual(bounds[-1][1], duration)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "last.wav"
            write_window(b"\0\0" * frames, *bounds[-1], output)
            with wave.open(str(output)) as audio:
                self.assertEqual(audio.getnframes(), frames - round(bounds[-1][0] * 16000))

    def test_numbers_and_insertions_count(self):
        result = measure("Nel 2026 ho detto 42, 42.", "Nel 2025 ho detto 42, 42 davvero.")
        self.assertEqual(result["substitutions"], 1)
        self.assertEqual(result["insertions"], 1)
        self.assertEqual(result["number_errors"], 1)
        self.assertEqual(result["reference_words"], 6)

    def test_editops_preserve_substitution_deletion_insertion_counts(self):
        counts, cost = edits(["uno", "due", "tre"], ["uno", "x", "tre", "quattro"])
        self.assertEqual(cost, 2)
        self.assertEqual(counts, {"substitutions": 1, "deletions": 0, "insertions": 1})

    def test_empty_speech_has_no_fake_wer(self):
        result = measure("", "", [])
        self.assertIsNone(result["wer"])

    def test_prompt_echo_is_flagged_and_kept_in_quality_counts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest, references, run = (root / name for name in ("manifest.json", "references.json", "run.json"))
            manifest.write_text(json.dumps({"samples": [{"id": "mute", "corpus": "control",
                "language": "it", "duration_seconds": 28}]}))
            references.write_text(json.dumps({"samples": [{"id": "mute", "reference": ""}]}))
            prompt = DIRECT.format(language="italiano")
            run.write_text(json.dumps({"configuration": {"manifest_sha256": sha256(manifest.read_bytes()).hexdigest()},
                "samples": [{"sample_id": "mute", "status": "ok", "text": prompt,
                             "chunks": [{"start": 0, "end": 28, "raw_text": prompt}]}]}))
            scored = score_run(manifest, references, run)
            self.assertEqual(scored["prompt_echo_chunk_count"], 1)
            self.assertEqual(scored["possible_silence_hallucinations"], 1)
            self.assertGreater(scored["totals"]["word_errors"], 0)
            self.assertIsNone(scored["micro_wer"])

    def test_score_keeps_reference_in_separate_file_and_compares_edits(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = root / "manifest.json"
            references = root / "references.json"
            baseline = root / "baseline.json"
            corrected = root / "corrected.json"
            manifest.write_text(json.dumps({"samples": [{"id": "a", "audio_path": "/tmp/a.wav",
                                                         "corpus": "fleurs", "language": "it", "duration_seconds": 2}]}))
            configuration = {"manifest_sha256": sha256(manifest.read_bytes()).hexdigest()}
            references.write_text(json.dumps({"samples": [{"id": "a", "reference": "quarantadue 42",
                                                          "entities": ["42"], "segments": [
                                                              {"start": 0, "end": 2, "text": "quarantadue 42"}]}]}))
            baseline.write_text(json.dumps({"configuration": configuration, "samples": [{"sample_id": "a", "status": "ok",
                                                        "text": "quarantadue 43", "chunks": [
                                                            {"start": 0, "end": 2, "raw_text": "quarantadue 43"}]}]}))
            corrected_configuration = {**configuration, "source_run_sha256": sha256(baseline.read_bytes()).hexdigest()}
            corrected.write_text(json.dumps({"configuration": corrected_configuration, "samples": [{"sample_id": "a", "status": "ok",
                                                         "text": "quarantadue 42", "chunks": [
                                                             {"start": 0, "end": 2, "raw_text": "quarantadue 42"}]}]}))
            scored = score_run(manifest, references, corrected)
            self.assertEqual(scored["micro_wer"], 0)
            self.assertEqual(scored["samples"][0]["entity_reference_intervals"]["42"],
                             [{"start_seconds": 0, "end_seconds": 2}])
            comparison = compare_runs(manifest, references, baseline, corrected)
            self.assertEqual(comparison["net_fixed_errors"], 1)
            self.assertIsNone(comparison["asr_plus_gemma_elapsed_seconds"])
            self.assertEqual(comparison["fallback_word_errors"], 0)
            self.assertEqual(comparison["records"][0]["changes"][0]["start_seconds"], 0)
            corrected.write_text(json.dumps({"configuration": corrected_configuration, "samples": [{
                "sample_id": "a", "status": "truncated", "text": "quarantadue 43 43",
                "chunks": [{"start": 0, "end": 2, "raw_text": "quarantadue 43 43"}]}]}))
            failed = compare_runs(manifest, references, baseline, corrected)
            self.assertEqual(failed["failed_correction_sample_ids"], ["a"])
            self.assertEqual(failed["fallback_word_errors"], 1)
            self.assertEqual(failed["fallback_micro_wer"], 0.5)
            corrected.write_text(json.dumps({"configuration": corrected_configuration, "samples": [{"sample_id": "a", "status": "ok",
                                                         "text": "quarantadue 43!", "chunks": [
                                                             {"start": 0, "end": 2, "raw_text": "quarantadue 43!"}]}]}))
            punctuation_only = compare_runs(manifest, references, baseline, corrected)["records"][0]["changes"]
            self.assertEqual(len(punctuation_only), 1)
            self.assertFalse(punctuation_only[0]["lexical_change"])
            corrected.write_text(json.dumps({"configuration": corrected_configuration, "samples": [{"sample_id": "a", "status": "ok",
                                                         "text": "quarantadue 42", "chunks": []}]}))
            with self.assertRaisesRegex(ValueError, "chunk count"):
                compare_runs(manifest, references, baseline, corrected)
            corrected.write_text(json.dumps({"configuration": {"manifest_sha256": "wrong"}, "samples": []}))
            with self.assertRaisesRegex(ValueError, "manifest checksum"):
                score_run(manifest, references, corrected)
            corrected.write_text(json.dumps({"configuration": corrected_configuration, "samples": [
                {"sample_id": "a", "status": "ok"}, {"sample_id": "a", "status": "ok"}]}))
            with self.assertRaisesRegex(ValueError, "duplicate run sample ID"):
                score_run(manifest, references, corrected)

    def test_subset_correction_does_not_claim_full_baseline_elapsed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest, references, baseline, corrected = (root / name for name in
                ("manifest.json", "references.json", "baseline.json", "corrected.json"))
            manifest.write_text(json.dumps({"samples": [
                {"id": sample_id, "audio_path": f"/tmp/{sample_id}.wav", "corpus": "fleurs", "language": "it"}
                for sample_id in ("a", "b")]}))
            references.write_text(json.dumps({"samples": [{"id": "a", "reference": "ciao"}]}))
            config = {"manifest_sha256": sha256(manifest.read_bytes()).hexdigest()}
            sample = {"sample_id": "a", "status": "ok", "text": "ciao", "chunks": [
                {"start": 0, "end": 2, "raw_text": "ciao"}]}
            baseline.write_text(json.dumps({"configuration": config, "elapsed_seconds": 9,
                                            "samples": [sample, {**sample, "sample_id": "b"}]}))
            corrected.write_text(json.dumps({"configuration": {**config,
                "source_run_sha256": sha256(baseline.read_bytes()).hexdigest()},
                "elapsed_seconds": 2, "samples": [sample]}))
            comparison = compare_runs(manifest, references, baseline, corrected)
            self.assertIsNone(comparison["asr_plus_gemma_elapsed_seconds"])
            self.assertEqual(comparison["missing_corrected_sample_ids"], ["b"])
            self.assertEqual(comparison["extra_corrected_sample_ids"], [])
            corrected.write_text(json.dumps({"configuration": {**config,
                "source_run_sha256": sha256(baseline.read_bytes()).hexdigest()},
                "elapsed_seconds": 2, "samples": [sample, {**sample, "sample_id": "c"}]}))
            self.assertEqual(compare_runs(manifest, references, baseline, corrected)[
                "extra_corrected_sample_ids"], ["c"])

    def test_ami_partial_reference_scores_only_annotated_interior(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest, references, run = (root / name for name in
                ("manifest.json", "references.json", "run.json"))
            manifest.write_text(json.dumps({"samples": [{"id": "meeting", "corpus": "ami",
                "language": "en", "duration_seconds": 100, "audio_path": "/tmp/meeting.wav"}]}))
            references.write_text(json.dumps({"samples": [{"id": "meeting", "reference": "opening middle tail closing",
                "segments": [{"start": 10, "end": 11, "text": "opening"},
                             {"start": 30, "end": 31, "text": "middle"},
                             {"start": 60, "end": 61, "text": "tail"},
                             {"start": 90, "end": 91, "text": "closing"}]}]}))
            chunks = [{"start": start, "end": end, "raw_text": text} for start, end, text in
                [(0, 28, "opening"), (26, 54, "middle"), (52, 80, "tail"), (78, 100, "closing")]]
            run.write_text(json.dumps({"configuration": {
                "manifest_sha256": sha256(manifest.read_bytes()).hexdigest(), "overlap": 2},
                "samples": [{"sample_id": "meeting", "status": "ok",
                             "text": "opening middle tail closing", "chunks": chunks}]}))
            scored = score_run(manifest, references, run)
            self.assertEqual(scored["micro_wer"], 0)
            self.assertEqual(scored["samples"][0]["scored_interval"],
                             {"start_seconds": 26, "end_seconds": 80})
            gold = json.loads(references.read_text())
            gold["samples"][0]["segments"].append({"start": 79, "end": 81, "text": "crossed utterance"})
            references.write_text(json.dumps(gold))
            scored = score_run(manifest, references, run)
            self.assertEqual(scored["micro_wer"], 0)
            self.assertEqual(scored["samples"][0]["scored_interval"],
                             {"start_seconds": 26, "end_seconds": 54})
            corrected = root / "corrected.json"
            payload = json.loads(run.read_text())
            payload["configuration"]["source_run_sha256"] = sha256(run.read_bytes()).hexdigest()
            payload["samples"][0]["text"] = "changed outside annotation"
            payload["samples"][0]["chunks"][0]["raw_text"] = "changed outside annotation"
            payload["samples"][0]["chunks"][-1]["raw_text"] = "also changed outside annotation"
            corrected.write_text(json.dumps(payload))
            comparison = compare_runs(manifest, references, run, corrected, corrected)
            self.assertEqual(comparison["net_fixed_errors"], 0)
            self.assertEqual(comparison["records"][0]["text_only_word_errors"], 0)
            self.assertEqual(comparison["fallback_word_errors"], 0)
            self.assertEqual(comparison["records"][0]["scored_interval"],
                             {"start_seconds": 26, "end_seconds": 54})

    def test_reference_free_stability_sample_is_reported_but_not_scored(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = root / "manifest.json"
            references = root / "references.json"
            run = root / "run.json"
            corrected = root / "corrected.json"
            samples = [
                {"id": "clip", "audio_path": "/tmp/clip.wav", "corpus": "fleurs",
                 "language": "it", "duration_seconds": 2},
                {"id": "stability-90m", "audio_path": "/tmp/stability.wav", "corpus": "stability",
                 "language": "it", "duration_seconds": 5400},
            ]
            manifest.write_text(json.dumps({"samples": samples}))
            configuration = {"manifest_sha256": sha256(manifest.read_bytes()).hexdigest()}
            references.write_text(json.dumps({"samples": [{"id": "clip", "reference": "parola"}]}))
            run.write_text(json.dumps({"configuration": configuration, "samples": [
                {"sample_id": "clip", "status": "ok", "text": "parola",
                 "chunks": [{"start": 0, "end": 2, "raw_text": "parola"}]},
                {"sample_id": "stability-90m", "status": "ok", "text": "output stress",
                 "chunks": [{"start": 0, "end": 5400, "raw_text": "output stress"}]},
            ]}))

            report = score_run(manifest, references, run)
            self.assertEqual(report["sample_count"], 2)
            self.assertEqual(report["quality_sample_count"], 1)
            self.assertEqual(report["micro_wer"], 0)
            self.assertEqual(report["unscored_sample_ids"], ["stability-90m"])
            self.assertEqual(report["samples"][1]["status"], "unscored")

            corrected_payload = json.loads(run.read_text())
            corrected_payload["configuration"]["source_run_sha256"] = sha256(run.read_bytes()).hexdigest()
            corrected.write_text(json.dumps(corrected_payload))
            comparison = compare_runs(manifest, references, run, corrected)
            self.assertEqual(comparison["paired_recordings"], 1)
            self.assertEqual(comparison["unscored_sample_ids"], ["stability-90m"])

    def test_timed_entity_must_appear_near_its_reference_interval(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            manifest = root / "manifest.json"
            references = root / "references.json"
            run = root / "run.json"
            manifest.write_text(json.dumps({"samples": [{"id": "talk", "corpus": "mtedx",
                                                         "language": "it", "duration_seconds": 12}]}))
            references.write_text(json.dumps({"samples": [{"id": "talk", "reference": "Turing",
                                                          "entities": ["Turing"], "segments": [
                                                              {"start": 10, "end": 12, "text": "Turing"}]}]}))
            run.write_text(json.dumps({"configuration": {"manifest_sha256": sha256(manifest.read_bytes()).hexdigest()},
                                       "samples": [{"sample_id": "talk", "status": "ok", "text": "Turing altro",
                                                    "chunks": [{"start": 0, "end": 2, "raw_text": "Turing"},
                                                               {"start": 10, "end": 12, "raw_text": "altro"}]}]}))
            report = score_run(manifest, references, run)
            self.assertEqual(report["entity_correct_count"], 1)
            self.assertEqual(report["entity_local_correct_count"], 0)


if __name__ == "__main__":
    unittest.main()
