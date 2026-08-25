#!/usr/bin/env python3
"""Exercise a patched whisper-stream binary with deterministic real-time WAV replay."""

from __future__ import annotations

import argparse
import array
import ctypes
import json
import os
import re
import subprocess
import threading
import time
import unicodedata
import wave
from pathlib import Path


def rss_mib(pid: int) -> float | None:
    if os.name != "nt":
        try:
            value = subprocess.check_output(
                ["ps", "-o", "rss=", "-p", str(pid)], text=True
            ).strip()
            return float(value) / 1024.0 if value else None
        except (OSError, subprocess.CalledProcessError, ValueError):
            return None

    class ProcessMemoryCounters(ctypes.Structure):
        _fields_ = [
            ("cb", ctypes.c_ulong),
            ("PageFaultCount", ctypes.c_ulong),
            ("PeakWorkingSetSize", ctypes.c_size_t),
            ("WorkingSetSize", ctypes.c_size_t),
            ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPagedPoolUsage", ctypes.c_size_t),
            ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
            ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
            ("PagefileUsage", ctypes.c_size_t),
            ("PeakPagefileUsage", ctypes.c_size_t),
        ]

    process = ctypes.windll.kernel32.OpenProcess(0x0400 | 0x0010, False, pid)
    if not process:
        return None
    try:
        counters = ProcessMemoryCounters()
        counters.cb = ctypes.sizeof(counters)
        if not ctypes.windll.psapi.GetProcessMemoryInfo(
            process, ctypes.byref(counters), counters.cb
        ):
            return None
        return counters.WorkingSetSize / (1024.0 * 1024.0)
    finally:
        ctypes.windll.kernel32.CloseHandle(process)


def finalized_transcript(stdout: str) -> str:
    final_lines: list[str] = []
    preview = ""
    records = re.split(r"(\r\n|\r|\n)", stdout)
    for index in range(0, len(records), 2):
        raw_record = records[index]
        separator = records[index + 1] if index + 1 < len(records) else ""
        has_redraw_marker = "[2K]" in raw_record or "\x1b[2K" in raw_record
        is_preview = separator == "\r" or (not separator and has_redraw_marker)
        cleaned = (
            raw_record.replace("\x1b[2K", "")
            .replace("\x1b[0m", "")
            .replace("[2K]", "")
            .replace("[BLANK_AUDIO]", "")
            .strip()
        )
        if not cleaned or cleaned in {"[Start speaking]", "[Start speaking...]"}:
            continue
        if is_preview:
            preview = cleaned
            continue
        # A newline is a finalized decoder record. Identical adjacent final
        # records may be two genuinely repeated utterances and must remain
        # visible to the loss/duplication gate. Only carriage-return previews
        # are replaceable UI redraws.
        final_lines.append(cleaned)
        preview = ""
    if preview and (not final_lines or final_lines[-1] != preview):
        final_lines.append(preview)
    return "\n".join(final_lines)


def final_runtime_summary(stderr: str, sample_rate: int) -> tuple[int, int] | None:
    matches = re.findall(
        r"SBOBINO_WHISPER_LIVE_METRICS\s+captured_seconds=([0-9.]+)\s+"
        r"processed_seconds=([0-9.]+)\s+backlog_seconds=([0-9.]+)\s+"
        r"dropped_samples=([0-9]+)",
        stderr,
    )
    if not matches:
        return None
    return round(float(matches[-1][0]) * sample_rate), int(matches[-1][3])


def final_preflight_result(stderr: str) -> dict[str, float | str] | None:
    matches = re.findall(
        r"SBOBINO_WHISPER_LIVE_PREFLIGHT\s+status=(passed|rejected)\s+"
        r"inference_ms=([0-9.]+)\s+budget_ms=([0-9.]+)\s+step_ms=([0-9]+)",
        stderr,
    )
    if not matches:
        return None
    status, inference_ms, budget_ms, step_ms = matches[-1]
    return {
        "status": status,
        "inference_ms": float(inference_ms),
        "budget_ms": float(budget_ms),
        "step_ms": float(step_ms),
    }


def backlog_threshold_overshoot(stderr: str, sample_rate: int) -> float | None:
    matches = re.findall(
        r"SBOBINO_WHISPER_LIVE_BACKLOG\s+exceeded\s+captured=([0-9]+)\s+"
        r"inferred=([0-9]+)\s+buffered=([0-9]+)",
        stderr,
    )
    if not matches:
        return None
    buffered = int(matches[-1][2])
    queued_seconds = buffered / sample_rate
    return max(0.0, queued_seconds - 2.0)


def captured_wav_paths(run_dir: Path, audio: Path, fixture: Path) -> list[Path]:
    excluded = {audio.resolve(), fixture.resolve()}
    return sorted(path for path in run_dir.glob("*.wav") if path.resolve() not in excluded)


def live_command_profile(device: str, available_cpus: int | None) -> tuple[int, int, int]:
    """Mirror the app's bounded thread count and CPU/GPU live window."""
    threads = max(1, min(8, available_cpus or 1))
    step_ms = 1280 if device == "cpu" else 1000
    return threads, step_ms, 2000


def preview_latency_seconds(step_ms: int, inference_ms: float) -> float:
    return (step_ms + inference_ms) / 1000.0


def first_voiced_frame(samples: list[int], sample_rate: int, threshold: float = 0.01) -> int:
    """Return the first 20 ms block whose PCM16 RMS crosses the speech floor."""
    block_size = max(1, sample_rate // 50)
    threshold_pcm = threshold * 32768.0
    for start in range(0, len(samples), block_size):
        block = samples[start : start + block_size]
        if not block:
            break
        rms = (sum(float(sample) * sample for sample in block) / len(block)) ** 0.5
        if rms >= threshold_pcm:
            return start
    return 0


def speech_onset_seconds(path: Path) -> float:
    with wave.open(str(path), "rb") as handle:
        if handle.getnchannels() != 1 or handle.getsampwidth() != 2:
            return 0.0
        sample_rate = handle.getframerate()
        pcm = array.array("h")
        pcm.frombytes(handle.readframes(handle.getnframes()))
    return first_voiced_frame(pcm.tolist(), sample_rate) / sample_rate


def count_fixture_utterances(normalized_transcript: str) -> int:
    """Count replayed utterances without treating harmless ASR wording as audio loss.

    The pinned fixture starts with "Well, I don't wish" and ends with "portrait".
    Whisper may repeat a short token across adjacent stream windows or vary words in
    between, so require both ordered boundary anchors without requiring an exact
    transcript. A truncated final fixture repetition is intentionally not complete.
    """
    tokens = normalized_transcript.split()
    anchor_tail = ("i", "don't", "wish")
    count = 0
    start = 0
    while start < len(tokens):
        if tokens[start] != "well":
            start += 1
            continue
        next_start = start + 1
        while next_start < len(tokens) and tokens[next_start] != "well":
            next_start += 1
        utterance = tokens[start:next_start]
        # A complete fixture utterance contains substantially more than its
        # opening/closing anchors. Keep the boundary local so a truncated
        # utterance cannot borrow evidence from the following replay.
        if len(utterance) < 18:
            start = next_start
            continue
        cursor = start + 1
        window_end = min(next_start, start + 9)
        for expected in anchor_tail:
            while cursor < window_end and tokens[cursor] != expected:
                cursor += 1
            if cursor >= window_end:
                break
            cursor += 1
        else:
            interior = (
                ("see",),
                ("eye", "eyes"),
                ("certain", "certainly", "true"),
                ("portrait",),
            )
            for alternatives in interior:
                while cursor < next_start and tokens[cursor] not in alternatives:
                    cursor += 1
                if cursor >= next_start:
                    break
                cursor += 1
            else:
                # Only punctuation-window repetition may trail the closing
                # anchor; substantial text after it is an unfinished/misaligned
                # utterance, not a complete replay.
                if next_start - cursor > 3:
                    start = next_start
                    continue
                count += 1
                start = next_start
                continue
        start = next_start
    return count


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--audio", type=Path, required=True)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--run-dir", type=Path, required=True)
    parser.add_argument("--device", choices=("auto", "cpu"), default="auto")
    parser.add_argument("--platform", required=True)
    expected_outcome = parser.add_mutually_exclusive_group()
    expected_outcome.add_argument("--expect-backlog-recovery", action="store_true")
    expected_outcome.add_argument("--expect-preflight-rejection", action="store_true")
    args = parser.parse_args()

    with wave.open(str(args.audio), "rb") as handle:
        input_frames = handle.getnframes()
        sample_rate = handle.getframerate()
        duration = input_frames / sample_rate
    with wave.open(str(args.fixture), "rb") as handle:
        fixture_duration = handle.getnframes() / handle.getframerate()
    speech_onset = speech_onset_seconds(args.audio)

    threads, step_ms, length_ms = live_command_profile(args.device, os.cpu_count())
    command = [
        str(args.binary), "-m", str(args.model), "-t", str(threads), "--step", str(step_ms),
        "--length", str(length_ms), "--no-fallback", "--save-audio", "-l", "auto",
    ]
    if args.device == "cpu":
        command.extend(["-ng", "-nfa"])

    environment = os.environ.copy()
    environment["SBOBINO_WHISPER_REPLAY_WAV"] = str(args.audio)
    if args.expect_backlog_recovery:
        # Test-only bypass: the recovery proof deliberately reaches capture and
        # injects a stall after the independent capability proof has exercised
        # the normal, non-bypassed path.
        environment["SBOBINO_WHISPER_SKIP_LIVE_PREFLIGHT"] = "1"
    started = time.monotonic()
    process = subprocess.Popen(
        command,
        cwd=args.run_dir,
        env=environment,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        bufsize=0,
    )
    stdout_chunks: list[str] = []
    stderr_chunks: list[str] = []
    first_preview_wall: float | None = None
    replay_started_wall: float | None = None
    last_metric_wall: float | None = None

    def drain(stream, target: list[str], is_stdout: bool = False) -> None:
        nonlocal first_preview_wall, replay_started_wall, last_metric_wall
        while True:
            chunk = os.read(stream.fileno(), 4096)
            if not chunk:
                return
            text = chunk.decode("utf-8", errors="replace")
            target.append(text)
            now = time.monotonic()
            if is_stdout and "[Start speaking]" in text:
                replay_started_wall = replay_started_wall or now
            if not is_stdout and "SBOBINO_WHISPER_LIVE_METRIC " in text:
                last_metric_wall = now
            if is_stdout and first_preview_wall is None:
                candidate = finalized_transcript(text)
                if candidate:
                    first_preview_wall = now

    readers = [
        threading.Thread(target=drain, args=(process.stdout, stdout_chunks, True), daemon=True),
        threading.Thread(target=drain, args=(process.stderr, stderr_chunks), daemon=True),
    ]
    for reader in readers:
        reader.start()

    rss_samples: list[float] = []
    deadline = started + max(180.0, duration * 3.0)
    while process.poll() is None and time.monotonic() < deadline:
        value = rss_mib(process.pid)
        # Model loading and backend warmup happen before capture begins. The
        # release contract measures sustained-session growth, not expected
        # one-time model allocation.
        if value is not None and replay_started_wall is not None:
            rss_samples.append(value)
        time.sleep(0.25)
    timed_out = process.poll() is None
    if timed_out:
        process.kill()
    return_code = process.wait()
    for reader in readers:
        reader.join(timeout=3)
    finished = time.monotonic()
    stderr = "".join(stderr_chunks)
    stdout = "".join(stdout_chunks)

    metric_pattern = re.compile(
        r"SBOBINO_WHISPER_LIVE_METRIC\s+"
        r"captured_seconds=(?P<captured>[0-9.]+)\s+"
        r"processed_seconds=(?P<processed>[0-9.]+)\s+"
        r"backlog_seconds=(?P<backlog>[0-9.]+)\s+"
        r"inference_ms=(?P<inference>[0-9.]+)\s+"
        r"dropped_samples=(?P<dropped>[0-9]+)"
    )
    samples = [
        {
            "captured_seconds": float(match.group("captured")),
            "processed_seconds": float(match.group("processed")),
            "backlog_seconds": float(match.group("backlog")),
            "inference_ms": float(match.group("inference")),
            "preview_latency_seconds": preview_latency_seconds(
                step_ms, float(match.group("inference"))
            ),
            "dropped_samples": int(match.group("dropped")),
        }
        for match in metric_pattern.finditer(stderr)
    ]
    output_text = finalized_transcript(stdout)
    saved_audio = captured_wav_paths(args.run_dir, args.audio, args.fixture)
    saved_frames = 0
    for path in saved_audio:
        with wave.open(str(path), "rb") as handle:
            saved_frames += handle.getnframes()
    normalized = unicodedata.normalize("NFKC", output_text).casefold()
    normalized = " ".join(re.findall(r"[^\W\d_]+(?:['’][^\W\d_]+)?", normalized))
    expected_segments = int(duration // fixture_duration)
    observed_segments = count_fixture_utterances(normalized)
    final_summary = final_runtime_summary(stderr, sample_rate)
    preflight = final_preflight_result(stderr)
    captured_frames = final_summary[0] if final_summary else 0
    final_dropped_samples = final_summary[1] if final_summary else None
    backlog_reaction_seconds = backlog_threshold_overshoot(stderr, sample_rate)
    failures: list[str] = []
    if timed_out:
        failures.append("whisper-stream timed out")
    if final_summary is None and not args.expect_preflight_rejection:
        failures.append("terminal runtime summary is missing")
    backlog_failure = "SBOBINO_WHISPER_LIVE_BACKLOG" in stderr
    if args.expect_preflight_rejection:
        if return_code != 8:
            failures.append(f"expected preflight exit 8, got {return_code}")
        if preflight is None:
            failures.append("live preflight result is missing")
        elif preflight["status"] != "rejected":
            failures.append(f"expected rejected preflight, got {preflight['status']}")
        if replay_started_wall is not None or "[Start speaking]" in stdout:
            failures.append("audio capture started after preflight rejection")
        if final_summary is not None or samples:
            failures.append("runtime telemetry was emitted after preflight rejection")
        if output_text:
            failures.append("preflight rejection emitted transcript text")
        if saved_audio or saved_frames:
            failures.append("preflight rejection created captured audio")
    elif args.expect_backlog_recovery:
        if return_code != 7:
            failures.append(f"expected backlog exit 7, got {return_code}")
        if not backlog_failure:
            failures.append("forced backlog was not reported")
        if not final_summary:
            failures.append("forced backlog is missing final capture counters")
        if backlog_reaction_seconds is None:
            failures.append("forced backlog is missing reaction timing")
        elif backlog_reaction_seconds > 0.05:
            failures.append(
                f"forced backlog reaction was late: {backlog_reaction_seconds:.3f}s"
            )
    else:
        if return_code != 0:
            failures.append(f"whisper-stream exited with status {return_code}")
        if preflight is None:
            failures.append("live preflight result is missing")
        elif preflight["status"] != "passed":
            failures.append(f"live preflight did not pass: {preflight['status']}")
        if backlog_failure:
            failures.append("whisper-stream reported a real-time backlog")
        if not output_text:
            failures.append("whisper-stream produced no finalized transcript")
    if not saved_audio and not args.expect_preflight_rejection:
        failures.append("whisper-stream did not preserve captured WAV")
    if final_dropped_samples not in (None, 0):
        failures.append(
            f"final runtime summary reported {final_dropped_samples} dropped samples"
        )
    expected_saved_frames = (
        0
        if args.expect_preflight_rejection
        else captured_frames
        if args.expect_backlog_recovery
        else input_frames
    )
    if saved_frames != expected_saved_frames:
        failures.append(
            f"saved WAV frame count mismatch: saved={saved_frames} expected={expected_saved_frames}"
        )

    payload = {
        "schema_version": 1,
        "status": "passed" if not failures else "failed",
        "engine": "whisper.cpp/whisper-stream",
        "platform": args.platform,
        "duration_seconds": duration,
        "requested_duration_seconds": duration,
        "captured_duration_seconds": captured_frames / sample_rate,
        "live_mode": (
            "preflight-rejected-incompatible-cpu"
            if args.expect_preflight_rejection
            else "backlog-recovery"
            if args.expect_backlog_recovery
            else "realtime"
        ),
        "samples": samples,
        "first_preview_seconds": 0.0 if args.expect_preflight_rejection else (
            max(0.0, first_preview_wall - replay_started_wall - speech_onset)
            if first_preview_wall is not None and replay_started_wall is not None
            else 9999.0
        ),
        "finalization_seconds": 0.0 if args.expect_preflight_rejection else (
            max(0.0, finished - last_metric_wall) if last_metric_wall is not None else 9999.0
        ),
        "rss_samples_mib": rss_samples,
        "dropped_samples": (
            0
            if args.expect_preflight_rejection
            else final_dropped_samples
            if final_dropped_samples is not None
            else -1
        ),
        "missing_segments": 0 if (args.expect_backlog_recovery or args.expect_preflight_rejection) else max(0, expected_segments - observed_segments),
        "duplicate_segments": 0 if (args.expect_backlog_recovery or args.expect_preflight_rejection) else max(0, observed_segments - expected_segments),
        "backlog_recovery_expected": args.expect_backlog_recovery,
        "preflight_rejection_expected": args.expect_preflight_rejection,
        "preflight_rejected": preflight is not None and preflight["status"] == "rejected",
        "preflight": preflight,
        "backlog_reaction_seconds": backlog_reaction_seconds,
        "captured_audio_frames": captured_frames,
        "saved_audio_frames": saved_frames,
        "stdout_transcript": output_text,
        "stderr_tail": stderr[-8000:],
        "failures": failures,
    }
    args.report.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    return 0 if not failures else 1


if __name__ == "__main__":
    raise SystemExit(main())
