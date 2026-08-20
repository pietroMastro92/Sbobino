#!/usr/bin/env python3

import importlib.util
import pathlib
import unittest


MODULE_PATH = pathlib.Path(__file__).with_name("evaluate_live_latency.py")
SPEC = importlib.util.spec_from_file_location("evaluate_live_latency", MODULE_PATH)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class EvaluateLiveLatencyTests(unittest.TestCase):
    def test_realtime_report_passes(self):
        payload = {
            "engine": "parakeet",
            "platform": "macos-x86_64",
            "first_preview_seconds": 0.8,
            "finalization_seconds": 1.2,
            "dropped_samples": 0,
            "missing_segments": 0,
            "duplicate_segments": 0,
            "rss_samples_mib": [500, 505, 508, 510, 512, 515],
            "samples": [
                {"captured_seconds": second, "processed_seconds": second - 0.3,
                 "backlog_seconds": 0.3, "preview_latency_seconds": 0.7}
                for second in range(1, 20)
            ],
        }
        self.assertEqual(MODULE.evaluate(payload, 2.0, 256.0)["status"], "passed")

    def test_growing_backlog_and_loss_fail(self):
        payload = {
            "engine": "whisper",
            "platform": "windows-x86_64",
            "first_preview_seconds": 2.5,
            "finalization_seconds": 3.0,
            "dropped_samples": 16000,
            "missing_segments": 1,
            "duplicate_segments": 0,
            "rss_samples_mib": [400, 410, 420, 430, 440, 800],
            "samples": [
                {"captured_seconds": second, "processed_seconds": max(0, second / 2),
                 "backlog_seconds": second / 2, "preview_latency_seconds": second / 2}
                for second in range(1, 20)
            ],
        }
        report = MODULE.evaluate(payload, 2.0, 256.0)
        self.assertEqual(report["status"], "failed")
        self.assertTrue(any("backlog P95" in item for item in report["failures"]))
        self.assertTrue(any("dropped_samples" in item for item in report["failures"]))
        self.assertTrue(any("RSS growth" in item for item in report["failures"]))

    def test_non_monotonic_cursor_fails(self):
        payload = {
            "first_preview_seconds": 1.0,
            "finalization_seconds": 1.0,
            "samples": [
                {"captured_seconds": 2.0, "processed_seconds": 1.5, "backlog_seconds": 0.5, "preview_latency_seconds": 1.0},
                {"captured_seconds": 1.0, "processed_seconds": 0.5, "backlog_seconds": 0.5, "preview_latency_seconds": 1.0},
            ],
        }
        report = MODULE.evaluate(payload, 2.0, 256.0)
        self.assertEqual(report["status"], "failed")
        self.assertTrue(any("non-monotonic" in item for item in report["failures"]))


if __name__ == "__main__":
    unittest.main()
