#!/usr/bin/env python3

import argparse
import importlib.util
import pathlib
import unittest


MODULE_PATH = pathlib.Path(__file__).with_name("evaluate_asr_reference.py")
SPEC = importlib.util.spec_from_file_location("evaluate_asr_reference", MODULE_PATH)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def args(**overrides):
    values = {
        "max_wer": 0.35,
        "max_cer": 0.25,
        "max_gap_seconds": 2.0,
        "language_tolerance_seconds": 2.0,
        "require_reviewed_reference": False,
    }
    values.update(overrides)
    return argparse.Namespace(**values)


class EvaluateAsrReferenceTests(unittest.TestCase):
    def test_matching_multilingual_timeline_passes(self):
        timeline = {
            "duration_seconds": 4.0,
            "segments": [
                {"start_seconds": 0.0, "end_seconds": 2.0, "language_code": "it", "text": "Ciao mondo"},
                {"start_seconds": 2.0, "end_seconds": 4.0, "language_code": "en", "text": "Hello world"},
            ],
        }
        report = MODULE.evaluate(timeline, timeline, args())
        self.assertEqual(report["status"], "passed")
        self.assertEqual(report["metrics"]["wer"], 0.0)

    def test_missing_interval_and_language_transition_fail(self):
        reference = {
            "duration_seconds": 8.0,
            "segments": [
                {"start_seconds": 0.0, "end_seconds": 3.0, "language_code": "it", "text": "ciao a tutti"},
                {"start_seconds": 3.0, "end_seconds": 8.0, "language_code": "en", "text": "hello to everyone"},
            ],
        }
        hypothesis = {
            "duration_seconds": 8.0,
            "segments": [
                {"start_seconds": 0.0, "end_seconds": 2.0, "language_code": "it", "text": "ciao a tutti"},
            ],
        }
        report = MODULE.evaluate(reference, hypothesis, args())
        self.assertEqual(report["status"], "failed")
        self.assertTrue(any("uncovered interval" in item for item in report["failures"]))
        self.assertTrue(any("transition to en" in item for item in report["failures"]))

    def test_service_text_and_non_monotonic_timestamps_fail(self):
        reference = {
            "duration_seconds": 3.0,
            "segments": [
                {"start_seconds": 0.0, "end_seconds": 3.0, "language_code": "en", "text": "hello world"},
            ],
        }
        hypothesis = {
            "duration_seconds": 3.0,
            "segments": [
                {"start_seconds": 1.0, "end_seconds": 2.0, "language_code": "en", "text": "fallback CPU-safe mode"},
                {"start_seconds": 0.5, "end_seconds": 3.0, "language_code": "en", "text": "hello world"},
            ],
        }
        report = MODULE.evaluate(reference, hypothesis, args(max_wer=10.0, max_cer=10.0))
        self.assertEqual(report["status"], "failed")
        self.assertTrue(any("non-monotonic" in item for item in report["failures"]))
        self.assertTrue(any("technical/service" in item for item in report["failures"]))

    def test_release_gate_rejects_unreviewed_reference(self):
        timeline = {
            "duration_seconds": 1.0,
            "segments": [
                {"start_seconds": 0.0, "end_seconds": 1.0, "language_code": "it", "text": "ciao"},
            ],
        }
        report = MODULE.evaluate(timeline, timeline, args(require_reviewed_reference=True))
        self.assertEqual(report["status"], "failed")
        self.assertIn("reference is not marked review_status=reviewed", report["failures"])

    def test_reference_silence_is_not_counted_as_missing_speech(self):
        reference = {
            "duration_seconds": 10.0,
            "segments": [
                {"start_seconds": 0.0, "end_seconds": 2.0, "language_code": "it", "text": "ciao"},
                {"start_seconds": 8.0, "end_seconds": 10.0, "language_code": "it", "text": "fine"},
            ],
        }
        report = MODULE.evaluate(reference, reference, args(max_gap_seconds=0.1))
        self.assertEqual(report["status"], "passed")


if __name__ == "__main__":
    unittest.main()
