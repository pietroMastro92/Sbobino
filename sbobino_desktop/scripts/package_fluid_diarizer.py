#!/usr/bin/env python3
"""Embed the verified native diarizer after Tauri builds and seals its existing sidecars."""
import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import platform
import shutil
import subprocess
import sys
import tempfile

from prepare_fluid_diarizer_models import install, MANIFEST


def inventory(root):
    files = []
    for path in sorted(root.rglob('*')):
        if path.is_symlink():
            raise ValueError(f'symlink in native bundle: {path}')
        if path.is_file() and path != root/'manifest.json':
            files.append(dict(path=path.relative_to(root).as_posix(), bytes=path.stat().st_size,
                              sha256=hashlib.sha256(path.read_bytes()).hexdigest()))
    return files


def verify_bundle(root):
    manifest = json.loads((root/'manifest.json').read_text())
    if manifest['version'] != 1 or manifest['files'] != inventory(root):
        raise ValueError('native bundle inventory or hash mismatch; preserved')
    return manifest


def stage_bundle(binary, models, model_manifest, destination, identity, sdk_license):
    (destination/'bin').mkdir(parents=True)
    helper = destination/'bin/sbobino-diarizer'
    shutil.copy2(binary, helper)
    helper.chmod(0o755)
    subprocess.run(['lipo', str(helper), '-verify_arch', platform.machine()], check=True)
    subprocess.run(['codesign', '--force', '--sign', identity, str(helper)], check=True)
    (destination/'licenses').mkdir()
    shutil.copyfile(sdk_license, destination/'licenses/FluidAudio-LICENSE')
    shutil.copytree(sdk_license.parent/'ThirdPartyLicenses', destination/'licenses/FluidAudio-ThirdParty')
    model_destination = destination/'models/speaker-diarization'
    model_destination.mkdir(parents=True)
    for entry in model_manifest['files']:
        path = PurePosixPath(entry['path'])
        if path.is_absolute() or '..' in path.parts:
            raise ValueError('unsafe model inventory')
        target = model_destination/entry['path']
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(models/entry['path'], target)
    shutil.copyfile(models/'.fluidaudio-revision', model_destination/'.fluidaudio-revision')
    manifest = dict(version=1, sdk_revision=model_manifest['fluid_audio_revision'],
                    model_revision=model_manifest['revision'], architecture=platform.machine(),
                    minimum_macos_major=15, files=inventory(destination))
    (destination/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
    verify_bundle(destination)


def seal_app(app, bundle, identity, temporary):
    candidate = temporary/'candidate.app'
    shutil.copytree(app, candidate, symlinks=True)
    destination = candidate/'Contents/Resources/fluid-diarizer'
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        verify_bundle(destination)
        shutil.rmtree(destination)
    subprocess.run(['codesign', '--force', '--deep', '--sign', identity, str(candidate)], check=True)
    shutil.move(str(bundle), destination)
    # Deep signing here would change the helper bytes covered by the manifest.
    subprocess.run(['codesign', '--force', '--sign', identity, str(candidate)], check=True)
    subprocess.run(['codesign', '--verify', '--deep', '--strict', str(candidate)], check=True)
    verify_bundle(destination)
    previous = temporary/'previous.app'
    app.rename(previous)
    try:
        candidate.rename(app)
    except BaseException:
        previous.rename(app)
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--app', type=Path, required=True)
    parser.add_argument('--build-root', type=Path, required=True)
    parser.add_argument('--identity', default='-')
    args = parser.parse_args()
    if sys.platform != 'darwin' or not (args.app/'Contents/MacOS/sbobino-desktop').is_file():
        parser.error('requires a built native Sbobino.app on macOS')
    if int(platform.mac_ver()[0].split('.')[0]) < 15:
        parser.error('building the bundled FluidAudio runtime requires macOS 15+')
    if not args.build_root.is_absolute():
        parser.error('--build-root must be absolute')
    scripts = Path(__file__).resolve().parent
    subprocess.run(['bash', str(scripts/'test_fluid_diarizer_native.sh'), str(args.build_root)], check=True)
    model_manifest = json.loads(MANIFEST.read_text())
    models = install(args.build_root/'models/offline', model_manifest)
    resources = args.app/'Contents/Resources'
    destination = resources/'fluid-diarizer'
    with tempfile.TemporaryDirectory(dir=args.build_root, prefix='fluid-bundle-') as temporary:
        stage = Path(temporary)/'fluid-diarizer'
        stage_bundle(args.build_root/'swift-build/release/sbobino-diarizer', models,
                     model_manifest, stage, args.identity,
                     args.build_root/'swift-build/checkouts/FluidAudio/LICENSE')
        seal_app(args.app, stage, args.identity, Path(temporary))
    print(json.dumps(dict(bundle=str(destination), manifest=verify_bundle(destination))))


if __name__ == '__main__':
    main()
