#!/usr/bin/env python3
"""Explicit build/setup download of pinned FluidAudio files; never purge an existing cache."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import re
import tempfile
import urllib.parse
import urllib.request


MANIFEST = Path(__file__).with_name('fluid_diarizer')/'offline-model-manifest.json'
SDK_REVISION = next(pin['state']['revision'] for pin in json.loads(MANIFEST.with_name('Package.resolved').read_text())['pins']
                    if pin['identity'] == 'fluidaudio')
REQUIRED_PATHS = {f'{name}.mlmodelc/coremldata.bin' for name in ('Embedding', 'FBank', 'PldaRho', 'Segmentation')} | {'plda-parameters.json'}


def validate_manifest(manifest):
    if manifest.get('fluid_audio_revision') != SDK_REVISION:
        raise ValueError('model inventory does not match the pinned FluidAudio SDK')
    if manifest['repository'] != 'FluidInference/speaker-diarization-coreml':
        raise ValueError('unexpected model repository')
    if manifest['installed_repository_folder'] != 'speaker-diarization':
        raise ValueError('unexpected SDK cache folder')
    if not re.fullmatch('[0-9a-f]{40}', manifest['revision']) or not manifest['files']:
        raise ValueError('an immutable revision and nonempty file inventory are required')
    paths = set()
    for entry in manifest['files']:
        path = PurePosixPath(entry['path'])
        if (path.is_absolute() or '..' in path.parts or not path.parts or str(path) != entry['path']
                or '\\' in entry['path'] or ':' in entry['path']):
            raise ValueError('unsafe model path')
        if entry['path'] in paths or not re.fullmatch('[0-9a-f]{64}', entry['sha256']) or entry['bytes'] <= 0:
            raise ValueError('invalid model inventory')
        paths.add(entry['path'])
    if not REQUIRED_PATHS.issubset(paths):
        raise ValueError('model inventory is missing required offline artifacts')


def verify(root, manifest):
    for path in REQUIRED_PATHS:
        if path.endswith('/coremldata.bin') and any((root/path).parent.rglob('*.partial')):
            raise ValueError('incomplete compiled model download; existing cache preserved')
    for entry in manifest['files']:
        path = root/entry['path']
        if (path.is_symlink() or not path.is_file() or path.stat().st_size != entry['bytes']
                or hashlib.sha256(path.read_bytes()).hexdigest() != entry['sha256']):
            raise ValueError(f'model integrity check failed: {path}')


def install(root, manifest):
    validate_manifest(manifest)
    root.mkdir(parents=True, exist_ok=True)
    destination = root/manifest['installed_repository_folder']
    lock = root/'.fluid-diarizer-install.lock'
    lock.mkdir()  # Fail closed if another installer is active; never remove someone else's lock.
    try:
        if destination.exists():
            if destination.is_symlink():
                raise ValueError('model cache must not be a symlink')
            verify(destination, manifest)
            marker = destination/'.fluidaudio-revision'
            if marker.exists():
                if marker.read_text().strip() != manifest['revision']:
                    raise ValueError('model revision mismatch; existing cache preserved')
                return destination
            # Adopt a complete cache only after every file matches the immutable manifest.
            with tempfile.NamedTemporaryFile(dir=destination, prefix='.revision-', delete=False) as staged:
                staged.write((manifest['revision']+'\n').encode())
            try:
                Path(staged.name).replace(marker)
            finally:
                Path(staged.name).unlink(missing_ok=True)
            return destination
        with tempfile.TemporaryDirectory(dir=root, prefix='.fluid-diarizer-') as temporary:
            stage = Path(temporary)/'speaker-diarization'
            stage.mkdir()
            for entry in manifest['files']:
                target = stage/entry['path']
                target.parent.mkdir(parents=True, exist_ok=True)
                url = (f"https://huggingface.co/{manifest['repository']}/resolve/{manifest['revision']}/"
                       + urllib.parse.quote(entry['path'], safe='/'))
                with urllib.request.urlopen(url, timeout=120) as response:
                    data = response.read(entry['bytes']+1)
                if len(data) != entry['bytes'] or hashlib.sha256(data).hexdigest() != entry['sha256']:
                    raise ValueError(f'download integrity check failed: {entry["path"]}')
                target.write_bytes(data)
            verify(stage, manifest)
            (stage/'.fluidaudio-revision').write_text(manifest['revision']+'\n')
            if destination.exists():
                raise ValueError('destination appeared during installation; preserved')
            stage.rename(destination)
        return destination
    finally:
        lock.rmdir()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    manifest = json.loads(MANIFEST.read_text())
    destination = install(args.output, manifest)
    print(json.dumps(dict(path=str(destination), revision=manifest['revision'],
                          verified_files=len(manifest['files']))))


if __name__ == '__main__':
    main()
