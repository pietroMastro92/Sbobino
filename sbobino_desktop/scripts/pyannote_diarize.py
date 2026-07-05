#!/usr/bin/env python3

import argparse
import json
import os
import sys
import wave
from typing import Dict, List


PROGRESS_PREFIX = "SBOBINO_DIARIZATION_PROGRESS "
_last_progress = 0


def emit_progress(phase, percentage, message, completed=None, total=None):
    global _last_progress
    percentage = max(_last_progress, min(100, int(round(percentage))))
    _last_progress = percentage
    payload = {
        "phase": phase,
        "percentage": percentage,
        "completed": None if completed is None else int(completed),
        "total": None if total is None else int(total),
        "message": message,
    }
    sys.stderr.write(PROGRESS_PREFIX + json.dumps(payload) + "\n")
    sys.stderr.flush()


class SbobinoProgressHook:
    RANGES = {
        "segmentation": (5, 55, "Segmenting speech"),
        "speaker_counting": (55, 60, "Counting active speakers"),
        "embeddings": (60, 92, "Detecting speakers"),
        "discrete_diarization": (92, 98, "Clustering speaker turns"),
    }

    def __call__(self, step_name, _artifact, file=None, total=None, completed=None):
        if step_name not in self.RANGES:
            return
        start, end, message = self.RANGES[step_name]
        if completed is None or total in (None, 0):
            percentage = end
        else:
            percentage = start + (end - start) * min(1.0, completed / total)
        emit_progress(step_name, percentage, message, completed, total)


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
    parser.add_argument("--batch-size", type=int, default=16)
    args = parser.parse_args()

    try:
        os.nice(5)
    except OSError:
        pass

    try:
        import torch  # noqa: F401 - explicit dependency preflight check
        from pyannote.audio import Pipeline
    except Exception as error:
        sys.stderr.write(
            "pyannote dependencies are not available. Install torch and pyannote.audio in the configured Python environment.\n"
        )
        sys.stderr.write(f"{error}\n")
        return 1

    worker_threads = max(1, int(os.environ.get("OMP_NUM_THREADS", "2")))
    torch.set_num_threads(worker_threads)
    torch.set_num_interop_threads(1)
    emit_progress("loading_model", 0, "Loading speaker diarization model")

    input_payload = None
    last_error = None
    for device in resolve_device_candidates(args.device):
        try:
            if input_payload is None:
                input_payload = load_wav_input(args.audio_path)
            emit_progress("loading_model", 2, f"Loading diarization model for {device}")
            pipeline = Pipeline.from_pretrained(args.model_path)
            pipeline.to(device)
            batch_size = max(1, int(args.batch_size))
            if str(device) == "cpu":
                batch_size = min(batch_size, 8)
            pipeline.segmentation_batch_size = batch_size
            pipeline.embedding_batch_size = batch_size
            emit_progress("loading_model", 5, f"Diarization model ready on {device}")
            diarization = pipeline(input_payload, hook=SbobinoProgressHook())
            break
        except Exception as error:
            last_error = error
            if str(device) != "cpu":
                sys.stderr.write(
                    f"pyannote inference on {device} failed; retrying on cpu: {error}\n"
                )
                emit_progress("retrying_cpu", _last_progress, "Retrying diarization on CPU")
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
    emit_progress("finalizing", 100, "Speaker diarization completed")
    sys.stdout.write(json.dumps({"speakers": turns}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
