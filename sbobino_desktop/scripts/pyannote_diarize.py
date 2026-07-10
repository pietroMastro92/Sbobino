#!/usr/bin/env python3

import argparse
import json
import os
import sys
import wave
from pathlib import Path
from typing import Dict, List


_DLL_DIRECTORY_HANDLES = []
if sys.platform == "win32":
    dll_dir = Path(sys.executable).resolve().parent / "DLLs"
    if dll_dir.is_dir():
        _DLL_DIRECTORY_HANDLES.append(os.add_dll_directory(str(dll_dir)))


def resolve_device_candidates(requested: str):
    import torch

    value = (requested or "cpu").strip().lower()
    if value == "auto":
        if torch.backends.mps.is_available():
            return [torch.device("mps"), torch.device("cpu")]
        if torch.cuda.is_available():
            return [torch.device("cuda"), torch.device("cpu")]
        return [torch.device("cpu")]
    if value == "mps" and torch.backends.mps.is_available():
        return [torch.device("mps"), torch.device("cpu")]
    if value == "cuda" and torch.cuda.is_available():
        return [torch.device("cuda"), torch.device("cpu")]
    return [torch.device("cpu")]


def resolve_annotation(diarization):
    if hasattr(diarization, "exclusive_speaker_diarization"):
        annotation = diarization.exclusive_speaker_diarization
        if annotation is not None:
            return annotation

    if hasattr(diarization, "speaker_diarization"):
        annotation = diarization.speaker_diarization
        if annotation is not None:
            return annotation

    return diarization


def load_wav_input(audio_path: str):
    import numpy as np
    import torch

    with wave.open(audio_path, "rb") as wav_file:
        if wav_file.getcomptype() != "NONE":
            raise ValueError(
                "compressed WAV input is not supported for offline diarization"
            )

        channels = wav_file.getnchannels()
        sample_width = wav_file.getsampwidth()
        sample_rate = wav_file.getframerate()
        frame_count = wav_file.getnframes()
        raw = wav_file.readframes(frame_count)

    if channels <= 0 or sample_rate <= 0:
        raise ValueError("invalid WAV metadata")

    if sample_width == 1:
        samples = (np.frombuffer(raw, dtype=np.uint8).astype(np.float32) - 128.0) / 128.0
    elif sample_width == 2:
        samples = np.frombuffer(raw, dtype="<i2").astype(np.float32) / 32768.0
    elif sample_width == 3:
        data = np.frombuffer(raw, dtype=np.uint8).reshape(-1, 3)
        samples = (
            data[:, 0].astype(np.int32)
            | (data[:, 1].astype(np.int32) << 8)
            | (data[:, 2].astype(np.int32) << 16)
        )
        samples = np.where(samples & 0x800000, samples - 0x1000000, samples)
        samples = samples.astype(np.float32) / 8388608.0
    elif sample_width == 4:
        samples = np.frombuffer(raw, dtype="<i4").astype(np.float32) / 2147483648.0
    else:
        raise ValueError(f"unsupported WAV sample width: {sample_width}")

    if channels > 1:
        samples = samples.reshape(-1, channels).transpose()
    else:
        samples = samples.reshape(1, -1)

    waveform = torch.from_numpy(np.ascontiguousarray(samples))
    return {"waveform": waveform, "sample_rate": sample_rate}


def main() -> int:
    parser = argparse.ArgumentParser(description="Run speaker diarization with pyannote.audio")
    parser.add_argument("--audio-path", required=True)
    parser.add_argument("--model-path", required=True)
    parser.add_argument("--device", default="cpu")
    args = parser.parse_args()

    try:
        import torch  # noqa: F401 - explicit dependency preflight check
        from pyannote.audio import Pipeline
    except Exception as error:
        sys.stderr.write(
            "pyannote dependencies are not available. Install torch and pyannote.audio in the configured Python environment.\n"
        )
        sys.stderr.write(f"{error}\n")
        return 1

    input_payload = None
    last_error = None
    for device in resolve_device_candidates(args.device):
        try:
            if input_payload is None:
                input_payload = load_wav_input(args.audio_path)
            pipeline = Pipeline.from_pretrained(args.model_path)
            pipeline.to(device)
            diarization = pipeline(input_payload)
            break
        except Exception as error:
            last_error = error
            if str(device) != "cpu":
                sys.stderr.write(
                    f"pyannote inference on {device} failed; retrying on cpu: {error}\n"
                )
                continue
            sys.stderr.write(f"pyannote inference failed: {error}\n")
            return 1
    else:
        sys.stderr.write(f"pyannote inference failed: {last_error}\n")
        return 1

    annotation = resolve_annotation(diarization)
    if not hasattr(annotation, "itertracks"):
        sys.stderr.write(
            f"pyannote inference returned unsupported annotation type: {type(annotation).__name__}\n"
        )
        return 1

    speaker_order: Dict[str, int] = {}
    turns: List[dict] = []

    for turn, _, backend_speaker in annotation.itertracks(yield_label=True):
        if backend_speaker not in speaker_order:
            speaker_order[backend_speaker] = len(speaker_order) + 1
        index = speaker_order[backend_speaker]
        turns.append(
            {
                "speaker_id": f"speaker_{index}",
                "speaker_label": f"Speaker {index}",
                "start_seconds": float(turn.start),
                "end_seconds": float(turn.end),
                "backend_speaker": backend_speaker,
            }
        )

    turns.sort(key=lambda item: (item["start_seconds"], item["end_seconds"]))
    sys.stdout.write(json.dumps({"speakers": turns}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
