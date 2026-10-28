#!/usr/bin/env python3
"""Replay public/synthetic WAV into the native speaker helper, without opening a microphone."""
import argparse
import array
from collections import deque
import hashlib
import json
import math
import platform
from pathlib import Path
import statistics
import subprocess
import sys
import threading
import time
import wave


def merge_turns(turns, last, incoming):
    for turn in incoming:
        speaker = turn['speaker_id']
        start, end = turn['start_seconds'], turn['end_seconds']
        if not (isinstance(speaker, str) and speaker and math.isfinite(start)
                and math.isfinite(end) and 0 <= start < end):
            raise ValueError('invalid streaming speaker turn')
        previous = last.get(speaker)
        if previous is not None and abs(turns[previous]['end_seconds'] - start) < 0.0001:
            turns[previous]['end_seconds'] = end
        else:
            last[speaker] = len(turns)
            turns.append(dict(turn))


def run(args):
    if sys.platform != 'darwin':
        raise RuntimeError('native speaker replay requires macOS')
    import resource

    args.output.mkdir(parents=True, exist_ok=False)
    with wave.open(str(args.audio)) as audio:
        if (audio.getnchannels(), audio.getsampwidth(), audio.getframerate(), audio.getcomptype()) != (1, 2, 16000, 'NONE'):
            raise ValueError('replay requires mono 16 kHz PCM16 WAV')
        audio_frames = audio.getnframes()
        duration = audio_frames / 16000
        command = [str(args.helper), 'stream', '--model-path', str(args.model)]
        record = dict(command=command, realtime_paced=args.realtime, audio_frames=audio_frames,
                      audio_seconds=duration, platform=platform.platform(), machine=platform.machine(),
                      sha256={str(p): hashlib.sha256(p.read_bytes()).hexdigest()
                      for p in (Path(__file__), args.helper, args.audio, *sorted(args.model.rglob('*')))
                      if p.is_file()},
                      scope='paced file replay, not microphone/app Live; process RSS excludes external CoreML allocations')
        started = time.monotonic()
        turns, last, events, errors = [], {}, [], []
        latency = deque(maxlen=10_000)
        latency_peak, latency_count = [0.0], [0]
        ready = threading.Event()
        playback_start = [None]
        with (args.output/'stderr.log').open('wb') as stderr:
            process = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr)
            timed_out = threading.Event()

            def abort():
                timed_out.set()
                process.kill()

            timeout_seconds = duration + 90 if args.realtime else max(300, duration * 3 + 90)
            record['timeout_seconds'] = timeout_seconds
            watchdog = threading.Timer(timeout_seconds, abort)

            def read_events():
                try:
                    with (args.output/'events.jsonl').open('wb') as raw:
                        while line := process.stdout.readline(65537):
                            raw.write(line)
                            raw.flush()
                            if len(line) > 65536 or not line.endswith(b'\n'):
                                raise ValueError('oversized or incomplete event')
                            event = json.loads(line)
                            if event.get('version') != 1 or event.get('backend') != 'fluid_lseend':
                                raise ValueError('unsupported streaming protocol')
                            kind = event.get('type')
                            if kind == 'ready':
                                if ready.is_set():
                                    raise ValueError('duplicate ready event')
                                events.append(event)
                                ready.set()
                            elif kind == 'update':
                                if not ready.is_set():
                                    raise ValueError('update before readiness')
                                merge_turns(turns, last, event['speakers'])
                                if args.realtime and playback_start[0] is not None:
                                    end = min(duration, event['finalized_frames'] * events[0]['frame_seconds'])
                                    lag = max(0, time.monotonic() - playback_start[0] - end)
                                    latency.append(lag)
                                    latency_peak[0] = max(latency_peak[0], lag)
                                    latency_count[0] += 1
                            elif kind == 'done':
                                events.append(event)
                            else:
                                raise ValueError('unknown streaming event')
                except Exception as error:
                    errors.append(str(error))
                    process.kill()
                    ready.set()

            reader = threading.Thread(target=read_events, daemon=True)
            watchdog.start()
            reader.start()
            try:
                if not ready.wait(60) or errors or not events:
                    raise RuntimeError('helper did not become ready')
                playback_start[0] = time.monotonic()
                sent = 0
                while data := audio.readframes(1600):
                    pcm = array.array('h', data)
                    if sys.byteorder != 'little':
                        pcm.byteswap()
                    samples = array.array('f', (sample / 32768 for sample in pcm))
                    if sys.byteorder != 'little':
                        samples.byteswap()
                    sent += len(pcm)
                    if args.realtime:
                        time.sleep(max(0, playback_start[0] + sent / 16000 - time.monotonic()))
                    process.stdin.write(samples.tobytes())
                    process.stdin.flush()
                process.stdin.close()
                code = process.wait(timeout=90)
                reader.join(timeout=5)
                if reader.is_alive() or errors or timed_out.is_set() or code != 0:
                    raise RuntimeError(f'helper failed: code={code}, errors={errors}, timeout={timed_out.is_set()}')
                done = events[-1]
                if done['type'] != 'done' or done['samples_received'] != audio_frames or sent != audio_frames:
                    raise ValueError('stream did not acknowledge every input sample')
                frame_coverage = done['finalized_frames'] * events[0]['frame_seconds']
                if abs(frame_coverage - duration) > events[0]['frame_seconds'] + 0.00001:
                    raise ValueError('streaming tail exceeds one output-frame tolerance')
                if args.realtime and (not latency or latency_peak[0] > 2.0):
                    raise ValueError('speaker frame lag exceeded the fixed 2 second replay budget')
                record.update(status='PASS', input_samples_acknowledged=sent, done=done, ready=events[0],
                              output_coverage_seconds=frame_coverage,
                              maximum_frame_lag_seconds=latency_peak[0] if latency else None,
                              median_frame_lag_seconds=statistics.median(latency) if latency else None,
                              median_frame_lag_window_updates=len(latency),
                              frame_lag_total_updates=latency_count[0], maximum_allowed_frame_lag_seconds=2.0)
            except Exception as error:
                record.update(status='FAIL', error=str(error))
                raise
            finally:
                watchdog.cancel()
                if process.poll() is None:
                    process.kill()
                record['exit_code'] = process.wait()
                reader.join(timeout=5)
                process.stdout.close()
                if not process.stdin.closed:
                    process.stdin.close()
                record.update(wall_seconds=time.monotonic() - started,
                              process_peak_rss_bytes=resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss)
                (args.output/'result.json').write_text(json.dumps(record, indent=2)+'\n')
                (args.output/'prediction.json').write_text(json.dumps(dict(speakers=turns), indent=2)+'\n')
    print(json.dumps(record, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--helper', type=Path, required=True)
    parser.add_argument('--model', type=Path, required=True)
    parser.add_argument('--audio', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--realtime', action='store_true')
    args = parser.parse_args()
    if sys.platform != 'darwin':
        parser.error('this native helper replay currently requires macOS')
    run(args)


if __name__ == '__main__':
    main()
