#!/usr/bin/env python3
"""Prepare a reproducible public corpus for the Gemma/Redux ASR spike.

The inference runner should consume only ``manifest.json``'s audio fields.
Reference text and timing are written separately to ``references.json`` for the
scorer, with explicit status and checksums so a runner cannot consume them.

FLEURS is loaded from the public Hugging Face dataset when no local source is
given.  mTEDx and AMI are deliberately local-only: pass an archive or
directory explicitly rather than making an accidental multi-gigabyte download.
"""

from __future__ import annotations

import argparse
import contextlib
import csv
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tarfile
import tempfile
import wave
import zipfile
from dataclasses import dataclass, replace
from pathlib import Path
from typing import Any, Iterator, Mapping, Sequence
from xml.etree import ElementTree


SCHEMA_VERSION = 1
SAMPLE_RATE = 16_000
AUDIO_EXTENSIONS = frozenset({".aac", ".flac", ".m4a", ".mp3", ".ogg", ".wav", ".webm"})
TEXT_EXTENSIONS = (".txt", ".srt", ".vtt")
SPLIT_ALIASES = {
    "dev": "dev",
    "development": "dev",
    "validation": "dev",
    "valid": "dev",
    "test": "test",
    "testing": "test",
    "train": "train",
}


class CorpusError(RuntimeError):
    """An actionable input or preparation error."""


@dataclass(frozen=True)
class Record:
    corpus: str
    split: str
    source_id: str
    audio_path: Path
    reference: str
    reference_source: str
    reference_status: str
    source_audio_sha256: str = ""
    alignment: tuple[dict[str, Any], ...] = ()


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sha256_text(text: str) -> str:
    return sha256_bytes(text.encode("utf-8"))


def _nonempty_text(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, str):
        return value.strip()
    return str(value).strip()


def _reference_from_mapping(row: Mapping[str, Any]) -> str:
    for key in ("raw_transcription", "transcription", "reference", "text", "sentence"):
        value = _nonempty_text(row.get(key))
        if value:
            return value
    return ""


def _canonical_split(value: Any, fallback: str = "test") -> str:
    text = _nonempty_text(value).lower().replace("_", "-")
    return SPLIT_ALIASES.get(text, fallback)


def _safe_relative(path: Path, root: Path) -> str:
    try:
        return path.resolve().relative_to(root.resolve()).as_posix()
    except ValueError:
        return path.name


def _audio_path_from_mapping(row: Mapping[str, Any], root: Path) -> Path | None:
    value = row.get("audio")
    if isinstance(value, Mapping):
        value = value.get("path")
    if value is None:
        for key in ("audio_path", "path", "file", "filename"):
            if row.get(key) is not None:
                value = row[key]
                break
    if not isinstance(value, (str, os.PathLike)):
        return None
    candidate = Path(value).expanduser()
    if not candidate.is_absolute():
        candidate = root / candidate
    return candidate.resolve()


_VTT_TIME = re.compile(
    r"^(?P<start>(?:\d+:)?\d{2}:\d{2}[.,]\d{3})\s+-->\s+"
    r"(?P<end>(?:\d+:)?\d{2}:\d{2}[.,]\d{3})"
)


def _caption_time(value: str) -> float:
    fields = value.replace(",", ".").split(":")
    if len(fields) == 2:
        hours = 0.0
        minutes, seconds = fields
    elif len(fields) == 3:
        hours, minutes, seconds = fields
    else:
        raise CorpusError(f"invalid WebVTT timestamp: {value}")
    return float(hours) * 3600.0 + float(minutes) * 60.0 + float(seconds)


def _parse_vtt_segments(path: Path) -> list[dict[str, Any]]:
    blocks = path.read_text(encoding="utf-8-sig", errors="replace").replace("\r\n", "\n").split("\n\n")
    segments: list[dict[str, Any]] = []
    for block in blocks:
        lines = [line.strip() for line in block.splitlines() if line.strip()]
        time_index = next((index for index, line in enumerate(lines) if _VTT_TIME.match(line)), None)
        if time_index is None:
            continue
        match = _VTT_TIME.match(lines[time_index])
        assert match is not None
        text = " ".join(lines[time_index + 1 :]).strip()
        text = re.sub(r"<[^>]+>", "", text)
        if not text:
            continue
        start = _caption_time(match.group("start"))
        end = _caption_time(match.group("end"))
        if end <= start:
            raise CorpusError(f"WebVTT segment has non-positive duration in {path}: {lines[time_index]}")
        segments.append({"start": round(start, 3), "end": round(end, 3), "text": text})
    if not segments:
        raise CorpusError(f"no timed transcript cues found in {path}")
    return segments


def _parse_caption_text(path: Path) -> str:
    if path.suffix.lower() == ".vtt":
        return " ".join(segment["text"] for segment in _parse_vtt_segments(path))
    text = path.read_text(encoding="utf-8-sig", errors="replace")
    lines: list[str] = []
    for line in text.splitlines():
        stripped = line.strip()
        if not stripped or stripped.upper() == "WEBVTT":
            continue
        if stripped.isdigit() or " --> " in stripped:
            continue
        lines.append(stripped)
    return " ".join(lines).strip()


def _sidecar_for(audio: Path) -> Path | None:
    for suffix in TEXT_EXTENSIONS:
        candidate = audio.with_suffix(suffix)
        if candidate.is_file():
            return candidate
    return None


def _infer_split(path: Path, root: Path, fallback: str = "test") -> str:
    relative = path.relative_to(root)
    parts = [part.lower() for part in relative.parts[:-1]] + [relative.stem.lower()]
    for part in reversed(parts):
        if part in SPLIT_ALIASES:
            return SPLIT_ALIASES[part]
    return fallback


def _metadata_candidates(root: Path) -> list[Path]:
    preferred = {
        "manifest.json",
        "manifest.jsonl",
        "metadata.json",
        "metadata.jsonl",
        "data.jsonl",
        "dev.jsonl",
        "development.jsonl",
        "validation.jsonl",
        "test.jsonl",
    }
    files = sorted(
        path
        for path in root.rglob("*")
        if path.is_file() and path.suffix.lower() in {".json", ".jsonl", ".ndjson", ".tsv"}
    )
    chosen = [path for path in files if path.name.lower() in preferred]
    return chosen or files


def _read_metadata_rows(path: Path) -> Iterator[Mapping[str, Any]]:
    suffix = path.suffix.lower()
    if suffix in {".jsonl", ".ndjson"}:
        with path.open(encoding="utf-8-sig") as handle:
            for line_number, line in enumerate(handle, 1):
                if not line.strip():
                    continue
                try:
                    row = json.loads(line)
                except json.JSONDecodeError as error:
                    raise CorpusError(f"invalid JSON in {path}:{line_number}: {error}") from error
                if not isinstance(row, Mapping):
                    raise CorpusError(f"metadata row in {path}:{line_number} is not an object")
                yield row
        return

    if suffix == ".json":
        try:
            payload = json.loads(path.read_text(encoding="utf-8-sig"))
        except json.JSONDecodeError as error:
            raise CorpusError(f"invalid JSON in {path}: {error}") from error
        if isinstance(payload, list):
            rows = payload
        elif isinstance(payload, Mapping):
            rows = payload.get("samples", payload.get("data", []))
        else:
            rows = []
        if not isinstance(rows, list):
            raise CorpusError(f"JSON metadata in {path} must be a list or contain samples/data")
        for row in rows:
            if not isinstance(row, Mapping):
                raise CorpusError(f"metadata row in {path} is not an object")
            yield row
        return

    with path.open(newline="", encoding="utf-8-sig") as handle:
        for row in csv.DictReader(handle, delimiter="\t"):
            yield row


def _records_from_local_metadata(root: Path, corpus: str) -> list[Record]:
    records: list[Record] = []
    for metadata_path in _metadata_candidates(root):
        try:
            rows = list(_read_metadata_rows(metadata_path))
        except CorpusError:
            if metadata_path.name.lower() in {"manifest.json", "manifest.jsonl", "metadata.json", "metadata.jsonl"}:
                raise
            continue
        for index, row in enumerate(rows):
            audio_path = _audio_path_from_mapping(row, metadata_path.parent)
            reference = _reference_from_mapping(row)
            if audio_path is None or not reference:
                continue
            if not audio_path.is_file():
                raise CorpusError(f"metadata points to missing audio: {audio_path}")
            source_value = row.get("id") or row.get("audio_path") or row.get("path") or audio_path.name
            source_id = _nonempty_text(source_value) or f"row-{index:06d}"
            split = _canonical_split(row.get("split"), _infer_split(metadata_path, root))
            records.append(
                Record(
                    corpus=corpus,
                    split=split,
                    source_id=source_id,
                    audio_path=audio_path,
                    reference=reference,
                    reference_source=f"local:{_safe_relative(metadata_path, root)}",
                    reference_status="provided-local",
                    source_audio_sha256=sha256_file(audio_path),
                )
            )
    return records


def _records_from_local_sidecars(root: Path, corpus: str) -> list[Record]:
    records: list[Record] = []
    audio_files = sorted(
        path for path in root.rglob("*") if path.is_file() and path.suffix.lower() in AUDIO_EXTENSIONS
    )
    for audio_path in audio_files:
        sidecar = _sidecar_for(audio_path)
        if sidecar is None:
            continue
        reference = _parse_caption_text(sidecar)
        if not reference:
            raise CorpusError(f"empty transcript sidecar: {sidecar}")
        relative = _safe_relative(audio_path, root)
        records.append(
            Record(
                corpus=corpus,
                split=_infer_split(audio_path, root),
                source_id=relative,
                audio_path=audio_path,
                reference=reference,
                reference_source=f"local:{_safe_relative(sidecar, root)}",
                reference_status="provided-local",
                source_audio_sha256=sha256_file(audio_path),
            )
        )
    return records


def _records_from_mtedx(root: Path) -> list[Record]:
    """Read the official SLR100 ``data/<split>/{wav,vtt}`` layout.

    mTEDx stores full-talk FLAC files and sentence-level WebVTT cues.  Keep one
    record per talk so long-form behavior is measured; the cue timing is kept in
    the matching entry in ``references.json``.
    """
    records: list[Record] = []
    seen: set[Path] = set()
    for wav_dir in sorted(path for path in root.rglob("wav") if path.is_dir()):
        data_dir = wav_dir.parent.parent
        if data_dir.name != "data":
            continue
        split = _canonical_split(wav_dir.parent.name, fallback="")
        if split not in {"dev", "test"}:
            continue
        vtt_dir = wav_dir.parent / "vtt"
        for audio_path in sorted(path for path in wav_dir.iterdir() if path.is_file() and path.suffix.lower() in AUDIO_EXTENSIONS):
            if audio_path in seen:
                continue
            candidates = sorted(vtt_dir.glob(f"{audio_path.stem}.*.vtt"))
            if not candidates:
                candidates = sorted(vtt_dir.glob(f"{audio_path.stem}.vtt"))
            if not candidates:
                continue
            transcript_path = candidates[0]
            alignment = tuple(_parse_vtt_segments(transcript_path))
            reference = " ".join(segment["text"] for segment in alignment).strip()
            if not reference:
                continue
            seen.add(audio_path)
            records.append(
                Record(
                    corpus="mtedx",
                    split=split,
                    source_id=audio_path.stem,
                    audio_path=audio_path.resolve(),
                    reference=reference,
                    reference_source=f"OpenSLR-100:{_safe_relative(transcript_path, root)}",
                    reference_status="manual-subtitles-unreviewed",
                    source_audio_sha256=sha256_file(audio_path),
                    alignment=alignment,
                )
            )
    return records


def _xml_local_name(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def _float_attribute(element: ElementTree.Element, *names: str) -> float | None:
    for name in names:
        value = element.attrib.get(name)
        if value is None:
            continue
        try:
            return float(value)
        except ValueError:
            continue
    return None


def _ami_word_text(element: ElementTree.Element) -> str:
    text = " ".join(part.strip() for part in element.itertext() if part.strip()).strip()
    if text:
        return text
    for key in ("text", "word", "orth", "c"):
        value = element.attrib.get(key, "").strip()
        if value:
            return value
    return ""


def _read_ami_words(path: Path) -> list[dict[str, Any]]:
    try:
        root = ElementTree.parse(path).getroot()
    except ElementTree.ParseError as error:
        raise CorpusError(f"invalid AMI NXT XML in {path}: {error}") from error
    words: list[dict[str, Any]] = []
    for element in root.iter():
        if _xml_local_name(element.tag) not in {"w", "word", "nonword", "transformerror"}:
            continue
        start = _float_attribute(element, "starttime", "start", "transcriber_start")
        end = _float_attribute(element, "endtime", "end", "transcriber_end")
        text = _ami_word_text(element)
        if start is None or end is None or end < start or not text:
            continue
        words.append({"start": start, "end": end, "text": text})
    return words


def _ami_meeting_id(path: Path) -> str:
    return path.name.split(".", 1)[0]


def _ami_audio_for(root: Path, meeting: str) -> Path | None:
    candidates = sorted(
        path
        for path in root.rglob("*")
        if path.is_file()
        and path.suffix.lower() in AUDIO_EXTENSIONS
        and path.name.split(".", 1)[0] == meeting
    )
    if not candidates:
        return None
    return next((path for path in candidates if ".Mix-Headset." in path.name), candidates[0])


def _records_from_ami(root: Path) -> list[Record]:
    """Read AMI's official NXT ``words/*.words.xml`` and ``segments/*.segments.xml``."""
    words_root_candidates = [path for path in root.rglob("words") if path.is_dir()]
    words_root = next((path for path in words_root_candidates if any(path.glob("*.words.xml"))), None)
    if words_root is None:
        return []
    words_files = sorted(words_root.glob("*.words.xml"))
    meetings = sorted({_ami_meeting_id(path) for path in words_files})
    records: list[Record] = []
    segments_roots = [path for path in root.rglob("segments") if path.is_dir()]
    for meeting in meetings:
        audio_path = _ami_audio_for(root, meeting)
        if audio_path is None:
            continue
        speaker_words: dict[str, list[dict[str, Any]]] = {}
        meeting_word_files = [path for path in words_files if _ami_meeting_id(path) == meeting]
        for words_path in meeting_word_files:
            parts = words_path.name.split(".")
            speaker = parts[1] if len(parts) > 1 else "?"
            speaker_words[speaker] = _read_ami_words(words_path)

        aligned: list[dict[str, Any]] = []
        segment_files = [
            path
            for segments_root in segments_roots
            for path in segments_root.glob(f"{meeting}.*.segments.xml")
        ]
        for segment_path in sorted(segment_files):
            parts = segment_path.name.split(".")
            speaker = parts[1] if len(parts) > 1 else "?"
            words = speaker_words.get(speaker, [])
            try:
                root_element = ElementTree.parse(segment_path).getroot()
            except ElementTree.ParseError as error:
                raise CorpusError(f"invalid AMI NXT XML in {segment_path}: {error}") from error
            for segment in root_element.iter():
                if _xml_local_name(segment.tag) != "segment":
                    continue
                start = _float_attribute(segment, "transcriber_start", "starttime", "start")
                end = _float_attribute(segment, "transcriber_end", "endtime", "end")
                if start is None or end is None or end <= start:
                    continue
                text = " ".join(
                    word["text"] for word in words if word["start"] >= start - 0.05 and word["end"] <= end + 0.05
                ).strip()
                if text:
                    aligned.append({"start": round(start, 3), "end": round(end, 3), "speaker": speaker, "text": text})

        if not aligned:
            # Some local AMI distributions omit segments.xml.  Keep a timed
            # transcript by grouping each speaker's words at pauses.
            for speaker, words in speaker_words.items():
                current: list[dict[str, Any]] = []
                for word in words:
                    if current and word["start"] - current[-1]["end"] > 1.5:
                        aligned.append(
                            {
                                "start": round(current[0]["start"], 3),
                                "end": round(current[-1]["end"], 3),
                                "speaker": speaker,
                                "text": " ".join(item["text"] for item in current),
                            }
                        )
                        current = []
                    current.append(word)
                if current:
                    aligned.append(
                        {
                            "start": round(current[0]["start"], 3),
                            "end": round(current[-1]["end"], 3),
                            "speaker": speaker,
                            "text": " ".join(item["text"] for item in current),
                        }
                    )
        aligned.sort(key=lambda segment: (segment["start"], segment["speaker"], segment["end"]))
        reference = " ".join(segment["text"] for segment in aligned).strip()
        if not reference:
            continue
        source_path = segment_files[0] if segment_files else meeting_word_files[0]
        records.append(
            Record(
                corpus="ami",
                split="test",
                source_id=meeting,
                audio_path=audio_path.resolve(),
                reference=reference,
                reference_source=f"AMI-NXT:{_safe_relative(source_path, root)}",
                reference_status="manual-meeting-unreviewed",
                source_audio_sha256=sha256_file(audio_path),
                alignment=tuple(aligned),
            )
        )
    return records


def _validate_archive_member(name: str) -> None:
    candidate = Path(name)
    if candidate.is_absolute() or ".." in candidate.parts:
        raise CorpusError(f"archive contains an unsafe path: {name}")


def _needs_source_copy(source: Path) -> bool:
    source = source.expanduser().resolve()
    if source.is_file():
        return True
    if not source.is_dir():
        return False
    return bool(
        list(source.glob("*.zip"))
        and not any(path.is_dir() and path.name == "words" for path in source.rglob("*"))
    )


@contextlib.contextmanager
def materialize_local_source(source: Path) -> Iterator[Path]:
    source = source.expanduser().resolve()
    if not source.exists():
        raise CorpusError(f"local corpus source not found: {source}")
    if source.is_dir():
        # The official AMI download supplies a small annotations ZIP separately
        # from the selected WAV signal. Compose both into one temporary view so
        # ``--ami-source /path/to/ami-source`` works without unpacking in place.
        annotation_archives = sorted(
            path for path in source.iterdir() if path.is_file() and path.suffix.lower() == ".zip"
        )
        if annotation_archives and not any(path.is_dir() and path.name == "words" for path in source.rglob("*")):
            with tempfile.TemporaryDirectory(prefix="asr-spike-local-source-") as temporary:
                target = Path(temporary)
                for archive_path in annotation_archives:
                    with zipfile.ZipFile(archive_path) as archive:
                        for member in archive.infolist():
                            _validate_archive_member(member.filename)
                            mode = (member.external_attr >> 16) & 0o170000
                            if mode == 0o120000:
                                raise CorpusError(f"archive contains an unsupported link: {member.filename}")
                        archive.extractall(target)
                external = target / "external"
                for path in source.rglob("*"):
                    if not path.is_file() or path in annotation_archives:
                        continue
                    link = external / path.relative_to(source)
                    link.parent.mkdir(parents=True, exist_ok=True)
                    os.symlink(path.resolve(), link)
                yield target
            return
        yield source
        return
    if not source.is_file():
        raise CorpusError(f"local corpus source is not a file or directory: {source}")

    suffixes = "".join(source.suffixes).lower()
    if not (suffixes.endswith(".zip") or suffixes.endswith(".tar") or ".tar." in suffixes or suffixes.endswith(".tgz")):
        raise CorpusError(f"unsupported local corpus source format: {source} (use a directory or zip/tar archive)")
    with tempfile.TemporaryDirectory(prefix="asr-spike-source-") as temporary:
        target = Path(temporary)
        if suffixes.endswith(".zip"):
            with zipfile.ZipFile(source) as archive:
                for member in archive.infolist():
                    _validate_archive_member(member.filename)
                    mode = (member.external_attr >> 16) & 0o170000
                    if mode == 0o120000:
                        raise CorpusError(f"archive contains an unsupported link: {member.filename}")
                archive.extractall(target)
        else:
            with tarfile.open(source) as archive:
                for member in archive.getmembers():
                    _validate_archive_member(member.name)
                    if member.issym() or member.islnk():
                        raise CorpusError(f"archive contains an unsupported link: {member.name}")
                archive.extractall(target)
        yield target


def _load_local_records_in_context(root: Path, corpus: str) -> list[Record]:
    if corpus == "mtedx":
        records = _records_from_mtedx(root)
    elif corpus == "ami":
        records = _records_from_ami(root)
    else:
        records = _records_from_local_metadata(root, corpus)
    if not records:
        records = _records_from_local_metadata(root, corpus) if corpus in {"mtedx", "ami"} else []
    if not records:
        records = _records_from_local_sidecars(root, corpus)
    if not records:
        raise CorpusError(
            f"no audio/transcript pairs found in {root}; use JSONL metadata or same-stem .txt/.srt/.vtt sidecars"
        )
    return records


def _copy_records_to_scratch(records: Sequence[Record], scratch: Path, namespace: str) -> list[Record]:
    """Keep files extracted from a temporary archive alive through scoring."""
    stable: list[Record] = []
    target_root = scratch / namespace
    target_root.mkdir(parents=True, exist_ok=True)
    for index, record in enumerate(records):
        suffix = record.audio_path.suffix.lower() or ".audio"
        target = target_root / f"{index:06d}{suffix}"
        shutil.copyfile(record.audio_path, target)
        stable.append(replace(record, audio_path=target, source_audio_sha256=sha256_file(target)))
    return stable


def _array_to_wav(array: Any, sample_rate: Any, destination: Path) -> None:
    try:
        rate = int(sample_rate)
    except (TypeError, ValueError) as error:
        raise CorpusError("FLEURS audio has no valid sampling rate") from error
    if rate <= 0:
        raise CorpusError("FLEURS audio has an invalid sampling rate")
    values = array.tolist() if hasattr(array, "tolist") else array
    if not isinstance(values, Sequence) or isinstance(values, (str, bytes)):
        raise CorpusError("FLEURS audio array is not one-dimensional")
    if values and isinstance(values[0], Sequence) and not isinstance(values[0], (str, bytes)):
        raise CorpusError("FLEURS audio array unexpectedly has multiple channels")
    pcm: list[int] = []
    for value in values:
        try:
            number = float(value)
        except (TypeError, ValueError) as error:
            raise CorpusError("FLEURS audio array contains a non-numeric sample") from error
        if -1.0 <= number <= 1.0:
            number *= 32767.0
        pcm.append(max(-32768, min(32767, int(round(number)))))
    destination.parent.mkdir(parents=True, exist_ok=True)
    with wave.open(str(destination), "wb") as handle:
        handle.setnchannels(1)
        handle.setsampwidth(2)
        handle.setframerate(rate)
        handle.writeframes(struct.pack(f"<{len(pcm)}h", *pcm))


def _fleurs_record(row: Mapping[str, Any], index: int, split: str, scratch: Path) -> Record:
    audio = row.get("audio")
    audio_path: Path | None = None
    if isinstance(audio, Mapping):
        value = audio.get("path")
        if isinstance(value, (str, os.PathLike)) and Path(value).is_file():
            audio_path = Path(value).expanduser().resolve()
        elif isinstance(audio.get("bytes"), (bytes, bytearray)):
            audio_path = scratch / f"fleurs-{split}-{index:06d}.audio"
            audio_path.parent.mkdir(parents=True, exist_ok=True)
            audio_path.write_bytes(bytes(audio["bytes"]))
        elif audio.get("array") is not None:
            audio_path = scratch / f"fleurs-{split}-{index:06d}.wav"
            _array_to_wav(audio["array"], audio.get("sampling_rate"), audio_path)
    if audio_path is None:
        value = row.get("path")
        if isinstance(value, (str, os.PathLike)) and Path(value).is_file():
            audio_path = Path(value).expanduser().resolve()
    if audio_path is None or not audio_path.is_file():
        raise CorpusError(f"FLEURS row {index} has no readable audio path or decoded audio array")
    reference = _reference_from_mapping(row)
    if not reference:
        raise CorpusError(f"FLEURS row {index} has no transcription")
    source_id = (_nonempty_text(row.get("id") or row.get("path")) or "row") + f"#{index:06d}"
    reference_source = "google/fleurs:raw_transcription" if _nonempty_text(row.get("raw_transcription")) else "google/fleurs:transcription"
    return Record(
        corpus="fleurs",
        split=split,
        source_id=source_id,
        audio_path=audio_path,
        reference=reference,
        reference_source=reference_source,
        reference_status="gold",
        source_audio_sha256=sha256_file(audio_path),
    )


def _fleurs_source_id(row: Mapping[str, Any], index: int) -> str:
    return (_nonempty_text(row.get("id") or row.get("path")) or "row") + f"#{index:06d}"


def _fleurs_dataset(config: str, source_split: str) -> Any:
    try:
        from datasets import Audio, load_dataset  # type: ignore
    except ImportError as error:
        raise CorpusError(
            "FLEURS download requires the 'datasets' package; install it or pass --fleurs-source with local metadata"
        ) from error
    try:
        dataset = load_dataset("google/fleurs", config, split=source_split, streaming=True)
    except TypeError:
        # Older datasets releases may not accept streaming for this builder.
        dataset = load_dataset("google/fleurs", config, split=source_split)
    try:
        dataset = dataset.cast_column("audio", Audio(decode=False))
    except (AttributeError, TypeError, ValueError):
        # A local test double or an older datasets release may not expose
        # Audio(decode=False).  Selected rows are still handled correctly.
        pass
    return dataset


def load_fleurs_huggingface(
    config: str,
    scratch: Path,
    dev_count: int | None = None,
    test_count: int | None = None,
    seed: int = 0,
) -> list[Record]:
    records: list[Record] = []
    try:
        requested = {"dev": dev_count, "test": test_count}
        for source_split, split in (("validation", "dev"), ("test", "test")):
            dataset = _fleurs_dataset(config, source_split)
            candidates: list[tuple[str, int]] = []
            for index, row in enumerate(dataset):
                if not isinstance(row, Mapping):
                    raise CorpusError(f"FLEURS {source_split} row {index} is not an object")
                if not _reference_from_mapping(row):
                    raise CorpusError(f"FLEURS {source_split} row {index} has no transcription")
                candidates.append((_fleurs_source_id(row, index), index))
            count = requested[split]
            if count is None:
                wanted_indices = {index for _, index in candidates}
            else:
                if len(candidates) < count:
                    raise CorpusError(f"FLEURS has {len(candidates)} {split} records, but {count} are required")
                ranked = sorted(
                    candidates,
                    key=lambda item: hashlib.sha256(
                        f"{seed}\0fleurs\0{split}\0{item[0]}".encode()
                    ).hexdigest(),
                )
                wanted_indices = {index for _, index in ranked[:count]}
            if not wanted_indices:
                continue
            # Re-open the streaming split so only selected rows decode audio.
            selected_dataset = _fleurs_dataset(config, source_split)
            for index, row in enumerate(selected_dataset):
                if index in wanted_indices:
                    records.append(_fleurs_record(row, index, split, scratch))
    except Exception as error:  # datasets exposes several backend-specific error types
        if isinstance(error, CorpusError):
            raise
        raise CorpusError(f"unable to load public FLEURS {config}: {error}") from error
    return records


def load_fleurs_local(source: Path, scratch: Path) -> list[Record]:
    source = source.expanduser().resolve()
    with materialize_local_source(source) as root:
        metadata_files = _metadata_candidates(root)
        records: list[Record] = []
        for metadata_path in metadata_files:
            try:
                rows = list(_read_metadata_rows(metadata_path))
            except CorpusError:
                continue
            for index, row in enumerate(rows):
                audio_path = _audio_path_from_mapping(row, metadata_path.parent)
                if audio_path is None:
                    continue
                # Local FLEURS fixtures use ordinary file paths; unlike HF rows,
                # no decoding library is needed here.
                if not audio_path.is_file():
                    raise CorpusError(f"metadata points to missing FLEURS audio: {audio_path}")
                row_with_path = dict(row)
                row_with_path["path"] = str(audio_path)
                split = _canonical_split(row.get("split"), _infer_split(metadata_path, root))
                if split not in {"dev", "test"}:
                    continue
                records.append(_fleurs_record(row_with_path, index, split, scratch))
        if not records:
            records = _records_from_local_sidecars(root, "fleurs")
            records = [record for record in records if record.split in {"dev", "test"}]
        if not records:
            raise CorpusError(
                f"no FLEURS records found in {source}; use validation/test JSONL or same-stem transcript sidecars"
            )
        return _copy_records_to_scratch(records, scratch, "fleurs-source") if source.is_file() else records


def _selection_key(record: Record, seed: int) -> str:
    payload = f"{seed}\0{record.corpus}\0{record.split}\0{record.source_id}".encode()
    return hashlib.sha256(payload).hexdigest()


def select_fleurs(records: Sequence[Record], dev_count: int, test_count: int, seed: int) -> list[Record]:
    selected: list[Record] = []
    for split, count in (("dev", dev_count), ("test", test_count)):
        candidates = [record for record in records if record.split == split]
        if len(candidates) < count:
            raise CorpusError(f"FLEURS has {len(candidates)} {split} records, but {count} are required")
        ordered = sorted(candidates, key=lambda record: _selection_key(record, seed))
        selected.extend(ordered[:count])
    return selected


TECHNICAL_HINTS = (
    "algoritmo",
    "analisi",
    "calcolo",
    "cervello",
    "clima",
    "codice",
    "dati",
    "energia",
    "fisica",
    "genetica",
    "informatica",
    "ingegneria",
    "intelligenza artificiale",
    "laboratorio",
    "matematica",
    "medicina",
    "ricerca",
    "scienza",
    "scientifico",
    "tecnologia",
)


def _technical_score(record: Record) -> tuple[int, int, str]:
    text = record.reference.casefold()
    hints = sum(text.count(term) for term in TECHNICAL_HINTS)
    # Longer talks expose more boundary and long-form behavior; use the
    # transcript length as a stable proxy before normalization is performed.
    return (-hints, -len(text), _selection_key(record, 0))


def select_mtedx(records: Sequence[Record], dev_count: int, test_count: int, seed: int) -> list[Record]:
    selected: list[Record] = []
    for split, count in (("dev", dev_count), ("test", test_count)):
        candidates = [record for record in records if record.split == split]
        if len(candidates) < count:
            raise CorpusError(f"mTEDx has {len(candidates)} {split} talks, but {count} are required")
        # Prefer transcripts containing technical vocabulary, then longer talks;
        # the seeded ID order makes ties reproducible.
        ordered = sorted(
            candidates,
            key=lambda record: (_technical_score(record)[:2], _selection_key(record, seed)),
        )
        selected.extend(ordered[:count])
    return selected


def select_ami(records: Sequence[Record], count: int, seed: int) -> list[Record]:
    if len(records) < count:
        raise CorpusError(f"AMI source has {len(records)} meetings, but {count} are required")
    return sorted(records, key=lambda record: _selection_key(record, seed))[:count]


def _slug(value: str) -> str:
    slug = re.sub(r"[^A-Za-z0-9]+", "-", value).strip("-").lower()
    return slug[:72] or "sample"


def sample_id(record: Record) -> str:
    digest = sha256_text(f"{record.corpus}\0{record.split}\0{record.source_id}")[:10]
    return f"{_slug(record.corpus)}-{_slug(record.split)}-{_slug(record.source_id)}-{digest}"


def _wav_is_normalized(path: Path) -> bool:
    try:
        with wave.open(str(path), "rb") as handle:
            return (
                handle.getnchannels() == 1
                and handle.getsampwidth() == 2
                and handle.getframerate() == SAMPLE_RATE
                and handle.getcomptype() == "NONE"
                and handle.getnframes() > 0
            )
    except (OSError, wave.Error):
        return False


def normalize_audio(source: Path, destination: Path) -> float:
    destination.parent.mkdir(parents=True, exist_ok=True)
    if _wav_is_normalized(source):
        shutil.copyfile(source, destination)
    else:
        command = shutil.which("ffmpeg")
        if command is None:
            raise CorpusError(f"ffmpeg is required to normalize {source}")
        result = subprocess.run(
            [
                command,
                "-y",
                "-nostdin",
                "-v",
                "error",
                "-i",
                str(source),
                "-map",
                "0:a:0",
                "-vn",
                "-sn",
                "-dn",
                "-map_metadata",
                "-1",
                "-ar",
                str(SAMPLE_RATE),
                "-ac",
                "1",
                "-c:a",
                "pcm_s16le",
                "-f",
                "wav",
                str(destination),
            ],
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        if result.returncode != 0:
            detail = result.stderr.strip().splitlines()[-1:] or ["unknown ffmpeg error"]
            raise CorpusError(f"ffmpeg failed for {source}: {detail[0]}")
    if not _wav_is_normalized(destination):
        raise CorpusError(f"normalization produced an invalid 16 kHz mono PCM WAV: {destination}")
    with wave.open(str(destination), "rb") as handle:
        return handle.getnframes() / float(handle.getframerate())


def _ffmpeg_path() -> str:
    command = shutil.which("ffmpeg")
    if command is None:
        raise CorpusError("ffmpeg is required for perturbations and long-sequence generation")
    return command


def _render_perturbation(source: Path, destination: Path, variant: str, seed: int = 0) -> float:
    command = [_ffmpeg_path(), "-y", "-nostdin", "-v", "error", "-i", str(source)]
    if variant == "noise":
        command.extend(
            [
                "-filter_complex",
                f"[0:a]aresample=16000[a];anoisesrc=color=white:amplitude=0.02:sample_rate=16000:seed={seed}[noise];[a][noise]amix=inputs=2:duration=first:weights='1 0.12'[mixed]",
                "-map",
                "[mixed]",
            ]
        )
    elif variant == "speed_1p2":
        command.extend(["-af", "atempo=1.2"])
    elif variant == "silence":
        command.extend(["-af", "adelay=500:all=1,apad=pad_dur=0.5"])
    else:
        raise CorpusError(f"unsupported perturbation variant: {variant}")
    if variant == "silence":
        with wave.open(str(source), "rb") as handle:
            duration = handle.getnframes() / float(handle.getframerate())
        command.extend(["-t", f"{duration + 1.0:.3f}"])
    command.extend(
        [
            "-map_metadata",
            "-1",
            "-ar",
            str(SAMPLE_RATE),
            "-ac",
            "1",
            "-c:a",
            "pcm_s16le",
            "-f",
            "wav",
            str(destination),
        ]
    )
    result = subprocess.run(command, check=False, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if result.returncode != 0:
        detail = result.stderr.strip().splitlines()[-1:] or ["unknown ffmpeg error"]
        raise CorpusError(f"ffmpeg failed for {variant} perturbation: {detail[0]}")
    if not _wav_is_normalized(destination):
        raise CorpusError(f"perturbation produced an invalid WAV: {destination}")
    with wave.open(str(destination), "rb") as handle:
        return handle.getnframes() / float(handle.getframerate())


def _render_long_sequence(source_paths: Sequence[Path], destination: Path, duration_seconds: float) -> float:
    if not source_paths:
        raise CorpusError("cannot build a long sequence without source audio")
    list_path = destination.parent / "long-sequence.concat.txt"
    lines: list[str] = []
    elapsed = 0.0
    index = 0
    while elapsed < duration_seconds:
        source = source_paths[index % len(source_paths)]
        escaped_source = str(source).replace("'", "'\\''")
        lines.append(f"file '{escaped_source}'")
        with wave.open(str(source), "rb") as handle:
            elapsed += handle.getnframes() / float(handle.getframerate())
        index += 1
    list_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    result = subprocess.run(
        [
            _ffmpeg_path(),
            "-y",
            "-nostdin",
            "-v",
            "error",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            str(list_path),
            "-t",
            f"{duration_seconds:.3f}",
            "-ar",
            str(SAMPLE_RATE),
            "-ac",
            "1",
            "-c:a",
            "pcm_s16le",
            "-map_metadata",
            "-1",
            "-f",
            "wav",
            str(destination),
        ],
        check=False,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if result.returncode != 0:
        detail = result.stderr.strip().splitlines()[-1:] or ["unknown ffmpeg error"]
        raise CorpusError(f"ffmpeg failed for long sequence: {detail[0]}")
    list_path.unlink(missing_ok=True)
    if not _wav_is_normalized(destination):
        raise CorpusError(f"long sequence produced an invalid WAV: {destination}")
    with wave.open(str(destination), "rb") as handle:
        return handle.getnframes() / float(handle.getframerate())


def _validate_output(output: Path, dry_run: bool) -> None:
    if dry_run:
        return
    if output.exists():
        raise CorpusError(f"output path already exists; choose a new path to avoid overwriting data: {output}")
    try:
        output.parent.mkdir(parents=True, exist_ok=True)
    except OSError as error:
        raise CorpusError(f"cannot create output parent {output.parent}: {error}") from error


def write_manifest(
    output: Path,
    records: Sequence[Record],
    *,
    seed: int,
    dev_count: int,
    test_count: int,
    fleurs_config: str,
    perturb_count: int = 0,
    long_seconds: float = 0.0,
) -> Path:
    _validate_output(output, dry_run=False)
    stage = Path(tempfile.mkdtemp(prefix=f".{output.name}.", dir=str(output.parent)))
    entries: list[dict[str, Any]] = []
    normalized_destinations: list[Path] = []
    references: list[dict[str, Any]] = []
    references_by_id: dict[str, dict[str, Any]] = {}
    seen_ids: set[str] = set()
    try:
        for record in records:
            stable_id = sample_id(record)
            if stable_id in seen_ids:
                raise CorpusError(f"duplicate stable sample id: {stable_id}; source identifiers are not unique")
            seen_ids.add(stable_id)
            destination = stage / "audio" / f"{stable_id}.wav"
            duration = normalize_audio(record.audio_path, destination)
            normalized_destinations.append(destination)
            final_audio = (output / "audio" / f"{stable_id}.wav").resolve()
            entry: dict[str, Any] = {
                "id": stable_id,
                "corpus": record.corpus,
                "split": record.split,
                "language": "it" if record.corpus != "ami" else "en",
                "source_id": record.source_id,
                "audio_path": str(final_audio),
                "audio_sha256": sha256_file(destination),
                "source_audio_sha256": record.source_audio_sha256 or sha256_file(record.audio_path),
                "duration_seconds": round(duration, 3),
            }
            entries.append(entry)
            reference_entry = {
                "id": stable_id,
                "reference": record.reference,
                "reference_sha256": sha256_text(record.reference),
                "reference_source": record.reference_source,
                "reference_status": record.reference_status,
            }
            if record.alignment:
                reference_entry["segments"] = list(record.alignment)
                reference_entry["overlap_count"] = sum(
                    1
                    for left, right in zip(record.alignment, record.alignment[1:])
                    if left.get("speaker")
                    and right.get("speaker")
                    and left["speaker"] != right["speaker"]
                    and right["start"] < left["end"]
                )
            references.append(reference_entry)
            references_by_id[stable_id] = reference_entry
        if perturb_count:
            perturbation_sources = [
                (entry, destination)
                for entry, destination in zip(entries, normalized_destinations)
                if entry["corpus"] == "fleurs" and entry["split"] in {"dev", "test"}
            ][:perturb_count]
            for parent, source in perturbation_sources:
                parent_reference = references_by_id[parent["id"]]
                for variant in ("noise", "speed_1p2", "silence"):
                    variant_id = f"{parent['id']}-perturb-{variant}"
                    destination = stage / "audio" / f"{variant_id}.wav"
                    noise_seed = int(sha256_text(f"{seed}\0{parent['id']}\0noise")[:8], 16)
                    duration = _render_perturbation(source, destination, variant, seed=noise_seed)
                    final_audio = (output / "audio" / f"{variant_id}.wav").resolve()
                    entry: dict[str, Any] = {
                        "id": variant_id,
                        "corpus": parent["corpus"],
                        "split": parent["split"],
                        "language": parent["language"],
                        "source_id": parent["source_id"],
                        "audio_path": str(final_audio),
                        "audio_sha256": sha256_file(destination),
                        "source_audio_sha256": parent["audio_sha256"],
                        "duration_seconds": round(duration, 3),
                        "parent_id": parent["id"],
                        "perturbation": variant,
                    }
                    if variant == "noise":
                        entry["noise_seed"] = noise_seed
                    variant_reference = {
                        "id": variant_id,
                        "reference": parent_reference["reference"],
                        "reference_sha256": parent_reference["reference_sha256"],
                        "reference_source": parent_reference["reference_source"],
                        "reference_status": parent_reference["reference_status"],
                    }
                    if parent_reference.get("segments"):
                        if "overlap_count" in parent_reference:
                            variant_reference["overlap_count"] = parent_reference["overlap_count"]
                        if variant == "speed_1p2":
                            variant_reference["segments"] = [
                                {
                                    **segment,
                                    "start": round(segment["start"] / 1.2, 3),
                                    "end": round(segment["end"] / 1.2, 3),
                                }
                                for segment in parent_reference["segments"]
                            ]
                        elif variant == "silence":
                            variant_reference["segments"] = [
                                {**segment, "start": round(segment["start"] + 0.5, 3), "end": round(segment["end"] + 0.5, 3)}
                                for segment in parent_reference["segments"]
                            ]
                        else:
                            variant_reference["segments"] = parent_reference["segments"]
                    entries.append(entry)
                    references.append(variant_reference)
                    references_by_id[variant_id] = variant_reference

        if long_seconds:
            original_destinations = normalized_destinations
            long_id = f"stability-sequence-{int(round(long_seconds))}s"
            destination = stage / "audio" / f"{long_id}.wav"
            duration = _render_long_sequence(original_destinations, destination, long_seconds)
            final_audio = (output / "audio" / f"{long_id}.wav").resolve()
            entries.append(
                {
                    "id": long_id,
                    "corpus": "stability",
                    "split": "stress",
                    "language": "und",
                    "source_id": "repeated-selected-audio",
                    "audio_path": str(final_audio),
                    "audio_sha256": sha256_file(destination),
                    "source_audio_sha256": sha256_text("|".join(entry["id"] for entry in entries if entry["corpus"] != "stability")),
                    "duration_seconds": round(duration, 3),
                    "stress_duration_seconds": long_seconds,
                }
            )

        manifest = {
            "schema_version": SCHEMA_VERSION,
            "generator": "asr_spike_corpus.py",
            "normalization": {"sample_rate": SAMPLE_RATE, "channels": 1, "sample_format": "s16le"},
            "selection": {
                "seed": seed,
                "fleurs_config": fleurs_config,
                "fleurs_dev_count": dev_count,
                "fleurs_test_count": test_count,
                "fleurs_perturbation_parents": perturb_count,
                "long_sequence_seconds": long_seconds,
                "references_file": "references.json",
            },
            "samples": entries,
        }
        (stage / "manifest.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        references_payload = {
            "schema_version": SCHEMA_VERSION,
            "manifest": "manifest.json",
            "samples": references,
        }
        # This file is the scorer's input; the runner only receives manifest.json.
        (stage / "references.json").write_text(
            json.dumps(references_payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
        )
        os.replace(stage, output)
    except Exception:
        shutil.rmtree(stage, ignore_errors=True)
        raise
    return output / "manifest.json"


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Prepare reproducible public audio/reference inputs for the Gemma and Parakeet Redux spike."
    )
    parser.add_argument("--output", type=Path, required=True, help="new output directory for manifest.json and normalized audio")
    parser.add_argument("--fleurs-source", type=Path, help="local FLEURS directory/archive; otherwise download google/fleurs")
    parser.add_argument("--fleurs-config", default="it_it", help="Hugging Face FLEURS config (default: it_it)")
    parser.add_argument("--mtedx-source", type=Path, action="append", help="explicit local mTEDx directory/archive (repeatable)")
    parser.add_argument("--ami-source", type=Path, action="append", help="explicit local AMI directory/archive (repeatable)")
    parser.add_argument("--mtedx-dev-talks", type=int, default=1, help="mTEDx development talks (default: 1)")
    parser.add_argument("--mtedx-test-talks", type=int, default=4, help="mTEDx test talks (default: 4)")
    parser.add_argument("--ami-meetings", type=int, default=1, help="AMI meetings (default: 1)")
    parser.add_argument("--dev-count", type=int, default=30, help="FLEURS validation/dev samples (default: 30)")
    parser.add_argument("--test-count", type=int, default=200, help="FLEURS test samples (default: 200)")
    parser.add_argument("--seed", type=int, default=0, help="stable selection seed (default: 0)")
    parser.add_argument("--perturb-count", type=int, default=0, help="FLEURS clips receiving noise, 1.2x speed, and silence variants")
    parser.add_argument("--long-seconds", type=float, default=0.0, help="optional repeated stress sequence duration (e.g. 5400 for 90 minutes)")
    parser.add_argument("--dry-run", action="store_true", help="select and validate records without writing audio or manifest")
    return parser


def _positive_count(value: int, name: str) -> None:
    if value < 0:
        raise CorpusError(f"--{name} must be non-negative")


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_parser()
    args = parser.parse_args(argv)
    try:
        _positive_count(args.dev_count, "dev-count")
        _positive_count(args.test_count, "test-count")
        _positive_count(args.mtedx_dev_talks, "mtedx-dev-talks")
        _positive_count(args.mtedx_test_talks, "mtedx-test-talks")
        _positive_count(args.ami_meetings, "ami-meetings")
        _positive_count(args.perturb_count, "perturb-count")
        if args.long_seconds < 0:
            raise CorpusError("--long-seconds must be non-negative")
        output = args.output.expanduser().resolve()
        _validate_output(output, args.dry_run)
        with tempfile.TemporaryDirectory(prefix="asr-spike-scratch-") as scratch_name:
            scratch = Path(scratch_name)
            if args.fleurs_source:
                fleurs_records = load_fleurs_local(args.fleurs_source, scratch)
            else:
                fleurs_records = load_fleurs_huggingface(
                    args.fleurs_config,
                    scratch,
                    dev_count=args.dev_count,
                    test_count=args.test_count,
                    seed=args.seed,
                )
            selected = select_fleurs(fleurs_records, args.dev_count, args.test_count, args.seed)
            mtedx_records: list[Record] = []
            for source_index, source in enumerate(args.mtedx_source or []):
                needs_copy = _needs_source_copy(source)
                with materialize_local_source(source) as root:
                    records = _load_local_records_in_context(root, "mtedx")
                    if needs_copy:
                        # Avoid copying every full-talk FLAC out of a local
                        # OpenSLR archive; only the requested talks survive.
                        records = select_mtedx(records, args.mtedx_dev_talks, args.mtedx_test_talks, args.seed)
                        if not args.dry_run:
                            records = _copy_records_to_scratch(records, scratch, f"mtedx-source-{source_index}")
                    mtedx_records.extend(records)
            selected.extend(select_mtedx(mtedx_records, args.mtedx_dev_talks, args.mtedx_test_talks, args.seed) if mtedx_records else [])
            ami_records: list[Record] = []
            for source_index, source in enumerate(args.ami_source or []):
                needs_copy = _needs_source_copy(source)
                with materialize_local_source(source) as root:
                    records = _load_local_records_in_context(root, "ami")
                    if needs_copy and not args.dry_run:
                        records = _copy_records_to_scratch(records, scratch, f"ami-source-{source_index}")
                    ami_records.extend(records)
            selected.extend(select_ami(ami_records, args.ami_meetings, args.seed) if ami_records else [])

            if args.dry_run:
                summary = {
                    "fleurs": {"dev": args.dev_count, "test": args.test_count},
                    "additional": {"mtedx": sum(r.corpus == "mtedx" for r in selected), "ami": sum(r.corpus == "ami" for r in selected)},
                    "perturbation_parents": min(args.perturb_count, sum(r.corpus == "fleurs" for r in selected)),
                    "long_seconds": args.long_seconds,
                    "sample_ids": [sample_id(record) for record in selected],
                }
                print(json.dumps(summary, ensure_ascii=False, indent=2))
                return 0

            manifest_path = write_manifest(
                output,
                selected,
                seed=args.seed,
                dev_count=args.dev_count,
                test_count=args.test_count,
                fleurs_config=args.fleurs_config,
                perturb_count=args.perturb_count,
                long_seconds=args.long_seconds,
            )
        manifest_count = len(json.loads(manifest_path.read_text(encoding="utf-8"))["samples"])
        print(f"wrote {manifest_count} samples to {manifest_path}")
        return 0
    except CorpusError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    except OSError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
