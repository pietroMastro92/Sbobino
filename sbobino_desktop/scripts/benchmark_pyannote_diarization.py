#!/usr/bin/env python3
"""Reuse the app's packaged Pyannote pipeline with one model load and real warm passes."""
import argparse
import hashlib
import json
import platform
from pathlib import Path
import time
import wave

from pyannote_diarize import load_wav_input, resolve_annotation, resolve_device_candidates


def schedule(audio, long_audio):
    return [(phase, long_audio if phase == 'long' else audio)
            for phase in ('cold', 'warm_1', 'warm_2', 'warm_3', 'long')]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--audio', type=Path, required=True)
    parser.add_argument('--long-audio', type=Path, required=True)
    parser.add_argument('--model-path', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    started = time.monotonic()
    # Import/model load cost is charged to the first pass, never to warm inference.
    import torch
    from pyannote.audio import Pipeline

    pipeline = Pipeline.from_pretrained(str(args.model_path))
    pipeline.to(resolve_device_candidates('cpu')[0])
    load_seconds = time.monotonic() - started
    metadata = dict(platform=platform.platform(), machine=platform.machine(),
                    torch_threads=torch.get_num_threads(), device='cpu',
                    cold_definition='first import/model load/inference; OS file cache not flushed',
                    memory_definition='external process peak RSS, not per-phase or system allocations',
                    sha256={str(p): hashlib.sha256(p.read_bytes()).hexdigest()
                            for p in (Path(__file__), Path(__file__).with_name('pyannote_diarize.py'),
                                      args.audio, args.long_audio)})
    (args.output / 'metadata.json').write_text(json.dumps(metadata, indent=2) + '\n')
    for index, (phase, audio) in enumerate(schedule(args.audio, args.long_audio)):
        start = started if index == 0 else time.monotonic()
        processing = time.monotonic()
        result = resolve_annotation(pipeline(load_wav_input(str(audio))))
        processing_seconds = time.monotonic() - processing
        with wave.open(str(audio)) as wav:
            duration = wav.getnframes() / wav.getframerate()
        order, turns = {}, []
        for turn, _, speaker in result.itertracks(yield_label=True):
            order.setdefault(speaker, len(order) + 1)
            if 0 <= turn.start < duration:
                turns.append(dict(speaker_id=f'speaker_{order[speaker]}',
                                  start_seconds=float(turn.start), end_seconds=min(float(turn.end), duration)))
        output = dict(speakers=turns, phase=phase, audio_path=str(audio),
                      metrics=dict(load_seconds=load_seconds if index == 0 else 0,
                                   processing_seconds=processing_seconds,
                                   elapsed_seconds=time.monotonic() - start))
        (args.output / f'{phase}.json').write_text(json.dumps(output, indent=2) + '\n')
        print(json.dumps(dict(phase=phase, metrics=output['metrics'])), flush=True)


if __name__ == '__main__':
    main()
