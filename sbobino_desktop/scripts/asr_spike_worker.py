#!/usr/bin/env python3
"""Resident MLX-VLM / Photon subprocess for the isolated ASR spike."""

import argparse
import importlib.metadata
import json
import sys
import time


def emit(value):
    print(json.dumps(value, ensure_ascii=False), flush=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--engine", choices=("mlx", "redux"), required=True)
    parser.add_argument("--model", required=True)
    parser.add_argument("--device", choices=("cpu", "mps"), default="mps")
    parser.add_argument("--max-tokens", type=int, default=512)
    args = parser.parse_args()
    start = time.perf_counter()
    if args.engine == "mlx":
        import mlx.core as mx
        from mlx_vlm import generate, load
        from mlx_vlm.prompt_utils import apply_chat_template

        model, processor = load(args.model)

        def infer(request):
            audio = [request["audio_path"]] if request.get("audio_path") else None
            formatted = apply_chat_template(
                processor, model.config, request["prompt"], num_audios=len(audio or []),
                chat_template_kwargs={"enable_thinking": False},
            )
            result = generate(
                model, processor, prompt=formatted, audio=audio,
                max_tokens=args.max_tokens, temperature=0.0, verbose=False,
            )
            return {"text": result.text if hasattr(result, "text") else str(result),
                    "finish_reason": getattr(result, "finish_reason", None),
                    "prompt_tokens": getattr(result, "prompt_tokens", None),
                    "generation_tokens": getattr(result, "generation_tokens", None),
                    "mlx_peak_gpu_allocation_bytes": int(mx.metal.get_peak_memory())}

    else:
        import moondream as md
        import torch

        speech_context = md.photon(args.model, device=args.device)
        speech = speech_context.__enter__()

        def infer(request):
            result = speech.transcribe(audio=request["audio_path"], timestamps="word")
            answer = {"text": result["text"], "segments": result.get("segments", [])}
            if args.device == "mps":
                answer["mps_current_allocated_bytes"] = int(torch.mps.current_allocated_memory())
                answer["mps_driver_allocated_bytes"] = int(torch.mps.driver_allocated_memory())
            return answer

    packages = (["mlx-vlm", "mlx", "transformers"] if args.engine == "mlx" else
                ["moondream", "kestrel", "torch"])
    ready = {"ready": True, "load_seconds": time.perf_counter() - start,
             "python": sys.version.split()[0], "engine": args.engine, "model": args.model,
             "packages": {name: importlib.metadata.version(name) for name in packages}}
    if args.engine == "redux":
        ready.update(cpu_threads_requested=None, torch_num_threads=torch.get_num_threads(),
                     torch_num_interop_threads=torch.get_num_interop_threads())
    emit(ready)
    try:
        for line in sys.stdin:
            started = time.perf_counter()
            try:
                result = infer(json.loads(line))
                emit({"status": "ok", "elapsed_seconds": time.perf_counter() - started, **result})
            except Exception as error:
                emit({"status": "error", "elapsed_seconds": time.perf_counter() - started,
                      "error": f"{type(error).__name__}: {error}"})
    finally:
        if args.engine == "redux":
            speech_context.__exit__(None, None, None)


if __name__ == "__main__":
    main()
