#!/usr/bin/env python3
"""Measure existing real service tests; compilation and downloads are excluded."""
import argparse
import ctypes
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import statistics
import subprocess
import time
import wave

from evaluate_asr_reference import normalize_text, evaluate
from types import SimpleNamespace
from run_whisper_live_replay import count_fixture_utterances


def descendants(rows, root):
    selected = {root}
    while True:
        added = {pid for pid, parent, _ in rows if parent in selected} - selected
        if not added:
            return selected
        selected.update(added)


def process_memory():
    if os.name != 'nt':
        text = subprocess.check_output(['ps', '-axo', 'pid=,ppid=,rss='], text=True)
        return [(int(pid), int(parent), int(rss) * 1024) for pid, parent, rss in (line.split() for line in text.splitlines())]
    # Native snapshots avoid starting PowerShell for every memory sample.
    from ctypes import wintypes as w
    class Entry(ctypes.Structure):
        _fields_ = [('size', w.DWORD), ('usage', w.DWORD), ('pid', w.DWORD), ('heap', ctypes.c_size_t), ('module', w.DWORD), ('threads', w.DWORD), ('parent', w.DWORD), ('priority', w.LONG), ('flags', w.DWORD), ('exe', w.WCHAR * 260)]
    class Memory(ctypes.Structure):
        _fields_ = [('cb', w.DWORD), ('faults', w.DWORD)] + [(name, ctypes.c_size_t) for name in ('peak', 'working', 'peak_pool', 'pool', 'peak_nonpaged', 'nonpaged', 'pagefile', 'peak_pagefile')]
    kernel = ctypes.WinDLL('kernel32', use_last_error=True)
    psapi = ctypes.WinDLL('psapi', use_last_error=True)
    kernel.CreateToolhelp32Snapshot.argtypes = [w.DWORD, w.DWORD]
    kernel.CreateToolhelp32Snapshot.restype = w.HANDLE
    kernel.Process32FirstW.argtypes = kernel.Process32NextW.argtypes = [w.HANDLE, ctypes.POINTER(Entry)]
    kernel.OpenProcess.argtypes = [w.DWORD, w.BOOL, w.DWORD]
    kernel.OpenProcess.restype = w.HANDLE
    kernel.CloseHandle.argtypes = [w.HANDLE]
    psapi.GetProcessMemoryInfo.argtypes = [w.HANDLE, ctypes.POINTER(Memory), w.DWORD]
    handle = kernel.CreateToolhelp32Snapshot(2, 0)
    if handle == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    rows = []
    try:
        entry = Entry(); entry.size = ctypes.sizeof(entry)
        available = kernel.Process32FirstW(handle, ctypes.byref(entry))
        while available:
            rss = None
            process = kernel.OpenProcess(0x0400 | 0x0010, False, entry.pid)
            if process:
                try:
                    memory = Memory(); memory.cb = ctypes.sizeof(memory)
                    if psapi.GetProcessMemoryInfo(process, ctypes.byref(memory), memory.cb):
                        rss = memory.working
                finally:
                    kernel.CloseHandle(process)
            rows.append((entry.pid, entry.parent, rss))
            available = kernel.Process32NextW(handle, ctypes.byref(entry))
    finally:
        kernel.CloseHandle(handle)
    return rows


def repeat_audio(source, target, repetitions):
    with wave.open(str(source), 'rb') as audio:
        assert audio.getnchannels() == 1 and audio.getsampwidth() == 2 and audio.getframerate() == 16000
        params, frames = audio.getparams(), audio.readframes(audio.getnframes())
    with wave.open(str(target), 'wb') as output:
        output.setparams(params)
        for _ in range(repetitions):
            output.writeframesraw(frames)
    return len(frames) / 32000 * repetitions


def run(binary, test, output, environment, timeout):
    command = [str(binary), test, '--ignored', '--exact', '--nocapture']
    record = {'command': command, 'status': 'failed', 'memory_method': 'sampled sum of root and live descendants RSS/working set; not an absolute allocation peak'}
    started = time.monotonic()
    with output.open('wb') as log:
        process = subprocess.Popen(command, env=environment, stdout=log, stderr=log)
        peak, samples, missing, seen = 0, 0, 0, {process.pid}
        try:
            while process.poll() is None:
                rows = process_memory()
                selected = descendants(rows, process.pid)
                seen.update(selected)
                values = [rss for pid, _, rss in rows if pid in selected]
                if values and all(value is not None for value in values):
                    peak = max(peak, sum(values)); samples += 1
                else:
                    missing += 1
                if time.monotonic() - started > timeout:
                    record['error'] = f'service exceeded {timeout}s'
                    raise TimeoutError(record['error'])
                time.sleep(0.1)
            record['exit_code'] = process.wait()
        finally:
            if process.poll() is None:
                if os.name == 'nt':
                    subprocess.run(['taskkill', '/PID', str(process.pid), '/T', '/F'], stdout=log, stderr=log, check=False)
                else:
                    for pid in seen:
                        try: os.kill(pid, 9)
                        except ProcessLookupError: pass
                process.kill(); process.wait()
            record.update(elapsed_seconds=time.monotonic()-started, sampled_peak_tree_mib=peak/1048576 if samples else None, memory_samples=samples, missing_memory_samples=missing)
            output.with_suffix('.measurement.json').write_text(json.dumps(record, indent=2)+'\n')
    text = output.read_text(errors='replace')
    assert record['exit_code'] == 0 and '1 passed; 0 failed' in text, 'real service test failed'
    assert samples > 0, 'no memory measurement'
    record['memory_samples_complete'] = missing == 0
    record['status'] = 'passed'
    output.with_suffix('.measurement.json').write_text(json.dumps(record, indent=2)+'\n')
    return record


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--audio', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    long_audio = args.output.parent.parent/'speech-12-repetitions.wav'
    long_seconds = repeat_audio(args.audio, long_audio, 12)
    assert hashlib.sha256(args.audio.read_bytes()).hexdigest() == '5fceacff0315d49cb59fcc505bcecf1ed5f2f35c2897b1e65a59f30e5d922150', 'reference audio identity mismatch'
    # Reuse the reviewed LibriSpeech reference and thresholds from the Intel smoke.
    reference_text = "Well, I don't wish to see it any more, observed Phebe, turning away her eyes. It is certainly very like the old portrait."
    quality_args = SimpleNamespace(language_tolerance_seconds=2.0, require_reviewed_reference=True, max_wer=0.35, max_cer=0.25, max_gap_seconds=2.0)
    summary = {'source_sha': os.environ['GITHUB_SHA'], 'platform': platform.platform(), 'architecture': platform.machine(), 'binary_sha256': hashlib.sha256(args.binary.read_bytes()).hexdigest(), 'input_sha256': hashlib.sha256(args.audio.read_bytes()).hexdigest(), 'long_sha256': hashlib.sha256(long_audio.read_bytes()).hexdigest(), 'long_seconds': long_seconds, 'scope': 'first invocation and cached-process restarts, not purged OS-cache cold start; production service with test repository; reviewed English WER/CER only, no Live or multilingual certification', 'engines': {}}
    try:
        for engine, test in [('whisper','whisper_service_real_smoke_preserves_timeline_and_plain_text'), ('parakeet','parakeet_service_real_smoke_persists_metadata')]:
            results = []
            for label in ['first','warm1','warm2','warm3','long']:
                env = os.environ.copy()
                env['SBOBINO_'+engine.upper()+'_AUDIO'] = str(long_audio if label=='long' else args.audio)
                timeline = args.output/f'{engine}-{label}-timeline.json'
                env['SBOBINO_ASR_TIMELINE_OUTPUT'] = str(timeline)
                record = run(args.binary, test, args.output/f'{engine}-{label}.log', env, 600)
                document = json.loads(timeline.read_text())
                segments = document['segments']; assert segments, 'empty timeline'
                previous = 0
                for segment in segments:
                    a,b=segment['start_seconds'],segment['end_seconds']
                    assert math.isfinite(a) and math.isfinite(b) and b>a and a>=previous
                    previous=b
                text=' '.join(segment['text'] for segment in segments)
                record['complete_fixture_utterances']=count_fixture_utterances(normalize_text(text))
                record['expected_fixture_utterances']=12 if label=='long' else 1
                record['label']=label
                # Anchors detect gross repetition loss; they are not a reviewed word-level reference.
                assert record['complete_fixture_utterances']==record['expected_fixture_utterances'], 'fixture utterance loss/duplication'
                count = record['expected_fixture_utterances']
                reference = {'audio_sha256':summary['long_sha256'] if label=='long' else summary['input_sha256'], 'review_status':'reviewed', 'reference_source':'release_intel_pyannote_parakeet_smoke.sh; parakeet.cpp 9edf17c3 LibriSpeech 2086-149220-0033 NeMo TDT reference', 'duration_seconds':long_seconds if label=='long' else long_seconds/12, 'segments':[{'start_seconds':i*long_seconds/12, 'end_seconds':(i+1)*long_seconds/12, 'language_code':'en', 'text':reference_text} for i in range(count)]}
                quality = evaluate(reference, document, quality_args)
                (args.output/f'{engine}-{label}-reference.json').write_text(json.dumps(reference,indent=2)+'\n')
                (args.output/f'{engine}-{label}-quality.json').write_text(json.dumps(quality,indent=2)+'\n')
                record['quality']=quality
                results.append(record)
                assert quality['status']=='passed', f"quality failed: {quality['failures']}"
                summary['engines'][engine]={'runs':results}
            summary['engines'][engine]['warm_median_seconds']=statistics.median(r['elapsed_seconds'] for r in results[1:4])
        summary['status']='passed'
    finally:
        summary.setdefault('status','failed')
        (args.output/'benchmark-summary.json').write_text(json.dumps(summary,indent=2)+'\n')

if __name__ == '__main__':
    main()
