#!/usr/bin/env python3
"""Native CLI inference on a pinned public recording; no microphone or private profile."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import platform
import subprocess
import sys
import time
import urllib.request
import wave

from prepare_fluid_diarizer_models import install, MANIFEST


def validate_output(output, duration):
    if output.get('version') != 1 or output.get('backend') != 'fluid_offline':
        raise ValueError('unsupported native helper protocol')
    turns = output['speakers']
    if len({turn['speaker_id'] for turn in turns}) < 2:
        raise ValueError('public multi-speaker fixture produced fewer than two speakers')
    for turn in turns:
        start, end = turn['start_seconds'], turn['end_seconds']
        if not (math.isfinite(start) and math.isfinite(end) and 0 <= start < end <= duration):
            raise ValueError('speaker timestamp outside actual recording')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != 'darwin':
        parser.error('requires native macOS 15+')
    root = args.output
    root.mkdir(parents=True, exist_ok=True)
    corpus = json.loads(Path(__file__).with_name('fixtures').joinpath('fluid_diarization_public_corpus.json').read_text())
    manifest = json.loads(MANIFEST.read_text())
    model_root = root/'models/offline'
    install(model_root, manifest)
    corpus_root = root/'corpus'
    corpus_root.mkdir(exist_ok=True)
    full = corpus_root/'ES2004a.Mix-Headset.wav'
    if not full.exists():
        with urllib.request.urlopen(corpus['audio']['url'], timeout=120) as response:
            data = response.read(corpus['audio']['bytes']+1)
        if len(data) != corpus['audio']['bytes'] or hashlib.sha256(data).hexdigest() != corpus['audio']['sha256']:
            raise ValueError('public audio integrity check failed')
        full.write_bytes(data)
    if hashlib.sha256(full.read_bytes()).hexdigest() != corpus['audio']['sha256']:
        raise ValueError('existing public audio has changed; preserved')
    excerpt = corpus_root/'AMI-ES2004a-120s.wav'
    if not excerpt.exists():
        with wave.open(str(full)) as audio:
            audio.setpos(corpus['excerpt']['offset_seconds'] * audio.getframerate())
            with wave.open(str(excerpt), 'wb') as output:
                output.setparams(audio.getparams())
                output.writeframes(audio.readframes(corpus['excerpt']['duration_seconds'] * audio.getframerate()))
    if hashlib.sha256(excerpt.read_bytes()).hexdigest() != corpus['excerpt']['sha256']:
        raise ValueError('public excerpt identity mismatch; preserved')
    helper = root/'swift-build/release/sbobino-diarizer'
    command = [str(helper), 'offline', '--audio-path', str(excerpt), '--models-dir', str(model_root), '--compute', 'cpu']
    record = dict(command=command, platform=platform.platform(), machine=platform.machine(),
                  model_revision=manifest['revision'], verified_model_files=len(manifest['files']),
                  sha256={str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in (helper, excerpt, MANIFEST, Path(__file__))},
                  scope='native CPU CLI inference only; public English AMI fixture, not accuracy/app/Live certification')
    start = time.monotonic()
    with (root/'offline-smoke.json').open('wb') as out, (root/'offline-smoke.stderr.log').open('wb') as err:
        try:
            code = subprocess.run(command, stdout=out, stderr=err, timeout=180).returncode
        except subprocess.TimeoutExpired:
            code = 'TIMEOUT'
    record.update(exit_code=code, wall_seconds=time.monotonic()-start, status='FAIL')
    try:
        if code != 0:
            raise RuntimeError(f'native helper exited {code}')
        validate_output(json.loads((root/'offline-smoke.json').read_text()), 120)
        record['status'] = 'PASS'
    finally:
        (root/'offline-smoke-receipt.json').write_text(json.dumps(record, indent=2)+'\n')
    print(json.dumps(record))


if __name__ == '__main__':
    main()
