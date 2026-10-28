#!/usr/bin/env python3
"""Score diarization against an actual JSON/RTTM reference; requires pyannote.metrics."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import wave


def annotation(turns, duration):
    from pyannote.core import Annotation, Segment

    result = Annotation()
    for index, turn in enumerate(turns):
        start, end = float(turn['start_seconds']), float(turn['end_seconds'])
        speaker = turn['speaker_id']
        if not (math.isfinite(start) and math.isfinite(end) and 0 <= start < end):
            raise ValueError(f'invalid turn {index}')
        if not isinstance(speaker, str) or not speaker.strip():
            raise ValueError(f'invalid speaker {index}')
        if start < duration:
            result[Segment(start, min(end, duration)), index] = speaker
    return result


def reference_turns(path):
    if path.suffix == '.json':
        return json.loads(path.read_text())['speakers']
    turns = []
    recordings = set()
    for line in path.read_text().splitlines():
        if not line.strip():
            continue
        fields = line.split()
        if len(fields) != 10 or fields[0] != 'SPEAKER':
            raise ValueError('expected single-recording SPEAKER RTTM')
        recordings.add(fields[1])
        turns.append(dict(speaker_id=fields[7], start_seconds=float(fields[3]),
                          end_seconds=float(fields[3]) + float(fields[4])))
    if len(recordings) != 1:
        raise ValueError('reference must contain exactly one recording')
    return turns


def evaluate(reference, hypothesis, duration):
    from pyannote.core import Segment, Timeline
    from pyannote.metrics.diarization import DiarizationErrorRate, JaccardErrorRate

    ref, hyp = annotation(reference, duration), annotation(hypothesis, duration)
    if not ref:
        raise ValueError('reference has no speech in the scored interval')
    uem = Timeline([Segment(0, duration)])
    der = DiarizationErrorRate(collar=0.0, skip_overlap=False)
    jer = JaccardErrorRate(collar=0.0, skip_overlap=False)
    der_details = der(ref, hyp, uem=uem, detailed=True)
    jer_details = jer(ref, hyp, uem=uem, detailed=True)
    return dict(der=float(abs(der)), jer=float(abs(jer)), der_details=der_details,
                jer_details=jer_details, reference_speakers=len(ref.labels()),
                predicted_speakers=len(hyp.labels()), collar_seconds=0,
                skip_overlap=False, scored_seconds=duration,
                boundaries='clipped to actual audio duration',
                scope='this annotated recording only; not general or multilingual quality')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--reference', type=Path, required=True)
    parser.add_argument('--prediction', type=Path, required=True)
    parser.add_argument('--audio', type=Path, required=True)
    args = parser.parse_args()
    with wave.open(str(args.audio)) as audio:
        duration = audio.getnframes() / audio.getframerate()
    result = evaluate(reference_turns(args.reference),
                      json.loads(args.prediction.read_text())['speakers'], duration)
    result['sha256'] = {str(p): hashlib.sha256(p.read_bytes()).hexdigest()
                        for p in (args.reference, args.prediction, args.audio)}
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    main()
