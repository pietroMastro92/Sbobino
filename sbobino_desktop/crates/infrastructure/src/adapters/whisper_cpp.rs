#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::Deserialize;
use tokio::fs;
use tokio::io::AsyncRead;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use sbobino_application::{ApplicationError, SpeechToTextEngine};
use sbobino_domain::{
    collapse_consecutive_repeated_segments, minimize_transcript_repetitions, LanguageCode,
    TimedSegment, TimedWord, TranscriptionLanguagePolicy, TranscriptionOutput, WhisperOptions,
};

use crate::adapters::transcript_segmentation::normalize_transcript_segments;
use crate::background_process::tokio_background_command;

static OUTPUT_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);
const DELTA_REPLACE_PREFIX: &str = "\u{001F}REPLACE:";
const PROCESS_WAIT_POLL_INTERVAL: Duration = Duration::from_secs(5);
const PROCESS_IDLE_TIMEOUT_MIN: Duration = Duration::from_secs(900);
const PROCESS_IDLE_TIMEOUT_MAX: Duration = Duration::from_secs(3600);
const WHISPER_SAMPLE_RATE: usize = 16_000;
const WHISPER_SILENCE_SPLIT_SECONDS: f32 = 0.5;
const WHISPER_MIN_UTTERANCE_SECONDS: f32 = 1.5;
const WHISPER_MAX_UTTERANCE_SECONDS: f32 = 15.0;
const WHISPER_MARGIN_SECONDS: f32 = 0.2;
const WHISPER_OVERLAP_SECONDS: f32 = 0.4;

fn frame_sample_count() -> usize {
    WHISPER_SAMPLE_RATE / 50
}

#[derive(Debug, Clone)]
pub struct WhisperCppEngine {
    binary_path: String,
    models_dir: String,
}

#[derive(Default)]
struct TranscriptCollector {
    segments: Vec<TimedSegment>,
    preview_lines: Vec<String>,
}

enum ParsedCliEvent {
    Segment {
        segment: TimedSegment,
        preview_text: String,
    },
    ProgressPercent(f32),
}

#[derive(Debug, Deserialize, Default)]
struct WhisperCliJsonOutput {
    #[serde(default)]
    result: Option<WhisperCliJsonResult>,
    #[serde(default)]
    transcription: Vec<WhisperCliJsonSegment>,
}

#[derive(Debug, Deserialize, Default)]
struct WhisperCliJsonResult {
    #[serde(default)]
    language: Option<String>,
    #[serde(default, alias = "language_probability")]
    language_confidence: Option<f32>,
}

#[derive(Debug, Deserialize, Default)]
struct WhisperCliJsonSegment {
    text: String,
    #[serde(default)]
    offsets: Option<WhisperCliJsonOffsets>,
    #[serde(default)]
    tokens: Vec<WhisperCliJsonToken>,
    #[serde(default)]
    speaker: Option<String>,
    #[serde(default, alias = "lang", alias = "language_code")]
    language: Option<String>,
    #[serde(default, alias = "language_probability")]
    language_confidence: Option<f32>,
}

#[derive(Debug, Deserialize, Default)]
struct WhisperCliJsonOffsets {
    #[serde(default)]
    from: Option<i64>,
    #[serde(default)]
    to: Option<i64>,
}

#[derive(Debug, Deserialize, Default)]
struct WhisperCliJsonToken {
    text: String,
    #[serde(default)]
    offsets: Option<WhisperCliJsonOffsets>,
    #[serde(default)]
    p: Option<f32>,
}

#[derive(Debug, Clone)]
struct WhisperAudioChunk {
    path: PathBuf,
    start_seconds: f32,
    end_seconds: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WhisperCliExecutionMode {
    Default,
    CpuFallback,
}

#[derive(Debug)]
struct WhisperCliAttemptError {
    message: String,
    stderr_output: String,
    status: Option<ExitStatus>,
}

impl WhisperCppEngine {
    pub fn new(binary_path: String, models_dir: String) -> Self {
        Self {
            binary_path,
            models_dir,
        }
    }

    fn normalize_detected_language(value: &str) -> Option<String> {
        LanguageCode::try_from_code(value)
            .ok()
            .filter(|code| !code.is_auto() && code.as_code() != "und")
            .map(|code| code.as_code().to_string())
    }

    fn wav_samples(input_wav: &Path) -> Result<Vec<i16>, ApplicationError> {
        let mut reader = hound::WavReader::open(input_wav).map_err(|error| {
            ApplicationError::SpeechToText(format!(
                "failed to read 16 kHz WAV for adaptive Whisper chunking: {error}"
            ))
        })?;
        let spec = reader.spec();
        if spec.channels != 1 || spec.sample_rate as usize != WHISPER_SAMPLE_RATE {
            return Err(ApplicationError::SpeechToText(format!(
                "adaptive Whisper chunking expects mono {} Hz WAV (got {} channels at {} Hz)",
                WHISPER_SAMPLE_RATE, spec.channels, spec.sample_rate
            )));
        }

        match spec.sample_format {
            hound::SampleFormat::Int => reader
                .samples::<i16>()
                .map(|sample| sample.map_err(|error| error.to_string()))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| {
                    ApplicationError::SpeechToText(format!(
                        "failed to decode WAV samples for adaptive Whisper chunking: {error}"
                    ))
                }),
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .map(|sample| {
                    sample
                        .map(|value| (value.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                        .map_err(|error| error.to_string())
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| {
                    ApplicationError::SpeechToText(format!(
                        "failed to decode float WAV samples for adaptive Whisper chunking: {error}"
                    ))
                }),
        }
    }

    fn sample_rms(samples: &[i16]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        let sum = samples
            .iter()
            .map(|sample| {
                let value = *sample as f32 / i16::MAX as f32;
                value * value
            })
            .sum::<f32>();
        (sum / samples.len() as f32).sqrt()
    }

    fn speech_ranges(samples: &[i16]) -> Vec<(usize, usize)> {
        if samples.is_empty() {
            return Vec::new();
        }
        let frame_len = WHISPER_SAMPLE_RATE / 50; // 20 ms
        let frame_rms = samples
            .chunks(frame_len)
            .map(Self::sample_rms)
            .collect::<Vec<_>>();
        let max_rms = frame_rms.iter().copied().fold(0.0_f32, f32::max);
        let threshold = (max_rms * 0.08).max(0.006);
        let silence_frames = (WHISPER_SILENCE_SPLIT_SECONDS * 50.0) as usize;
        let mut ranges = Vec::<(usize, usize)>::new();
        let mut start_frame = None;
        let mut last_speech_frame = 0usize;
        let mut silent = 0usize;
        for (frame, rms) in frame_rms.iter().copied().enumerate() {
            if rms >= threshold {
                if start_frame.is_none() {
                    start_frame = Some(frame);
                }
                last_speech_frame = frame;
                silent = 0;
            } else if start_frame.is_some() {
                silent += 1;
                if silent >= silence_frames {
                    let start = start_frame.take().unwrap_or(frame) * frame_len;
                    let end = ((last_speech_frame + 1) * frame_len).min(samples.len());
                    if end > start {
                        ranges.push((start, end));
                    }
                    silent = 0;
                }
            }
        }
        if let Some(start_frame) = start_frame {
            let start = start_frame * frame_len;
            if samples.len() > start {
                ranges.push((start, samples.len()));
            }
        }
        ranges
    }

    fn merge_short_speech_ranges(
        mut ranges: Vec<(usize, usize)>,
        sample_count: usize,
    ) -> Vec<(usize, usize)> {
        if ranges.len() < 2 {
            return ranges;
        }
        let minimum = (WHISPER_MIN_UTTERANCE_SECONDS * WHISPER_SAMPLE_RATE as f32) as usize;
        let mut index = 0usize;
        while index < ranges.len() {
            if ranges[index].1.saturating_sub(ranges[index].0) >= minimum {
                index += 1;
                continue;
            }
            if index > 0 {
                let end = ranges[index].1;
                ranges[index - 1].1 = end;
                ranges.remove(index);
                index = index.saturating_sub(1);
            } else if index + 1 < ranges.len() {
                let start = ranges[index].0;
                ranges[index + 1].0 = start;
                ranges.remove(index);
            } else {
                break;
            }
        }
        if ranges.is_empty() {
            vec![(0, sample_count)]
        } else {
            ranges
        }
    }

    fn split_long_range(samples: &[i16], start: usize, end: usize) -> Vec<(usize, usize)> {
        let max_len = (WHISPER_MAX_UTTERANCE_SECONDS * WHISPER_SAMPLE_RATE as f32) as usize;
        let overlap = (WHISPER_OVERLAP_SECONDS * WHISPER_SAMPLE_RATE as f32) as usize;
        if end.saturating_sub(start) <= max_len {
            return vec![(start, end)];
        }
        let mut output = Vec::new();
        let mut cursor = start;
        while end.saturating_sub(cursor) > max_len {
            let target = cursor + max_len;
            let window = WHISPER_SAMPLE_RATE.min(max_len / 4);
            let search_start = target
                .saturating_sub(window)
                .max(cursor + frame_sample_count());
            let search_end = (target + window).min(end.saturating_sub(frame_sample_count()));
            let mut cut = target;
            let mut best_rms = f32::INFINITY;
            let mut candidate = search_start;
            while candidate <= search_end {
                let candidate_end = (candidate + frame_sample_count()).min(end);
                let rms = Self::sample_rms(&samples[candidate..candidate_end]);
                if rms < best_rms {
                    best_rms = rms;
                    cut = candidate;
                }
                candidate += frame_sample_count();
            }
            let chunk_end = cut.min(end);
            if chunk_end <= cursor {
                break;
            }
            output.push((cursor, chunk_end));
            cursor = cut.saturating_sub(overlap);
        }
        if cursor < end {
            output.push((cursor, end));
        }
        output
    }

    fn write_whisper_chunks(
        input_wav: &Path,
    ) -> Result<(tempfile::TempDir, Vec<WhisperAudioChunk>), ApplicationError> {
        let samples = Self::wav_samples(input_wav)?;
        let speech = Self::speech_ranges(&samples);
        let base_ranges = if speech.is_empty() {
            vec![(0, samples.len())]
        } else {
            Self::merge_short_speech_ranges(speech, samples.len())
        };
        let margin = (WHISPER_MARGIN_SECONDS * WHISPER_SAMPLE_RATE as f32) as usize;
        let ranges = base_ranges
            .into_iter()
            .flat_map(|(start, end)| {
                let start = start.saturating_sub(margin);
                let end = (end + margin).min(samples.len());
                Self::split_long_range(&samples, start, end)
            })
            .filter(|(start, end)| end > start)
            .collect::<Vec<_>>();
        let temp_dir = tempfile::tempdir().map_err(|error| {
            ApplicationError::SpeechToText(format!(
                "failed to create Whisper chunk directory: {error}"
            ))
        })?;
        let mut chunks = Vec::with_capacity(ranges.len());
        for (index, (start, end)) in ranges.into_iter().enumerate() {
            let path = temp_dir.path().join(format!("chunk-{index:04}.wav"));
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: WHISPER_SAMPLE_RATE as u32,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = hound::WavWriter::create(&path, spec).map_err(|error| {
                ApplicationError::SpeechToText(format!(
                    "failed to create Whisper chunk {}: {error}",
                    path.display()
                ))
            })?;
            for sample in &samples[start..end] {
                writer.write_sample(*sample).map_err(|error| {
                    ApplicationError::SpeechToText(format!(
                        "failed to write Whisper chunk {}: {error}",
                        path.display()
                    ))
                })?;
            }
            writer.finalize().map_err(|error| {
                ApplicationError::SpeechToText(format!(
                    "failed to finalize Whisper chunk {}: {error}",
                    path.display()
                ))
            })?;
            chunks.push(WhisperAudioChunk {
                path,
                start_seconds: start as f32 / WHISPER_SAMPLE_RATE as f32,
                end_seconds: end as f32 / WHISPER_SAMPLE_RATE as f32,
            });
        }
        Ok((temp_dir, chunks))
    }

    fn model_path(&self, model_filename: &str) -> PathBuf {
        Path::new(&self.models_dir).join(model_filename)
    }

    fn clock_now_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0)
    }

    fn transcription_idle_timeout(total_audio_seconds: Option<f32>) -> Duration {
        let scaled_seconds = total_audio_seconds
            .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
            .map(|seconds| ((seconds as f64 * 0.25).ceil() as u64).saturating_add(300))
            .unwrap_or(PROCESS_IDLE_TIMEOUT_MIN.as_secs());

        let candidate = Duration::from_secs(scaled_seconds);
        candidate.clamp(PROCESS_IDLE_TIMEOUT_MIN, PROCESS_IDLE_TIMEOUT_MAX)
    }

    fn mark_activity(last_activity_at_ms: &AtomicU64) {
        last_activity_at_ms.store(Self::clock_now_millis(), Ordering::Relaxed);
    }

    async fn wait_for_child_with_idle_timeout(
        child: &mut tokio::process::Child,
        total_audio_seconds: Option<f32>,
        last_activity_at_ms: Arc<AtomicU64>,
    ) -> Result<ExitStatus, ApplicationError> {
        let idle_timeout = Self::transcription_idle_timeout(total_audio_seconds);
        let idle_timeout_millis = idle_timeout.as_millis().min(u128::from(u64::MAX)) as u64;
        let mut wait_future = Box::pin(child.wait());

        loop {
            match timeout(PROCESS_WAIT_POLL_INTERVAL, wait_future.as_mut()).await {
                Ok(wait_result) => {
                    return wait_result.map_err(|error| {
                        ApplicationError::SpeechToText(format!(
                            "failed to wait for whisper-cli: {error}"
                        ))
                    });
                }
                Err(_) => {
                    let idle_for_millis = Self::clock_now_millis()
                        .saturating_sub(last_activity_at_ms.load(Ordering::Relaxed));
                    if idle_for_millis < idle_timeout_millis {
                        continue;
                    }

                    drop(wait_future);
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    return Err(ApplicationError::SpeechToText(format!(
                        "whisper-cli stopped producing output for {}s and was terminated",
                        idle_timeout.as_secs()
                    )));
                }
            }
        }
    }

    fn validate_model_exists(&self, model_filename: &str) -> Result<PathBuf, ApplicationError> {
        let model_path = self.model_path(model_filename);
        if model_path.exists() {
            return Ok(model_path);
        }

        let download_url =
            format!("https://huggingface.co/ggerganov/whisper.cpp/resolve/main/{model_filename}");
        Err(ApplicationError::SpeechToText(format!(
            "model file not found at {}. Download it from {}",
            model_path.display(),
            download_url
        )))
    }

    fn parse_timecode_seconds(value: &str) -> Option<f32> {
        let parts: Vec<&str> = value.trim().split(':').collect();
        if parts.len() == 3 {
            let hh = parts[0].parse::<f32>().ok()?;
            let mm = parts[1].parse::<f32>().ok()?;
            let ss = parts[2].replace(',', ".").parse::<f32>().ok()?;
            Some((hh * 3600.0) + (mm * 60.0) + ss)
        } else if parts.len() == 2 {
            let mm = parts[0].parse::<f32>().ok()?;
            let ss = parts[1].replace(',', ".").parse::<f32>().ok()?;
            Some((mm * 60.0) + ss)
        } else {
            None
        }
    }

    fn parse_progress_percent(text: &str) -> Option<f32> {
        let percent_index = text.find('%')?;
        let before_percent = &text[..percent_index];
        let mut candidate: Option<&str> = None;
        for token in before_percent.split(|ch: char| ch.is_whitespace() || ch == '=' || ch == ':') {
            let trimmed = token.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed
                .chars()
                .all(|ch| ch.is_ascii_digit() || ch == '.' || ch == ',')
            {
                candidate = Some(trimmed);
            }
        }

        let value = candidate?.replace(',', ".");
        value
            .parse::<f32>()
            .ok()
            .filter(|parsed| parsed.is_finite())
            .map(|parsed| parsed.clamp(0.0, 100.0))
    }

    fn clean_cli_display_line(raw_line: &str) -> String {
        raw_line
            .replace("\u{001b}[2K", "")
            .replace("[2K]", "")
            .replace("[BLANK_AUDIO]", "")
            .split('\r')
            .next_back()
            .unwrap_or("")
            .trim()
            .to_string()
    }

    fn strip_ansi_escape_codes(value: &str) -> String {
        let mut output = String::with_capacity(value.len());
        let mut chars = value.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '\u{001b}' && chars.peek() == Some(&'[') {
                let _ = chars.next();
                for next in chars.by_ref() {
                    if next == 'm' {
                        break;
                    }
                    if !matches!(next, '0'..='9' | ';') {
                        break;
                    }
                }
                continue;
            }

            output.push(ch);
        }

        output
    }

    fn parse_cli_line(raw_line: &str) -> Option<ParsedCliEvent> {
        let display_line = Self::clean_cli_display_line(raw_line);
        if display_line.is_empty() {
            return None;
        }

        let cleaned = Self::strip_ansi_escape_codes(&display_line)
            .trim()
            .to_string();

        if cleaned.is_empty() {
            return None;
        }

        const NOISE_PREFIXES: [&str; 9] = [
            "init:",
            "main:",
            "whisper_",
            "ggml_",
            "system_info:",
            "output_",
            "sampling_",
            "encode",
            "decode",
        ];

        if NOISE_PREFIXES
            .iter()
            .any(|prefix| cleaned.starts_with(prefix))
        {
            if let Some(progress_percent) = Self::parse_progress_percent(&cleaned) {
                return Some(ParsedCliEvent::ProgressPercent(progress_percent));
            }
            return None;
        }

        if !cleaned.starts_with('[') {
            if let Some(progress_percent) = Self::parse_progress_percent(&cleaned) {
                return Some(ParsedCliEvent::ProgressPercent(progress_percent));
            }
            return None;
        }

        let end_index = cleaned.find(']')?;
        let display_end_index = display_line.find(']')?;
        let bracket_content = cleaned[1..end_index].trim();
        let (start_value, end_value) = bracket_content.split_once("-->")?;
        let start_seconds = Self::parse_timecode_seconds(start_value.trim());
        let end_seconds = Self::parse_timecode_seconds(end_value.trim());

        let without_timestamp = cleaned[end_index + 1..].trim().to_string();

        let normalized = without_timestamp.trim().to_string();
        if normalized.is_empty() {
            return None;
        }

        let preview_text = display_line[display_end_index + 1..].trim().to_string();

        let words = Self::build_word_candidates(&normalized, start_seconds, end_seconds);
        let segment = TimedSegment {
            text: normalized,
            start_seconds,
            end_seconds,
            speaker_id: None,
            speaker_label: None,
            language_code: None,
            language_confidence: None,
            words,
        };

        Some(ParsedCliEvent::Segment {
            segment,
            preview_text,
        })
    }

    fn build_word_candidates(
        text: &str,
        start_seconds: Option<f32>,
        end_seconds: Option<f32>,
    ) -> Vec<TimedWord> {
        let (Some(start), Some(end)) = (
            start_seconds.filter(|value| value.is_finite()),
            end_seconds.filter(|value| value.is_finite()),
        ) else {
            return Vec::new();
        };

        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Vec::new();
        }

        if trimmed.split_whitespace().count() != 1 {
            return Vec::new();
        }

        vec![TimedWord {
            text: trimmed.to_string(),
            start_seconds: Some(start),
            end_seconds: Some(end),
            confidence: None,
        }]
    }

    fn collect_segment(
        collector: &Arc<Mutex<TranscriptCollector>>,
        emit_partial: &Arc<dyn Fn(String) + Send + Sync>,
        segment: TimedSegment,
        preview_text: String,
    ) {
        let mut preview_snapshot: Option<String> = None;
        if let Ok(mut state) = collector.lock() {
            state.segments.push(segment.clone());
            state.preview_lines.push(preview_text.clone());
            preview_snapshot = Some(Self::join_preview_text(&state.preview_lines));
        }

        emit_partial(format!(
            "{DELTA_REPLACE_PREFIX}{}",
            preview_snapshot.unwrap_or(preview_text)
        ));
    }

    fn join_segment_text(segments: &[TimedSegment]) -> String {
        segments
            .iter()
            .map(|segment| segment.text.trim())
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn join_preview_text(lines: &[String]) -> String {
        lines
            .iter()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn offset_segments(segments: &mut [TimedSegment], offset_seconds: f32) {
        if !offset_seconds.is_finite() || offset_seconds.abs() < f32::EPSILON {
            return;
        }
        for segment in segments {
            if let Some(start) = segment.start_seconds.as_mut() {
                *start += offset_seconds;
            }
            if let Some(end) = segment.end_seconds.as_mut() {
                *end += offset_seconds;
            }
            for word in &mut segment.words {
                if let Some(start) = word.start_seconds.as_mut() {
                    *start += offset_seconds;
                }
                if let Some(end) = word.end_seconds.as_mut() {
                    *end += offset_seconds;
                }
            }
        }
    }

    fn deduplicate_overlapping_segments(mut segments: Vec<TimedSegment>) -> Vec<TimedSegment> {
        segments.sort_by(|left, right| {
            left.start_seconds
                .unwrap_or(f32::MAX)
                .partial_cmp(&right.start_seconds.unwrap_or(f32::MAX))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut output = Vec::with_capacity(segments.len());
        for segment in segments {
            let duplicate_index = output.iter().enumerate().rev().take(3).find_map(
                |(index, previous): (usize, &TimedSegment)| {
                    let same_text = previous
                        .text
                        .trim()
                        .eq_ignore_ascii_case(segment.text.trim());
                    let overlap = match (
                        previous.start_seconds,
                        previous.end_seconds,
                        segment.start_seconds,
                        segment.end_seconds,
                    ) {
                        (Some(previous_start), Some(previous_end), Some(start), Some(end)) => {
                            previous_start < end && start < previous_end
                        }
                        _ => false,
                    };
                    let confirmed_language_conflict = matches!(
                        (&previous.language_code, &segment.language_code),
                        (Some(previous), Some(current))
                            if !previous.eq_ignore_ascii_case(current)
                    );
                    (same_text && overlap && !confirmed_language_conflict).then_some(index)
                },
            );
            if let Some(index) = duplicate_index {
                // Keep a model-confirmed label when the overlapping copy was
                // only partially annotated. Never collapse two confirmed,
                // different languages (the language boundary is meaningful).
                if output[index].language_code.is_none() {
                    output[index].language_code = segment.language_code.clone();
                }
                if output[index].language_confidence.is_none() {
                    output[index].language_confidence = segment.language_confidence;
                }
            } else {
                output.push(segment);
            }
        }
        output
    }

    fn milliseconds_to_seconds(value: Option<i64>) -> Option<f32> {
        value.and_then(|milliseconds| {
            if milliseconds < 0 {
                None
            } else {
                Some(milliseconds as f32 / 1000.0)
            }
        })
    }

    fn parse_segments_from_output_json(
        raw_json: &str,
    ) -> Result<Vec<TimedSegment>, ApplicationError> {
        let parsed: WhisperCliJsonOutput = serde_json::from_str(raw_json).map_err(|error| {
            ApplicationError::SpeechToText(format!(
                "failed to parse whisper-cli JSON output: {error}"
            ))
        })?;
        let detected_language = parsed
            .result
            .as_ref()
            .and_then(|result| result.language.as_deref())
            .and_then(Self::normalize_detected_language);
        let result_language_confidence = parsed
            .result
            .as_ref()
            .and_then(|result| result.language_confidence)
            .filter(|value| value.is_finite());

        Ok(parsed
            .transcription
            .into_iter()
            .filter_map(|segment| {
                let text = segment.text.trim().to_string();
                if text.is_empty() {
                    return None;
                }

                let start_seconds = Self::milliseconds_to_seconds(
                    segment.offsets.as_ref().and_then(|offsets| offsets.from),
                );
                let end_seconds = Self::milliseconds_to_seconds(
                    segment.offsets.as_ref().and_then(|offsets| offsets.to),
                );
                let speaker_label = segment
                    .speaker
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string);

                let words = segment
                    .tokens
                    .into_iter()
                    .filter_map(|token| {
                        let text = token.text.trim().to_string();
                        if text.is_empty() {
                            return None;
                        }

                        Some(TimedWord {
                            text,
                            start_seconds: Self::milliseconds_to_seconds(
                                token.offsets.as_ref().and_then(|offsets| offsets.from),
                            ),
                            end_seconds: Self::milliseconds_to_seconds(
                                token.offsets.as_ref().and_then(|offsets| offsets.to),
                            ),
                            confidence: token.p.filter(|value| value.is_finite()),
                        })
                    })
                    .collect::<Vec<_>>();

                let language_confidence = segment
                    .language_confidence
                    .or(result_language_confidence)
                    .filter(|value| value.is_finite());

                let language_code = segment
                    .language
                    .as_deref()
                    .and_then(Self::normalize_detected_language)
                    .or_else(|| detected_language.clone());

                Some(TimedSegment {
                    text,
                    start_seconds,
                    end_seconds,
                    speaker_id: speaker_label.clone(),
                    speaker_label,
                    language_code,
                    language_confidence,
                    words,
                })
            })
            .collect())
    }

    async fn consume_stream<R>(
        reader: R,
        collector: Arc<Mutex<TranscriptCollector>>,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
        _total_audio_seconds: Option<f32>,
        last_activity_at_ms: Arc<AtomicU64>,
    ) -> Result<Vec<String>, ApplicationError>
    where
        R: AsyncRead + Unpin,
    {
        use tokio::io::AsyncBufReadExt;

        let mut lines = tokio::io::BufReader::new(reader).lines();
        let mut raw_lines = Vec::<String>::new();

        while let Ok(Some(raw)) = lines.next_line().await {
            Self::mark_activity(last_activity_at_ms.as_ref());
            raw_lines.push(raw.clone());
            if let Some(parsed_line) = Self::parse_cli_line(&raw) {
                match parsed_line {
                    ParsedCliEvent::Segment {
                        segment,
                        preview_text,
                    } => {
                        if let Some(end_seconds) = segment.end_seconds {
                            emit_progress_seconds(end_seconds);
                        }
                        Self::collect_segment(&collector, &emit_partial, segment, preview_text);
                    }
                    // The CLI prints internal progress updates that can run
                    // well ahead of the finalized segments we have actually
                    // received. Driving the UI from those percentages makes the
                    // progress pill look "ahead" of the live transcript, so
                    // we only advance progress from segment end times.
                    ParsedCliEvent::ProgressPercent(_progress_percent) => {}
                }
            }
        }

        Ok(raw_lines)
    }

    fn normalized_options(options: &WhisperOptions) -> WhisperOptions {
        let mut normalized = options.clone();

        normalized.temperature = normalized.temperature.clamp(0.0, 1.0);
        normalized.temperature_increment_on_fallback =
            normalized.temperature_increment_on_fallback.clamp(0.0, 1.0);
        normalized.entropy_threshold = normalized.entropy_threshold.clamp(0.0, 10.0);
        normalized.logprob_threshold = normalized.logprob_threshold.clamp(-10.0, 0.0);
        normalized.no_speech_threshold = normalized.no_speech_threshold.clamp(0.0, 1.0);
        normalized.word_threshold = normalized.word_threshold.clamp(0.0, 1.0);
        normalized.best_of = normalized.best_of.clamp(1, 20);
        normalized.beam_size = normalized.beam_size.clamp(1, 20);
        normalized.threads = normalized.threads.clamp(1, 32);
        normalized.processors = normalized.processors.clamp(1, 16);

        normalized
    }

    /// Summarize raw stderr for user-facing messages: collapse runs of identical
    /// consecutive lines into a single `<N identical lines: "...">` marker, drop
    /// pure-whitespace lines, and keep only a tail of the result so a verbose
    /// backend log never floods the UI. The full stderr is still used internally
    /// for retry classification — this only shapes what we show the user.
    fn summarize_stderr_for_user(raw_stderr: &str) -> String {
        const MAX_TAIL_LINES: usize = 8;

        let mut collapsed: Vec<String> = Vec::new();
        for line in raw_stderr.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match collapsed.last() {
                Some(last) if last == trimmed => {
                    // consecutive duplicate — skip, counted after the run ends
                }
                _ => collapsed.push(trimmed.to_string()),
            }
        }

        let tail: Vec<&String> = collapsed
            .iter()
            .rev()
            .take(MAX_TAIL_LINES)
            .collect::<Vec<_>>();
        tail.into_iter()
            .rev()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn should_retry_with_cpu_fallback(error: &WhisperCliAttemptError) -> bool {
        let haystack = format!(
            "{}\n{}",
            error.message.to_ascii_lowercase(),
            error.stderr_output.to_ascii_lowercase()
        );

        // Hard GPU failure patterns: these are unambiguous Metal/backend
        // breakdowns, so they always justify a CPU-safe retry regardless of
        // the exit code.
        let gpu_failure = haystack.contains("ggml_metal")
            || haystack.contains("metal buffer")
            || haystack.contains("failed to allocate buffer")
            || haystack.contains("use gpu    = 1");

        let crashed = error
            .status
            .map(|status| !status.success())
            .unwrap_or(false)
            && status_signal_is_crash(error.status);

        // Runtime-noise patterns that whisper.cpp also prints during normal
        // GPU initialization (e.g. `repack tensor`, `flash attention`, backend
        // buffer setup). On their own they are not failures, so they only
        // qualify for a CPU-safe retry when the process exited non-zero or
        // crashed — i.e. the noisy init did not actually succeed.
        let exited_nonzero = error.status.map(|s| !s.success()).unwrap_or(false);
        let runtime_failure = exited_nonzero
            && (haystack.contains("repack tensor")
                || haystack.contains("q8_0_4x4")
                || haystack.contains("repack = 1")
                || haystack.contains("flash attention")
                || haystack.contains("backend init")
                || haystack.contains("backend buffer"));

        gpu_failure || crashed || runtime_failure
    }

    fn configure_command_environment(command: &mut Command, binary_path: &str) {
        // Homebrew-installed whisper-cli links against @rpath/libggml.0.dylib but
        // ships with no embedded rpath. We resolve this by setting DYLD_LIBRARY_PATH
        // to the sibling libexec/lib directory where the dylibs actually live.
        if let Some(binary_dir) = Path::new(binary_path)
            .canonicalize()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        {
            let libexec_lib = binary_dir.join("../libexec/lib");
            let sibling_lib = binary_dir.join("../lib");

            let mut runtime_paths = vec![binary_dir.clone()];
            if libexec_lib.exists() {
                runtime_paths.push(libexec_lib.clone());
            }
            if sibling_lib.exists() {
                runtime_paths.push(sibling_lib.clone());
            }
            if let Some(existing) = std::env::var_os("PATH") {
                runtime_paths.extend(std::env::split_paths(&existing));
            }
            if let Ok(path) = std::env::join_paths(runtime_paths) {
                command.env("PATH", path);
            }

            #[cfg(target_os = "macos")]
            {
                let mut dyld_paths = Vec::new();
                // Always include the binary's own directory first — covers Tauri
                // bundled deployments where dylibs sit right next to whisper-cli.
                dyld_paths.push(binary_dir.to_string_lossy().to_string());
                if libexec_lib.exists() {
                    dyld_paths.push(libexec_lib.to_string_lossy().to_string());
                }
                if sibling_lib.exists() {
                    dyld_paths.push(sibling_lib.to_string_lossy().to_string());
                }
                if let Ok(existing) = std::env::var("DYLD_LIBRARY_PATH") {
                    dyld_paths.push(existing);
                }
                if !dyld_paths.is_empty() {
                    command.env("DYLD_LIBRARY_PATH", dyld_paths.join(":"));
                }
            }
        }
    }

    fn append_cli_flags(
        command: &mut Command,
        input_wav: &Path,
        model_path: &Path,
        language_code: &str,
        options: &WhisperOptions,
        output_base: &Path,
        mode: WhisperCliExecutionMode,
    ) {
        command
            .kill_on_drop(true)
            .arg("-m")
            .arg(model_path)
            .arg("-f")
            .arg(input_wav);

        let options = Self::normalized_options(options);

        command
            .arg("-t")
            .arg(options.threads.to_string())
            .arg("-p")
            .arg(options.processors.to_string())
            .arg("-tp")
            .arg(options.temperature.to_string())
            .arg("-tpi")
            .arg(options.temperature_increment_on_fallback.to_string())
            .arg("-et")
            .arg(options.entropy_threshold.to_string())
            .arg("-lpt")
            .arg(options.logprob_threshold.to_string())
            .arg("-nth")
            .arg(options.no_speech_threshold.to_string())
            .arg("-wt")
            .arg(options.word_threshold.to_string())
            .arg("-sns");

        let _preferred_language = language_code;
        command.arg("-l").arg("auto");

        if options.translate_to_english {
            command.arg("-tr");
        }
        if options.no_context {
            command.arg("-mc").arg("0");
        }
        if options.split_on_word {
            command.arg("-sow");
        }
        if options.tinydiarize {
            command.arg("-tdrz");
        }
        if options.diarize {
            command.arg("-di");
        }
        if let Some(prompt) = options
            .prompt
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            command.arg("--prompt").arg(prompt);
        }
        if options.beam_size > 1 {
            command.arg("-bs").arg(options.beam_size.to_string());
        } else if options.best_of > 1 {
            command.arg("-bo").arg(options.best_of.to_string());
        }

        if mode == WhisperCliExecutionMode::CpuFallback {
            command.arg("-ng").arg("-nfa");
        }

        command
            .arg("-otxt")
            .arg("-ojf")
            .arg("-pc")
            .arg("-pp")
            .arg("-of")
            .arg(output_base)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_whisper_cli_batch_attempt(
        &self,
        chunks: &[WhisperAudioChunk],
        model_path: &Path,
        language_code: &str,
        options: &WhisperOptions,
        total_audio_seconds: Option<f32>,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
        mode: WhisperCliExecutionMode,
    ) -> Result<TranscriptionOutput, WhisperCliAttemptError> {
        if chunks.is_empty() {
            return Err(WhisperCliAttemptError {
                message: "adaptive Whisper chunker produced no input chunks".to_string(),
                stderr_output: String::new(),
                status: None,
            });
        }
        let output_prefix = format!(
            "sbobino-whisper-batch-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or(0),
            OUTPUT_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
        );
        let output_bases = chunks
            .iter()
            .enumerate()
            .map(|(index, _)| std::env::temp_dir().join(format!("{output_prefix}-{index:04}")))
            .collect::<Vec<_>>();

        let mut command = tokio_background_command(&self.binary_path);
        Self::configure_command_environment(&mut command, &self.binary_path);
        Self::append_cli_flags(
            &mut command,
            &chunks[0].path,
            model_path,
            language_code,
            options,
            &output_bases[0],
            mode,
        );
        // whisper-cli v1.8.4 loads the model once, then loops over all
        // positional input files.  A matching -of is accepted for each file,
        // so adaptive chunks keep one model context while retaining offsets.
        for (index, chunk) in chunks.iter().enumerate().skip(1) {
            command
                .arg("-of")
                .arg(&output_bases[index])
                .arg(&chunk.path);
        }

        let mut child = command.spawn().map_err(|error| WhisperCliAttemptError {
            message: format!(
                "whisper-cli failed to start at '{}': {error}. Configure Whisper CLI path in Settings > Local Models.",
                self.binary_path
            ),
            stderr_output: String::new(),
            status: None,
        })?;
        let stdout = child.stdout.take().ok_or_else(|| WhisperCliAttemptError {
            message: "missing whisper-cli stdout pipe".to_string(),
            stderr_output: String::new(),
            status: None,
        })?;
        let stderr = child.stderr.take().ok_or_else(|| WhisperCliAttemptError {
            message: "missing whisper-cli stderr pipe".to_string(),
            stderr_output: String::new(),
            status: None,
        })?;

        let collected = Arc::new(Mutex::new(TranscriptCollector::default()));
        let last_activity_at_ms = Arc::new(AtomicU64::new(Self::clock_now_millis()));
        let suppress_progress: Arc<dyn Fn(f32) + Send + Sync> = Arc::new(|_| {});
        let stdout_task = tokio::spawn(Self::consume_stream(
            stdout,
            collected.clone(),
            emit_partial.clone(),
            suppress_progress.clone(),
            total_audio_seconds,
            last_activity_at_ms.clone(),
        ));
        let stderr_task = tokio::spawn(Self::consume_stream(
            stderr,
            collected.clone(),
            emit_partial.clone(),
            suppress_progress,
            total_audio_seconds,
            last_activity_at_ms.clone(),
        ));
        let status = Self::wait_for_child_with_idle_timeout(
            &mut child,
            total_audio_seconds,
            last_activity_at_ms,
        )
        .await
        .map_err(|error| WhisperCliAttemptError {
            message: error.to_string(),
            stderr_output: String::new(),
            status: None,
        })?;
        let _ = stdout_task.await;
        let stderr_lines = stderr_task
            .await
            .map_err(|error| WhisperCliAttemptError {
                message: format!("stderr reader task failed: {error}"),
                stderr_output: String::new(),
                status: Some(status),
            })?
            .map_err(|error| WhisperCliAttemptError {
                message: error.to_string(),
                stderr_output: String::new(),
                status: Some(status),
            })?;
        let stderr_output = stderr_lines.join("\n");
        if !status.success() {
            return Err(WhisperCliAttemptError {
                message: format!("whisper-cli failed: {}", stderr_output.trim()),
                stderr_output,
                status: Some(status),
            });
        }

        let mut all_segments = Vec::<TimedSegment>::new();
        let mut text_parts = Vec::<String>::new();
        let mut missing = Vec::<usize>::new();
        for (index, (chunk, output_base)) in chunks.iter().zip(output_bases.iter()).enumerate() {
            let json_path = output_base.with_extension("json");
            let txt_path = output_base.with_extension("txt");
            let parsed = match fs::read_to_string(&json_path).await {
                Ok(content) => Self::parse_segments_from_output_json(&content)
                    .ok()
                    .filter(|segments| !segments.is_empty()),
                Err(_) => None,
            };
            let text_content = fs::read_to_string(&txt_path).await.ok();
            let text_content = text_content
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string);
            let mut segments = if let Some(segments) = parsed {
                segments
            } else if let Some(text) = text_content.as_deref() {
                // A compatible CLI may be configured to emit TXT without
                // JSON. The text is still a valid, non-lossy chunk result;
                // retain it and leave language/timestamps undetermined.
                vec![TimedSegment {
                    text: text.to_string(),
                    start_seconds: Some(chunk.start_seconds),
                    end_seconds: Some(chunk.end_seconds),
                    speaker_id: None,
                    speaker_label: None,
                    language_code: None,
                    language_confidence: None,
                    words: Vec::new(),
                }]
            } else {
                missing.push(index);
                Vec::new()
            };
            Self::offset_segments(&mut segments, chunk.start_seconds);
            if let Some(text) = text_content {
                text_parts.push(text);
            }
            all_segments.extend(segments);
            emit_progress_seconds(
                total_audio_seconds
                    .filter(|total| total.is_finite() && *total > 0.0)
                    .map(|total| chunk.end_seconds.min(total))
                    .unwrap_or(chunk.end_seconds),
            );
            let _ = fs::remove_file(json_path).await;
            let _ = fs::remove_file(txt_path).await;
        }
        if !missing.is_empty() {
            // Retry every missing interval once in an isolated invocation.
            // This costs one extra model load only for the failed interval,
            // while guaranteeing that a partial batch cannot silently drop
            // audio.
            for index in missing.iter().copied() {
                let chunk = &chunks[index];
                let retry_start_seconds = chunk.start_seconds;
                let retry_end_seconds = chunk.end_seconds;
                let batch_progress = emit_progress_seconds.clone();
                let retry_progress: Arc<dyn Fn(f32) + Send + Sync> = Arc::new(move |seconds| {
                    batch_progress(
                        (retry_start_seconds + seconds)
                            .min(retry_end_seconds)
                            .max(retry_start_seconds),
                    );
                });
                match self
                    .run_whisper_cli_attempt(
                        &chunk.path,
                        model_path,
                        language_code,
                        options,
                        Some(chunk.end_seconds - chunk.start_seconds),
                        emit_partial.clone(),
                        retry_progress,
                        mode,
                    )
                    .await
                {
                    Ok(mut recovered) => {
                        Self::offset_segments(&mut recovered.segments, chunk.start_seconds);
                        all_segments.extend(recovered.segments);
                        if !recovered.text.trim().is_empty() {
                            text_parts.push(recovered.text);
                        }
                    }
                    Err(retry_error) => {
                        return Err(WhisperCliAttemptError {
                            message: format!(
                                "whisper-cli did not produce output for adaptive chunk {} ({:.2}-{:.2}s) after isolated retry: {}",
                                index + 1,
                                chunk.start_seconds,
                                chunk.end_seconds,
                                retry_error.message
                            ),
                            stderr_output: if retry_error.stderr_output.trim().is_empty() {
                                stderr_output.clone()
                            } else {
                                retry_error.stderr_output
                            },
                            status: retry_error.status.or(Some(status)),
                        });
                    }
                }
            }
        }
        all_segments = Self::deduplicate_overlapping_segments(all_segments);
        let transcript = if !all_segments.is_empty() {
            Self::join_segment_text(&all_segments)
        } else if text_parts.is_empty() {
            String::new()
        } else {
            text_parts.join("\n")
        };
        let transcript = minimize_transcript_repetitions(&transcript);
        if transcript.trim().is_empty() {
            return Err(WhisperCliAttemptError {
                message: "whisper-cli produced empty output for adaptive chunks".to_string(),
                stderr_output,
                status: Some(status),
            });
        }
        emit_progress_seconds(total_audio_seconds.unwrap_or_else(|| {
            chunks
                .last()
                .map(|chunk| chunk.end_seconds)
                .unwrap_or_default()
        }));
        Ok(TranscriptionOutput {
            text: transcript.clone(),
            segments: normalize_transcript_segments(
                &transcript,
                &all_segments,
                total_audio_seconds,
            ),
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_whisper_cli_attempt(
        &self,
        input_wav: &Path,
        model_path: &Path,
        language_code: &str,
        options: &WhisperOptions,
        total_audio_seconds: Option<f32>,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
        mode: WhisperCliExecutionMode,
    ) -> Result<TranscriptionOutput, WhisperCliAttemptError> {
        let output_base = std::env::temp_dir().join(format!(
            "sbobino-whisper-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or(0),
            OUTPUT_FILE_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let output_txt_path = output_base.with_extension("txt");
        let output_json_path = output_base.with_extension("json");

        let mut command = tokio_background_command(&self.binary_path);
        Self::configure_command_environment(&mut command, &self.binary_path);
        Self::append_cli_flags(
            &mut command,
            input_wav,
            model_path,
            language_code,
            options,
            &output_base,
            mode,
        );

        let mut child = command.spawn().map_err(|e| WhisperCliAttemptError {
            message: format!(
                "whisper-cli failed to start at '{}': {e}. Configure Whisper CLI path in Settings > Local Models.",
                self.binary_path
            ),
            stderr_output: String::new(),
            status: None,
        })?;

        let stdout = child.stdout.take().ok_or_else(|| WhisperCliAttemptError {
            message: "missing whisper-cli stdout pipe".to_string(),
            stderr_output: String::new(),
            status: None,
        })?;
        let stderr = child.stderr.take().ok_or_else(|| WhisperCliAttemptError {
            message: "missing whisper-cli stderr pipe".to_string(),
            stderr_output: String::new(),
            status: None,
        })?;

        let collected = Arc::new(Mutex::new(TranscriptCollector::default()));
        let last_activity_at_ms = Arc::new(AtomicU64::new(Self::clock_now_millis()));

        let stdout_emit = emit_partial.clone();
        let stdout_progress = emit_progress_seconds.clone();
        let stdout_collector = collected.clone();
        let stdout_total_seconds = total_audio_seconds;
        let stdout_last_activity = last_activity_at_ms.clone();
        let stdout_task = tokio::spawn(async move {
            Self::consume_stream(
                stdout,
                stdout_collector,
                stdout_emit,
                stdout_progress,
                stdout_total_seconds,
                stdout_last_activity,
            )
            .await
        });

        let stderr_emit = emit_partial.clone();
        let stderr_progress = emit_progress_seconds.clone();
        let stderr_collector = collected.clone();
        let stderr_total_seconds = total_audio_seconds;
        let stderr_last_activity = last_activity_at_ms.clone();
        let stderr_task = tokio::spawn(async move {
            Self::consume_stream(
                stderr,
                stderr_collector,
                stderr_emit,
                stderr_progress,
                stderr_total_seconds,
                stderr_last_activity,
            )
            .await
        });

        let status = Self::wait_for_child_with_idle_timeout(
            &mut child,
            total_audio_seconds,
            last_activity_at_ms,
        )
        .await
        .map_err(|error| WhisperCliAttemptError {
            message: error.to_string(),
            stderr_output: String::new(),
            status: None,
        })?;

        let _stdout_lines = stdout_task
            .await
            .map_err(|e| WhisperCliAttemptError {
                message: format!("stdout reader task failed: {e}"),
                stderr_output: String::new(),
                status: Some(status),
            })?
            .map_err(|error| WhisperCliAttemptError {
                message: error.to_string(),
                stderr_output: String::new(),
                status: Some(status),
            })?;

        let stderr_lines = stderr_task
            .await
            .map_err(|e| WhisperCliAttemptError {
                message: format!("stderr reader task failed: {e}"),
                stderr_output: String::new(),
                status: Some(status),
            })?
            .map_err(|error| WhisperCliAttemptError {
                message: error.to_string(),
                stderr_output: String::new(),
                status: Some(status),
            })?;
        let stderr_output = stderr_lines.join("\n");

        if !status.success() {
            return Err(WhisperCliAttemptError {
                message: format!("whisper-cli failed: {}", stderr_output.trim()),
                stderr_output,
                status: Some(status),
            });
        }

        let streamed_segments = if let Ok(state) = collected.lock() {
            collapse_consecutive_repeated_segments(&state.segments)
        } else {
            Vec::new()
        };

        let json_segments = match fs::read_to_string(&output_json_path).await {
            Ok(content) => match Self::parse_segments_from_output_json(&content) {
                Ok(segments) if !segments.is_empty() => Some(segments),
                Ok(_) => None,
                Err(_) => None,
            },
            Err(_) => None,
        };

        let transcript_from_file = match fs::read_to_string(&output_txt_path).await {
            Ok(content) => {
                let cleaned = content.trim().to_string();
                if cleaned.is_empty() {
                    None
                } else {
                    Some(cleaned)
                }
            }
            Err(_) => None,
        };

        let transcript = transcript_from_file.unwrap_or_else(|| {
            Self::join_segment_text(json_segments.as_deref().unwrap_or(&streamed_segments))
        });
        let transcript = minimize_transcript_repetitions(&transcript);

        let _ = fs::remove_file(&output_txt_path).await;
        let _ = fs::remove_file(&output_json_path).await;

        if transcript.is_empty() {
            return Err(WhisperCliAttemptError {
                message: "whisper-cli produced empty output".to_string(),
                stderr_output,
                status: Some(status),
            });
        }

        let segments = normalize_transcript_segments(
            &transcript,
            json_segments.as_deref().unwrap_or(&streamed_segments),
            total_audio_seconds,
        );

        Ok(TranscriptionOutput {
            text: transcript,
            segments,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn transcribe_with_cli(
        &self,
        input_wav: &Path,
        model_path: &Path,
        language_code: &str,
        options: &WhisperOptions,
        total_audio_seconds: Option<f32>,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        // The converter normally hands us a mono 16 kHz WAV.  If that
        // invariant is unavailable (for example in an older imported job),
        // retain the proven single-file path rather than dropping audio.
        let adaptive_chunks = Self::write_whisper_chunks(input_wav).ok();
        if let Some((chunk_dir, chunks)) = adaptive_chunks {
            if chunks.len() > 1 {
                let _chunk_dir = chunk_dir;
                match self
                    .run_whisper_cli_batch_attempt(
                        &chunks,
                        model_path,
                        language_code,
                        options,
                        total_audio_seconds,
                        emit_partial.clone(),
                        emit_progress_seconds.clone(),
                        WhisperCliExecutionMode::Default,
                    )
                    .await
                {
                    Ok(output) => return Ok(output),
                    Err(error) if Self::should_retry_with_cpu_fallback(&error) => {
                        emit_partial("Whisper fallback CPU-safe mode...".to_string());
                        emit_progress_seconds(0.0);
                        let mut fallback_options = options.clone();
                        fallback_options.processors = 1;
                        return self
                            .run_whisper_cli_batch_attempt(
                                &chunks,
                                model_path,
                                language_code,
                                &fallback_options,
                                total_audio_seconds,
                                emit_partial,
                                emit_progress_seconds,
                                WhisperCliExecutionMode::CpuFallback,
                            )
                            .await
                            .map_err(|retry_error| {
                                let summary =
                                    Self::summarize_stderr_for_user(&retry_error.stderr_output);
                                ApplicationError::SpeechToText(format!(
                                    "Whisper retry in CPU-safe mode failed: {summary}"
                                ))
                            });
                    }
                    Err(error) => {
                        return Err(ApplicationError::SpeechToText(error.message));
                    }
                }
            }
        }
        match self
            .run_whisper_cli_attempt(
                input_wav,
                model_path,
                language_code,
                options,
                total_audio_seconds,
                emit_partial.clone(),
                emit_progress_seconds.clone(),
                WhisperCliExecutionMode::Default,
            )
            .await
        {
            Ok(output) => Ok(output),
            Err(error) if Self::should_retry_with_cpu_fallback(&error) => {
                emit_partial("Whisper fallback CPU-safe mode...".to_string());
                // Reset progress before the retry so the UI does not stay stuck
                // at the first attempt's last value while the CPU-safe run
                // replays the audio from the beginning.
                emit_progress_seconds(0.0);
                // Force a single processor for the CPU-safe retry: multi-processor
                // audio-chunk splitting relies on the GPU path that just failed,
                // and on CPU it can trigger the same runtime failures we are
                // recovering from.
                let mut fallback_options = options.clone();
                fallback_options.processors = 1;
                self.run_whisper_cli_attempt(
                    input_wav,
                    model_path,
                    language_code,
                    &fallback_options,
                    total_audio_seconds,
                    emit_partial,
                    emit_progress_seconds,
                    WhisperCliExecutionMode::CpuFallback,
                )
                .await
                .map_err(|retry_error| {
                    let summary = Self::summarize_stderr_for_user(&retry_error.stderr_output);
                    ApplicationError::SpeechToText(format!(
                        "Whisper retry in CPU-safe mode failed: {summary}"
                    ))
                })
            }
            Err(error) => Err(ApplicationError::SpeechToText(error.message)),
        }
    }
}

#[cfg(unix)]
fn status_signal_is_crash(status: Option<ExitStatus>) -> bool {
    const CRASH_SIGNALS: [i32; 3] = [11, 6, 10]; // SIGSEGV, SIGABRT, SIGBUS
    status
        .and_then(|value| {
            // A process killed by a signal reports it via `.signal()`. But when
            // whisper-cli (or a wrapper script) is itself the child that exits
            // with `128 + signal`, the shell surfaces it as an exit code rather
            // than a signal — so we also recognize the `128 + n` form.
            if let Some(signal) = value.signal() {
                return Some(signal);
            }
            value
                .code()
                .filter(|code| *code >= 128)
                .map(|code| code - 128)
        })
        .is_some_and(|value| CRASH_SIGNALS.contains(&value))
}

#[cfg(not(unix))]
fn status_signal_is_crash(_status: Option<ExitStatus>) -> bool {
    false
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use std::path::Path;
    use tokio::process::Command;

    use super::{WhisperCppEngine, PROCESS_IDLE_TIMEOUT_MAX, PROCESS_IDLE_TIMEOUT_MIN};
    use sbobino_domain::WhisperOptions;

    #[test]
    fn transcription_idle_timeout_defaults_to_minimum_without_duration() {
        assert_eq!(
            WhisperCppEngine::transcription_idle_timeout(None),
            PROCESS_IDLE_TIMEOUT_MIN
        );
    }

    #[test]
    fn transcription_idle_timeout_scales_for_longer_audio() {
        assert_eq!(
            WhisperCppEngine::transcription_idle_timeout(Some(7_200.0)).as_secs(),
            2_100
        );
    }

    #[test]
    fn transcription_idle_timeout_caps_at_maximum() {
        assert_eq!(
            WhisperCppEngine::transcription_idle_timeout(Some(24_000.0)),
            PROCESS_IDLE_TIMEOUT_MAX
        );
    }

    #[test]
    fn append_cli_flags_passes_auto_language_explicitly() {
        let mut command = Command::new("whisper-cli");
        WhisperCppEngine::append_cli_flags(
            &mut command,
            Path::new("input.wav"),
            Path::new("model.bin"),
            "auto",
            &WhisperOptions::default(),
            Path::new("output"),
            super::WhisperCliExecutionMode::Default,
        );

        let args = command
            .as_std_mut()
            .get_args()
            .map(|value| value.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        let language_flag = args
            .windows(2)
            .any(|pair| pair[0] == "-l" && pair[1] == "auto");
        assert!(
            language_flag,
            "expected whisper-cli args to contain -l auto: {args:?}"
        );
    }

    #[test]
    fn append_cli_flags_ignores_preference_and_always_uses_auto() {
        let mut command = Command::new("whisper-cli");
        WhisperCppEngine::append_cli_flags(
            &mut command,
            Path::new("input.wav"),
            Path::new("model.bin"),
            "it",
            &WhisperOptions::default(),
            Path::new("output"),
            super::WhisperCliExecutionMode::Default,
        );

        let args = command
            .as_std_mut()
            .get_args()
            .map(|value| value.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        let language_flag = args
            .windows(2)
            .any(|pair| pair[0] == "-l" && pair[1] == "auto");
        assert!(
            language_flag,
            "expected whisper-cli args to contain -l auto: {args:?}"
        );
    }

    #[test]
    fn adaptive_chunker_splits_long_speech_and_keeps_overlap() {
        let mut samples = vec![0_i16; 16_000 * 2];
        samples.extend(std::iter::repeat_n(12_000_i16, 16_000 * 18));
        let ranges = WhisperCppEngine::split_long_range(&samples, 0, samples.len());
        assert!(ranges.len() >= 2);
        assert!(ranges.windows(2).all(|pair| pair[1].0 < pair[0].1));
        assert!(ranges.iter().all(|(start, end)| end > start));
    }

    #[test]
    fn overlapping_whisper_segments_are_deduplicated() {
        let first = sbobino_domain::TimedSegment {
            text: "same sentence".to_string(),
            start_seconds: Some(0.0),
            end_seconds: Some(2.0),
            ..sbobino_domain::TimedSegment::default()
        };
        let duplicate = sbobino_domain::TimedSegment {
            text: "Same sentence".to_string(),
            start_seconds: Some(1.8),
            end_seconds: Some(3.0),
            ..sbobino_domain::TimedSegment::default()
        };
        assert_eq!(
            WhisperCppEngine::deduplicate_overlapping_segments(vec![first, duplicate]).len(),
            1
        );
    }
}

#[async_trait]
impl SpeechToTextEngine for WhisperCppEngine {
    async fn transcribe(
        &self,
        input_wav: &Path,
        model_filename: &str,
        language_policy: &TranscriptionLanguagePolicy,
        options: &WhisperOptions,
        total_audio_seconds: Option<f32>,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        let model_path = self.validate_model_exists(model_filename)?;
        self.transcribe_with_cli(
            input_wav,
            &model_path,
            language_policy.preferred_language.as_code(),
            options,
            total_audio_seconds,
            emit_partial,
            emit_progress_seconds,
        )
        .await
    }
}
