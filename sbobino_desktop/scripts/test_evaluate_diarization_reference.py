import unittest
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from evaluate_diarization_reference import annotation, evaluate
from benchmark_pyannote_diarization import schedule


class DiarizationReferenceTests(unittest.TestCase):
    def test_benchmark_schedule_keeps_three_identical_warm_inputs(self):
        self.assertEqual(schedule('short', 'long'), [
            ('cold', 'short'), ('warm_1', 'short'), ('warm_2', 'short'),
            ('warm_3', 'short'), ('long', 'long'),
        ])

    def test_permuted_speaker_names_are_not_errors(self):
        reference = [dict(speaker_id='alice', start_seconds=0, end_seconds=1),
                     dict(speaker_id='bob', start_seconds=1, end_seconds=2)]
        hypothesis = [dict(t, speaker_id=str(i)) for i, t in enumerate(reference)]
        score = evaluate(reference, hypothesis, 2)
        self.assertEqual(score['der'], 0)
        self.assertEqual(score['jer'], 0)
        self.assertEqual(evaluate(reference, [], 2)['der'], 1)

    def test_overlapping_voices_and_padding_are_preserved(self):
        turns = [dict(speaker_id='alice', start_seconds=0, end_seconds=2),
                 dict(speaker_id='bob', start_seconds=1, end_seconds=3)]
        score = evaluate(turns, turns, 2)
        self.assertEqual(score['der'], 0)
        self.assertEqual(score['der_details']['total'], 3)
        self.assertEqual(score['reference_speakers'], 2)

    def test_invalid_turn_is_not_silently_ignored(self):
        for start, end in ((-1, 1), (1, 1), (0, float('nan'))):
            with self.assertRaises(ValueError):
                annotation([dict(speaker_id='x', start_seconds=start, end_seconds=end)], 2)
        with self.assertRaises(ValueError):
            evaluate([], [], 2)


if __name__ == '__main__':
    unittest.main()
