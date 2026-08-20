#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use lingua::{Language, LanguageDetector, LanguageDetectorBuilder};
use serde::Deserialize;
use tokio::fs;
use tokio::io::AsyncRead;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use sbobino_application::{ApplicationError, SpeechToTextEngine};
use sbobino_domain::{
    collapse_consecutive_repeated_segments, minimize_transcript_repetitions, LanguageCode,
    TimedSegment, TimedWord, TranscriptionComputeDevice, TranscriptionLanguagePolicy,
    TranscriptionOutput, WhisperOptions,
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
// Keep the physical chunk at or below whisper.cpp's roughly thirty-second
// context limit. The aggregate target leaves room for the decode margin while
// retaining enough neighboring speech for multilingual segment boundaries.
const WHISPER_MAX_UTTERANCE_SECONDS: f32 = 30.0;
const WHISPER_AGGREGATE_TARGET_SECONDS: f32 = 28.0;
const WHISPER_MARGIN_SECONDS: f32 = 0.2;
const WHISPER_OVERLAP_SECONDS: f32 = 0.4;
const WHISPER_MAX_INPUTS_PER_PROCESS: usize = 16;
const WHISPER_MIN_CLASSIFIER_ALPHA_TOKENS: usize = 3;
const WHISPER_CLASSIFIER_MINIMUM_RELATIVE_DISTANCE: f64 = 0.25;

fn monotonic_progress_callback(
    callback: Arc<dyn Fn(f32) + Send + Sync>,
) -> Arc<dyn Fn(f32) + Send + Sync> {
    let last = Arc::new(Mutex::new(0.0_f32));
    let last_ref = last.clone();
    Arc::new(move |seconds: f32| {
        let seconds = seconds.max(0.0);
        let Ok(mut previous) = last_ref.lock() else {
            return;
        };
        if seconds <= *previous + 0.05 {
            return;
        }
        *previous = seconds;
        callback(seconds);
    })
}

fn frame_sample_count() -> usize {
    WHISPER_SAMPLE_RATE / 50
}

#[derive(Debug, Clone)]
pub struct WhisperCppEngine {
    binary_path: String,
    models_dir: String,
    compute_device: TranscriptionComputeDevice,
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

#[derive(Debug, Default)]
struct WhisperCliAttemptError {
    message: String,
    stderr_output: String,
    status: Option<ExitStatus>,
    /// Output that was durably confirmed before this attempt failed.  A
    /// multi-input whisper process can write some `-of` artifacts and then
    /// crash (or leave a later artifact missing), so the backend retry must
    /// carry this state forward instead of replaying the whole input group.
    partial_output: Option<TranscriptionOutput>,
    /// The exact chunks whose artifacts are still missing or unconfirmed.
    /// Keeping the original chunk descriptors preserves their absolute
    /// timeline offsets for a CPU continuation.
    missing_chunks: Vec<WhisperAudioChunk>,
}

impl WhisperCppEngine {
    pub fn new(binary_path: String, models_dir: String) -> Self {
        Self {
            binary_path,
            models_dir,
            compute_device: TranscriptionComputeDevice::Auto,
        }
    }

    /// Select the compute policy used by whisper.cpp. Values are normalized to
    /// `auto`, `gpu`, or `cpu`; unknown values intentionally retain the safe
    /// automatic policy for forward compatibility with persisted settings.
    pub fn with_compute_device(mut self, value: TranscriptionComputeDevice) -> Self {
        self.compute_device = value;
        self
    }

    fn initial_execution_mode(&self) -> WhisperCliExecutionMode {
        match self.compute_device {
            TranscriptionComputeDevice::Cpu => WhisperCliExecutionMode::CpuFallback,
            TranscriptionComputeDevice::Auto | TranscriptionComputeDevice::Gpu => {
                WhisperCliExecutionMode::Default
            }
        }
    }

    fn allows_cpu_fallback(&self) -> bool {
        self.compute_device == TranscriptionComputeDevice::Auto
    }

    fn normalize_detected_language(value: &str) -> Option<String> {
        LanguageCode::try_from_code(value)
            .ok()
            .filter(|code| !code.is_auto() && code.as_code() != "und")
            .map(|code| code.as_code().to_string())
    }

    fn whisper_language_detector() -> &'static LanguageDetector {
        static DETECTOR: OnceLock<LanguageDetector> = OnceLock::new();
        DETECTOR.get_or_init(|| {
            let languages = [
                Language::Bulgarian,
                Language::Croatian,
                Language::Czech,
                Language::Danish,
                Language::Dutch,
                Language::English,
                Language::Estonian,
                Language::Finnish,
                Language::French,
                Language::German,
                Language::Greek,
                Language::Hindi,
                Language::Hungarian,
                Language::Italian,
                Language::Latvian,
                Language::Lithuanian,
                Language::Malay,
                Language::Persian,
                Language::Polish,
                Language::Portuguese,
                Language::Romanian,
                Language::Slovak,
                Language::Slovene,
                Language::Spanish,
                Language::Swahili,
                Language::Swedish,
                Language::Russian,
                Language::Ukrainian,
            ];
            LanguageDetectorBuilder::from_languages(&languages)
                .with_minimum_relative_distance(WHISPER_CLASSIFIER_MINIMUM_RELATIVE_DISTANCE)
                .build()
        })
    }

    /// Classify only sufficiently long, unambiguous transcript text. Whisper's
    /// JSON result language describes the whole physical input window, so this
    /// conservative per-segment pass can correct a window-level label at a
    /// language boundary without guessing on short or mixed text.
    fn classify_whisper_language(text: &str) -> Option<(String, f32)> {
        let alpha_tokens = text
            .split_whitespace()
            .filter(|token| token.chars().any(char::is_alphabetic))
            .count();
        if alpha_tokens < WHISPER_MIN_CLASSIFIER_ALPHA_TOKENS {
            return None;
        }

        let detector = Self::whisper_language_detector();
        let language = detector.detect_language_of(text.to_string())?;
        let confidence = detector
            .compute_language_confidence_values(text.to_string())
            .into_iter()
            .find(|(candidate, _)| *candidate == language)
            .map(|(_, value)| value as f32)
            .filter(|value| value.is_finite())?;

        Some((
            language.iso_code_639_1().to_string().to_ascii_lowercase(),
            confidence,
        ))
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

    /// Coalesce adjacent utterance ranges into process-sized windows before
    /// adding the decode margin.  The old planner sent every VAD range as a
    /// separate whisper input (often 4--6 seconds), which multiplied model
    /// scheduling overhead on long recordings.  Keeping the original speech
    /// boundaries inside a window lets whisper emit per-segment language
    /// labels while limiting each input to roughly fourteen seconds.
    fn aggregate_speech_ranges(ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
        if ranges.len() < 2 {
            return ranges;
        }

        let target_len = (WHISPER_AGGREGATE_TARGET_SECONDS * WHISPER_SAMPLE_RATE as f32) as usize;
        let mut aggregated = Vec::with_capacity(ranges.len());
        let mut current = ranges[0];

        for (start, end) in ranges.into_iter().skip(1) {
            if end.saturating_sub(current.0) <= target_len {
                // Include the intervening silence: it is useful context for
                // language/utterance boundaries and is bounded by the target
                // span above.
                current.1 = end;
            } else {
                aggregated.push(current);
                current = (start, end);
            }
        }
        aggregated.push(current);
        aggregated
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
            Self::aggregate_speech_ranges(Self::merge_short_speech_ranges(speech, samples.len()))
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
        Self::strip_ansi_escape_codes(raw_line)
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

    fn input_is_decodable_wav(input_wav: &Path) -> bool {
        hound::WavReader::open(input_wav).is_ok()
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
        let bracket_content = cleaned[1..end_index].trim();
        let (start_value, end_value) = bracket_content.split_once("-->")?;
        let start_seconds = Self::parse_timecode_seconds(start_value.trim());
        let end_seconds = Self::parse_timecode_seconds(end_value.trim());

        let without_timestamp = cleaned[end_index + 1..].trim().to_string();

        let normalized = without_timestamp.trim().to_string();
        if normalized.is_empty() {
            return None;
        }

        // Preview deltas are consumed by the desktop UI and persisted in the
        // transcript event stream. Keep them terminal-independent: ANSI color
        // escapes from whisper.cpp are presentation noise, not transcript
        // content.
        let preview_text = cleaned[end_index + 1..].trim().to_string();

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

    /// Keep overlap text while making the persisted timeline non-overlapping.
    ///
    /// Adjacent adaptive chunks intentionally overlap for decoding context. If
    /// both chunks produce different (non-duplicate) words at the boundary,
    /// deduplication must retain both; shifting the later segment forward by
    /// the overlap is lossless and satisfies the timeline contract.
    fn clamp_segment_timestamps_monotonic(mut segments: Vec<TimedSegment>) -> Vec<TimedSegment> {
        let mut previous_end = 0.0_f32;
        for segment in &mut segments {
            let Some(start) = segment.start_seconds else {
                continue;
            };
            if start < previous_end {
                let shift = previous_end - start;
                segment.start_seconds = Some(previous_end);
                if let Some(end) = segment.end_seconds.as_mut() {
                    *end = (*end + shift).max(previous_end);
                }
                for word in &mut segment.words {
                    if let Some(word_start) = word.start_seconds.as_mut() {
                        *word_start += shift;
                    }
                    if let Some(word_end) = word.end_seconds.as_mut() {
                        *word_end += shift;
                    }
                }
            }
            previous_end = previous_end.max(
                segment
                    .end_seconds
                    .or(segment.start_seconds)
                    .unwrap_or(previous_end),
            );
        }
        segments
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
                let text = Self::strip_ansi_escape_codes(segment.text.trim()).to_string();
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
                        let text = Self::strip_ansi_escape_codes(token.text.trim()).to_string();
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

                let explicit_language = segment
                    .language
                    .as_deref()
                    .and_then(Self::normalize_detected_language);
                let classified_language = explicit_language
                    .is_none()
                    .then(|| Self::classify_whisper_language(&text))
                    .flatten();
                let (language_code, language_confidence) = if let Some(language) = explicit_language
                {
                    // A per-segment whisper-cli label is authoritative, even
                    // when the text classifier would disagree.
                    (
                        Some(language),
                        segment
                            .language_confidence
                            .or(result_language_confidence)
                            .filter(|value| value.is_finite()),
                    )
                } else if let Some((language, confidence)) = classified_language {
                    // The result-level label describes the whole input window;
                    // replace it only when Lingua has enough unambiguous text.
                    (Some(language), Some(confidence))
                } else {
                    (
                        detected_language.clone(),
                        segment
                            .language_confidence
                            .or(result_language_confidence)
                            .filter(|value| value.is_finite()),
                    )
                };

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
        emit_progress_percent: Arc<dyn Fn(f32) + Send + Sync>,
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
                    // Single-input transcription keeps its historical
                    // segment-grounded progress callback. Batch callers map
                    // this per-process percentage to their current physical
                    // time range, keeping long jobs visibly active while the
                    // CLI is still decoding the group.
                    ParsedCliEvent::ProgressPercent(progress_percent) => {
                        emit_progress_percent(progress_percent)
                    }
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
            let cleaned = Self::strip_ansi_escape_codes(line);
            let trimmed = cleaned.trim();
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
        // A completed GPU process can still leave one or more requested
        // artifacts missing.  The adaptive batch already made one GPU recovery
        // pass for those chunks; when they remain unresolved, continue on CPU
        // even if the wrapper exited successfully and emitted no GPU-specific
        // diagnostic.  The error carries only these missing descriptors, so
        // the caller can avoid replaying confirmed chunks.
        if !error.missing_chunks.is_empty() {
            return true;
        }

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

    fn append_cli_common_flags(
        command: &mut Command,
        model_path: &Path,
        language_code: &str,
        options: &WhisperOptions,
        mode: WhisperCliExecutionMode,
    ) {
        command.kill_on_drop(true).arg("-m").arg(model_path);

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
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
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
        Self::append_cli_common_flags(command, model_path, language_code, options, mode);
        command.arg("-f").arg(input_wav).arg("-of").arg(output_base);
    }

    fn append_batch_io_args(
        command: &mut Command,
        chunks: &[WhisperAudioChunk],
        output_bases: &[PathBuf],
    ) {
        debug_assert_eq!(chunks.len(), output_bases.len());
        // whisper.cpp stores `fname_inp` and `fname_out` independently while
        // parsing argv. Keep all repeated `-f` entries first, followed by all
        // repeated `-of` entries, so every input/output index is paired by the
        // CLI regardless of option ordering.
        for chunk in chunks {
            command.arg("-f").arg(&chunk.path);
        }
        for output_base in output_bases {
            command.arg("-of").arg(output_base);
        }
    }

    /// Build a snapshot from the artifacts that were confirmed before a batch
    /// failed.  This snapshot is attached to the attempt error so a backend
    /// retry can merge it with the missing-chunk output without replaying or
    /// duplicating already persisted text.
    fn compose_batch_output(
        segments: Vec<TimedSegment>,
        text_parts: Vec<String>,
        total_audio_seconds: Option<f32>,
    ) -> Option<TranscriptionOutput> {
        let segments = Self::clamp_segment_timestamps_monotonic(
            Self::deduplicate_overlapping_segments(segments),
        );
        let transcript = if !segments.is_empty() {
            Self::join_segment_text(&segments)
        } else {
            text_parts
                .iter()
                .map(|text| text.trim())
                .filter(|text| !text.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        };
        let transcript = minimize_transcript_repetitions(&transcript);
        if transcript.trim().is_empty() {
            return None;
        }

        Some(TranscriptionOutput {
            text: transcript.clone(),
            segments: normalize_transcript_segments(&transcript, &segments, total_audio_seconds),
        })
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
        self.run_whisper_cli_batch_attempt_with_recovery(
            chunks,
            model_path,
            language_code,
            options,
            total_audio_seconds,
            emit_partial,
            emit_progress_seconds,
            mode,
            true,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_whisper_cli_batch_attempt_with_recovery(
        &self,
        chunks: &[WhisperAudioChunk],
        model_path: &Path,
        language_code: &str,
        options: &WhisperOptions,
        total_audio_seconds: Option<f32>,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
        mode: WhisperCliExecutionMode,
        retry_missing_individually: bool,
    ) -> Result<TranscriptionOutput, WhisperCliAttemptError> {
        if chunks.is_empty() {
            return Err(WhisperCliAttemptError {
                message: "adaptive Whisper chunker produced no input chunks".to_string(),
                stderr_output: String::new(),
                status: None,
                ..Default::default()
            });
        }
        if chunks.len() > WHISPER_MAX_INPUTS_PER_PROCESS {
            return Err(WhisperCliAttemptError {
                message: format!(
                    "adaptive Whisper process group contains {} inputs; maximum is {}",
                    chunks.len(),
                    WHISPER_MAX_INPUTS_PER_PROCESS
                ),
                stderr_output: String::new(),
                status: None,
                ..Default::default()
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
        Self::append_cli_common_flags(&mut command, model_path, language_code, options, mode);
        Self::append_batch_io_args(&mut command, chunks, &output_bases);

        let mut child = command.spawn().map_err(|error| WhisperCliAttemptError {
            message: format!(
                "whisper-cli failed to start at '{}': {error}. Configure Whisper CLI path in Settings > Local Models.",
                self.binary_path
            ),
            stderr_output: String::new(),
            status: None,
            ..Default::default()
        })?;
        let stdout = child.stdout.take().ok_or_else(|| WhisperCliAttemptError {
            message: "missing whisper-cli stdout pipe".to_string(),
            stderr_output: String::new(),
            status: None,
            ..Default::default()
        })?;
        let stderr = child.stderr.take().ok_or_else(|| WhisperCliAttemptError {
            message: "missing whisper-cli stderr pipe".to_string(),
            stderr_output: String::new(),
            status: None,
            ..Default::default()
        })?;

        let collected = Arc::new(Mutex::new(TranscriptCollector::default()));
        let last_activity_at_ms = Arc::new(AtomicU64::new(Self::clock_now_millis()));
        let suppress_progress: Arc<dyn Fn(f32) + Send + Sync> = Arc::new(|_| {});
        let progress_cursor = Arc::new(Mutex::new((0usize, 0.0_f32)));
        let progress_chunks = chunks.to_vec();
        let group_progress_seconds = emit_progress_seconds.clone();
        // whisper.cpp reports progress from zero to one hundred for each
        // positional input, not once for the whole process. Keep a cursor per
        // process and advance it only when a reset is observed; otherwise the
        // first input reaching 100% would falsely claim the whole 16-input
        // group was complete.
        let emit_progress_percent: Arc<dyn Fn(f32) + Send + Sync> = Arc::new(move |percent| {
            let percent = percent.clamp(0.0, 100.0);
            let Ok(mut cursor) = progress_cursor.lock() else {
                return;
            };
            // A true per-input reset follows a nearly completed input. This
            // high-water guard prevents the same progress line duplicated on
            // stdout/stderr from being mistaken for a new chunk.
            if cursor.1 >= 90.0 && percent + 0.5 < cursor.1 && cursor.0 + 1 < progress_chunks.len()
            {
                cursor.0 += 1;
                cursor.1 = 0.0;
            }
            // Ignore a stale line from the other output stream after the
            // cursor has advanced; it is not new processed coverage.
            if percent + 0.5 < cursor.1 {
                return;
            }
            cursor.1 = percent;
            let Some(chunk) = progress_chunks.get(cursor.0) else {
                return;
            };
            let span = (chunk.end_seconds - chunk.start_seconds).max(0.0);
            group_progress_seconds(
                (chunk.start_seconds + span * (percent / 100.0)).min(chunk.end_seconds),
            );
        });
        let stdout_task = tokio::spawn(Self::consume_stream(
            stdout,
            collected.clone(),
            emit_partial.clone(),
            suppress_progress.clone(),
            emit_progress_percent.clone(),
            total_audio_seconds,
            last_activity_at_ms.clone(),
        ));
        let stderr_task = tokio::spawn(Self::consume_stream(
            stderr,
            collected.clone(),
            emit_partial.clone(),
            suppress_progress,
            emit_progress_percent,
            total_audio_seconds,
            last_activity_at_ms.clone(),
        ));
        // A multi-input whisper process can finish each `-of` file well before
        // it exits. Polling for a complete JSON/TXT artifact makes those
        // confirmed outputs visible immediately instead of waiting for all
        // sixteen inputs in the group. This is grounded in durable per-chunk
        // output, not a synthetic elapsed-time heartbeat.
        let monitor_stop = Arc::new(AtomicBool::new(false));
        let monitor_stop_ref = monitor_stop.clone();
        let monitor_chunks = chunks.to_vec();
        let monitor_output_bases = output_bases.clone();
        let monitor_progress = emit_progress_seconds.clone();
        let monitor_task = tokio::spawn(async move {
            let mut completed = vec![false; monitor_chunks.len()];
            let mut interval = tokio::time::interval(Duration::from_millis(250));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                interval.tick().await;
                if monitor_stop_ref.load(Ordering::Acquire) {
                    break;
                }
                for (index, (chunk, output_base)) in monitor_chunks
                    .iter()
                    .zip(monitor_output_bases.iter())
                    .enumerate()
                {
                    if completed[index] {
                        continue;
                    }
                    let json_ready =
                        match fs::read_to_string(output_base.with_extension("json")).await {
                            Ok(content) => Self::parse_segments_from_output_json(&content)
                                // Empty transcription is a valid confirmed
                                // silence row; malformed/missing JSON alone
                                // should remain retryable.
                                .map(|_| true)
                                .unwrap_or(false),
                            Err(_) => false,
                        };
                    let txt_ready = if json_ready {
                        false
                    } else {
                        fs::read_to_string(output_base.with_extension("txt"))
                            .await
                            .map(|content| !content.trim().is_empty())
                            .unwrap_or(false)
                    };
                    if json_ready || txt_ready {
                        completed[index] = true;
                        monitor_progress(chunk.end_seconds);
                    }
                }
            }
        });
        let status_result = Self::wait_for_child_with_idle_timeout(
            &mut child,
            total_audio_seconds,
            last_activity_at_ms,
        )
        .await;
        monitor_stop.store(true, Ordering::Release);
        let _ = monitor_task.await;
        let status = status_result.map_err(|error| WhisperCliAttemptError {
            message: error.to_string(),
            stderr_output: String::new(),
            status: None,
            ..Default::default()
        })?;
        let _ = stdout_task.await;
        let stderr_lines = stderr_task
            .await
            .map_err(|error| WhisperCliAttemptError {
                message: format!("stderr reader task failed: {error}"),
                stderr_output: String::new(),
                status: Some(status),
                ..Default::default()
            })?
            .map_err(|error| WhisperCliAttemptError {
                message: error.to_string(),
                stderr_output: String::new(),
                status: Some(status),
                ..Default::default()
            })?;
        let stderr_output = Self::strip_ansi_escape_codes(&stderr_lines.join("\n"));

        let mut all_segments = Vec::<TimedSegment>::new();
        let mut text_parts = Vec::<String>::new();
        let mut missing = Vec::<usize>::new();
        for (index, (chunk, output_base)) in chunks.iter().zip(output_bases.iter()).enumerate() {
            let json_path = output_base.with_extension("json");
            let txt_path = output_base.with_extension("txt");
            let parsed = match fs::read_to_string(&json_path).await {
                Ok(content) => Self::parse_segments_from_output_json(&content).ok(),
                Err(_) => None,
            };
            let text_content = fs::read_to_string(&txt_path).await.ok();
            let text_content = text_content
                .as_deref()
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string);
            // A syntactically valid JSON result (including an empty
            // transcription for genuine silence) is confirmed coverage. Only
            // a missing/malformed artifact is eligible for isolated retry.
            let chunk_confirmed = parsed.is_some() || text_content.is_some();
            let mut segments = if let Some(segments) = parsed {
                segments
            } else if let Some(text) = text_content.as_deref() {
                // A compatible CLI may be configured to emit TXT without
                // JSON. The text is still a valid, non-lossy chunk result;
                // retain it and leave language/timestamps undetermined.
                vec![TimedSegment {
                    text: text.to_string(),
                    // TXT output has no timestamps; keep its local interval
                    // here and apply the chunk offset exactly once below.
                    start_seconds: Some(0.0),
                    end_seconds: Some(chunk.end_seconds - chunk.start_seconds),
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
            if chunk_confirmed {
                emit_progress_seconds(
                    total_audio_seconds
                        .filter(|total| total.is_finite() && *total > 0.0)
                        .map(|total| chunk.end_seconds.min(total))
                        .unwrap_or(chunk.end_seconds),
                );
            }
            let _ = fs::remove_file(json_path).await;
            let _ = fs::remove_file(txt_path).await;
        }
        if !missing.is_empty() {
            let missing_chunks = missing
                .iter()
                .map(|index| chunks[*index].clone())
                .collect::<Vec<_>>();
            if !retry_missing_individually {
                let intervals = missing
                    .iter()
                    .map(|index| {
                        let chunk = &chunks[*index];
                        format!("{:.3}-{:.3}s", chunk.start_seconds, chunk.end_seconds)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(WhisperCliAttemptError {
                    message: format!(
                        "whisper-cli batch left missing/unconfirmed chunk intervals: {intervals}"
                    ),
                    stderr_output,
                    status: Some(status),
                    partial_output: Self::compose_batch_output(
                        all_segments,
                        text_parts,
                        total_audio_seconds,
                    ),
                    missing_chunks,
                });
            }

            // Retry all unconfirmed intervals in recovery batches. Keeping the
            // original chunk paths preserves their absolute offsets and avoids
            // loading the model once per missing chunk.
            for retry_group in missing_chunks.chunks(WHISPER_MAX_INPUTS_PER_PROCESS) {
                match Box::pin(self.run_whisper_cli_batch_attempt_with_recovery(
                    retry_group,
                    model_path,
                    language_code,
                    options,
                    total_audio_seconds,
                    emit_partial.clone(),
                    emit_progress_seconds.clone(),
                    mode,
                    false,
                ))
                .await
                {
                    Ok(recovered) => {
                        all_segments.extend(recovered.segments);
                        if !recovered.text.trim().is_empty() {
                            text_parts.push(recovered.text);
                        }
                    }
                    Err(mut retry_error) => {
                        // The nested recovery may itself have confirmed some
                        // outputs before giving up.  Keep both generations of
                        // confirmed state and expose only the still-missing
                        // descriptors to the outer GPU->CPU fallback.
                        let mut retained_segments = all_segments;
                        let mut retained_text_parts = text_parts;
                        if let Some(recovered_partial) = retry_error.partial_output.take() {
                            retained_segments.extend(recovered_partial.segments);
                            if !recovered_partial.text.trim().is_empty()
                                && retained_segments.is_empty()
                            {
                                retained_text_parts.push(recovered_partial.text);
                            }
                        }
                        let unresolved_chunks = if retry_error.missing_chunks.is_empty() {
                            retry_group.to_vec()
                        } else {
                            retry_error.missing_chunks.clone()
                        };
                        retry_error.partial_output = Self::compose_batch_output(
                            retained_segments,
                            retained_text_parts,
                            total_audio_seconds,
                        );
                        retry_error.missing_chunks = unresolved_chunks;
                        return Err(retry_error);
                    }
                }
            }
        }
        let Some(output) =
            Self::compose_batch_output(all_segments, text_parts, total_audio_seconds)
        else {
            return Err(WhisperCliAttemptError {
                message: "whisper-cli produced empty output for adaptive chunks".to_string(),
                stderr_output,
                status: Some(status),
                ..Default::default()
            });
        };
        let batch_end_seconds = chunks
            .iter()
            .map(|chunk| chunk.end_seconds)
            .filter(|seconds| seconds.is_finite())
            .max_by(f32::total_cmp)
            .unwrap_or_default();
        emit_progress_seconds(
            total_audio_seconds
                .filter(|total| total.is_finite() && *total > 0.0)
                .map(|total| batch_end_seconds.min(total))
                .unwrap_or(batch_end_seconds),
        );
        Ok(output)
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
            ..Default::default()
        })?;

        let stdout = child.stdout.take().ok_or_else(|| WhisperCliAttemptError {
            message: "missing whisper-cli stdout pipe".to_string(),
            stderr_output: String::new(),
            status: None,
            ..Default::default()
        })?;
        let stderr = child.stderr.take().ok_or_else(|| WhisperCliAttemptError {
            message: "missing whisper-cli stderr pipe".to_string(),
            stderr_output: String::new(),
            status: None,
            ..Default::default()
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
                Arc::new(|_| {}),
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
                Arc::new(|_| {}),
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
            ..Default::default()
        })?;

        let _stdout_lines = stdout_task
            .await
            .map_err(|e| WhisperCliAttemptError {
                message: format!("stdout reader task failed: {e}"),
                stderr_output: String::new(),
                status: Some(status),
                ..Default::default()
            })?
            .map_err(|error| WhisperCliAttemptError {
                message: error.to_string(),
                stderr_output: String::new(),
                status: Some(status),
                ..Default::default()
            })?;

        let stderr_lines = stderr_task
            .await
            .map_err(|e| WhisperCliAttemptError {
                message: format!("stderr reader task failed: {e}"),
                stderr_output: String::new(),
                status: Some(status),
                ..Default::default()
            })?
            .map_err(|error| WhisperCliAttemptError {
                message: error.to_string(),
                stderr_output: String::new(),
                status: Some(status),
                ..Default::default()
            })?;
        let stderr_output = Self::strip_ansi_escape_codes(&stderr_lines.join("\n"));

        if !status.success() {
            return Err(WhisperCliAttemptError {
                message: format!("whisper-cli failed: {}", stderr_output.trim()),
                stderr_output,
                status: Some(status),
                ..Default::default()
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
                ..Default::default()
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

    fn merge_whisper_group_outputs(
        outputs: &[TranscriptionOutput],
        total_audio_seconds: Option<f32>,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        let mut segments = outputs
            .iter()
            .flat_map(|output| output.segments.iter().cloned())
            .collect::<Vec<_>>();
        segments = Self::clamp_segment_timestamps_monotonic(
            Self::deduplicate_overlapping_segments(segments),
        );
        let text = if segments.is_empty() {
            outputs
                .iter()
                .map(|output| output.text.trim())
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            Self::join_segment_text(&segments)
        };
        let text = minimize_transcript_repetitions(&text);
        if text.trim().is_empty() {
            return Err(ApplicationError::SpeechToText(
                "whisper-cli produced empty output for adaptive chunks".to_string(),
            ));
        }
        Ok(TranscriptionOutput {
            text: text.clone(),
            segments: normalize_transcript_segments(&text, &segments, total_audio_seconds),
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_whisper_cli_chunk_groups(
        &self,
        chunks: &[WhisperAudioChunk],
        model_path: &Path,
        language_code: &str,
        options: &WhisperOptions,
        total_audio_seconds: Option<f32>,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        let mut outputs = Vec::<TranscriptionOutput>::new();
        let initial_mode = self.initial_execution_mode();

        for group in chunks.chunks(WHISPER_MAX_INPUTS_PER_PROCESS) {
            let prior_preview = outputs
                .iter()
                .map(|output| output.text.trim())
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
            let group_emit = emit_partial.clone();
            let cumulative_emit: Arc<dyn Fn(String) + Send + Sync> = Arc::new(move |line| {
                if let Some(snapshot) = line.strip_prefix(DELTA_REPLACE_PREFIX) {
                    let snapshot = snapshot.trim();
                    let combined = if prior_preview.trim().is_empty() {
                        snapshot.to_string()
                    } else if snapshot.is_empty() {
                        prior_preview.clone()
                    } else {
                        format!("{}\n{}", prior_preview, snapshot)
                    };
                    group_emit(format!("{DELTA_REPLACE_PREFIX}{combined}"));
                } else {
                    group_emit(line);
                }
            });

            let attempt = self
                .run_whisper_cli_batch_attempt(
                    group,
                    model_path,
                    language_code,
                    options,
                    total_audio_seconds,
                    cumulative_emit.clone(),
                    emit_progress_seconds.clone(),
                    initial_mode,
                )
                .await;
            let output = match attempt {
                Ok(output) => output,
                Err(mut error)
                    if self.allows_cpu_fallback()
                        && Self::should_retry_with_cpu_fallback(&error)
                        && !error.message.contains("isolated adaptive chunk") =>
                {
                    // Only unresolved chunks are retried when the failed batch
                    // retained confirmed artifacts.  Earlier groups and the
                    // confirmed part of this group remain committed and are
                    // never replayed on CPU.
                    let retained_output = error.partial_output.take();
                    let retry_chunks = if error.missing_chunks.is_empty() {
                        group.to_vec()
                    } else {
                        error.missing_chunks.clone()
                    };
                    let mut fallback_options = options.clone();
                    fallback_options.processors = 1;
                    let fallback_output = if retry_chunks.is_empty() {
                        None
                    } else {
                        Some(
                            self.run_whisper_cli_batch_attempt(
                                &retry_chunks,
                                model_path,
                                language_code,
                                &fallback_options,
                                total_audio_seconds,
                                cumulative_emit,
                                emit_progress_seconds.clone(),
                                WhisperCliExecutionMode::CpuFallback,
                            )
                            .await
                            .map_err(|retry_error| {
                                let summary =
                                    Self::summarize_stderr_for_user(&retry_error.stderr_output);
                                ApplicationError::SpeechToText(format!(
                                    "Whisper transcription failed after a backend retry: {summary}"
                                ))
                            })?,
                        )
                    };
                    match (retained_output, fallback_output) {
                        (Some(confirmed), Some(recovered)) => Self::merge_whisper_group_outputs(
                            &[confirmed, recovered],
                            total_audio_seconds,
                        )?,
                        (Some(confirmed), None) => confirmed,
                        (None, Some(recovered)) => recovered,
                        (None, None) => {
                            return Err(ApplicationError::SpeechToText(
                                "whisper-cli produced no confirmed output after backend retry"
                                    .to_string(),
                            ));
                        }
                    }
                }
                Err(error) => return Err(ApplicationError::SpeechToText(error.message)),
            };
            outputs.push(output);
        }

        let output = Self::merge_whisper_group_outputs(&outputs, total_audio_seconds)?;
        emit_partial(format!("{DELTA_REPLACE_PREFIX}{}", output.text));
        emit_progress_seconds(total_audio_seconds.unwrap_or_else(|| {
            chunks
                .last()
                .map(|chunk| chunk.end_seconds)
                .unwrap_or_default()
        }));
        Ok(output)
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
        // The converter normally hands us a mono 16 kHz WAV. Preserve the
        // legacy single-file path only for inputs that are not decodable WAVs
        // at all (older imported jobs may contain placeholders); real WAV
        // chunking errors must reach the caller instead of being swallowed.
        let adaptive_chunks = match Self::write_whisper_chunks(input_wav) {
            Ok(chunks) => Some(chunks),
            Err(_error) if !Self::input_is_decodable_wav(input_wav) => None,
            Err(error) => return Err(error),
        };
        if let Some((chunk_dir, chunks)) = adaptive_chunks {
            if chunks.len() > 1 {
                let _chunk_dir = chunk_dir;
                return self
                    .run_whisper_cli_chunk_groups(
                        &chunks,
                        model_path,
                        language_code,
                        options,
                        total_audio_seconds,
                        emit_partial,
                        emit_progress_seconds,
                    )
                    .await;
            }
        }
        let initial_mode = self.initial_execution_mode();
        match self
            .run_whisper_cli_attempt(
                input_wav,
                model_path,
                language_code,
                options,
                total_audio_seconds,
                emit_partial.clone(),
                emit_progress_seconds.clone(),
                initial_mode,
            )
            .await
        {
            Ok(output) => Ok(output),
            Err(error)
                if self.allows_cpu_fallback() && Self::should_retry_with_cpu_fallback(&error) =>
            {
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
                        "Whisper transcription failed after a backend retry: {summary}"
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
    use std::path::{Path, PathBuf};
    use tokio::process::Command;

    use super::{
        WhisperAudioChunk, WhisperCppEngine, PROCESS_IDLE_TIMEOUT_MAX, PROCESS_IDLE_TIMEOUT_MIN,
        WHISPER_AGGREGATE_TARGET_SECONDS, WHISPER_MAX_UTTERANCE_SECONDS, WHISPER_SAMPLE_RATE,
    };
    use sbobino_domain::{TranscriptionComputeDevice, WhisperOptions};

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
    fn batch_cli_flags_repeat_all_inputs_before_outputs() {
        let chunks = (0..3)
            .map(|index| WhisperAudioChunk {
                path: Path::new(&format!("chunk-{index}.wav")).to_path_buf(),
                start_seconds: index as f32,
                end_seconds: index as f32 + 1.0,
            })
            .collect::<Vec<_>>();
        let outputs = (0..3)
            .map(|index| PathBuf::from(format!("output-{index}")))
            .collect::<Vec<_>>();
        let mut command = Command::new("whisper-cli");
        WhisperCppEngine::append_cli_common_flags(
            &mut command,
            Path::new("model.bin"),
            "auto",
            &WhisperOptions::default(),
            super::WhisperCliExecutionMode::Default,
        );
        WhisperCppEngine::append_batch_io_args(&mut command, &chunks, &outputs);
        let args = command
            .as_std_mut()
            .get_args()
            .map(|value| value.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        let first_output = args
            .iter()
            .position(|value| value == "-of")
            .expect("batch args should include -of");
        assert_eq!(
            args[..first_output]
                .iter()
                .filter(|arg| *arg == "-f")
                .count(),
            3
        );
        assert_eq!(
            args[first_output..]
                .iter()
                .filter(|arg| *arg == "-of")
                .count(),
            3
        );
        assert!(
            args[first_output..].iter().all(|arg| arg != "-f"),
            "all input paths must precede output paths: {args:?}"
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
    fn explicit_cpu_policy_adds_cpu_safe_flags_without_auto_fallback() {
        let engine = WhisperCppEngine::new("whisper-cli".to_string(), ".".to_string())
            .with_compute_device(TranscriptionComputeDevice::Cpu);
        let mut command = Command::new("whisper-cli");
        WhisperCppEngine::append_cli_flags(
            &mut command,
            Path::new("input.wav"),
            Path::new("model.bin"),
            "auto",
            &WhisperOptions::default(),
            Path::new("output"),
            engine.initial_execution_mode(),
        );
        let args = command
            .as_std_mut()
            .get_args()
            .map(|value| value.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        assert!(args
            .windows(2)
            .any(|pair| pair[0] == "-ng" && pair[1] == "-nfa"));
        assert!(!engine.allows_cpu_fallback());
    }

    #[test]
    fn adaptive_chunker_splits_long_speech_and_keeps_overlap_within_physical_limit() {
        let mut samples = vec![0_i16; 16_000 * 2];
        samples.extend(std::iter::repeat_n(12_000_i16, 16_000 * 38));
        let ranges = WhisperCppEngine::split_long_range(&samples, 0, samples.len());
        assert!(ranges.len() >= 2);
        assert!(ranges.windows(2).all(|pair| pair[1].0 < pair[0].1));
        assert!(ranges.iter().all(|(start, end)| end > start));
        assert!(ranges.iter().all(|(start, end)| {
            (end.saturating_sub(*start) as f32 / WHISPER_SAMPLE_RATE as f32)
                <= WHISPER_MAX_UTTERANCE_SECONDS
        }));
    }

    #[test]
    fn adaptive_chunker_aggregates_short_utterances_into_twenty_eight_second_windows() {
        let ranges = vec![
            (0, 7 * WHISPER_SAMPLE_RATE),
            (7 * WHISPER_SAMPLE_RATE, 14 * WHISPER_SAMPLE_RATE),
            (14 * WHISPER_SAMPLE_RATE, 21 * WHISPER_SAMPLE_RATE),
            (21 * WHISPER_SAMPLE_RATE, 28 * WHISPER_SAMPLE_RATE),
            (28 * WHISPER_SAMPLE_RATE, 35 * WHISPER_SAMPLE_RATE),
        ];
        let aggregated = WhisperCppEngine::aggregate_speech_ranges(ranges);
        assert_eq!(aggregated.len(), 2);
        assert_eq!(aggregated[0], (0, 28 * WHISPER_SAMPLE_RATE));
        assert_eq!(
            aggregated[1],
            (28 * WHISPER_SAMPLE_RATE, 35 * WHISPER_SAMPLE_RATE)
        );
        assert!(aggregated.iter().all(|(start, end)| {
            end.saturating_sub(*start)
                <= (WHISPER_AGGREGATE_TARGET_SECONDS * WHISPER_SAMPLE_RATE as f32) as usize
        }));
    }

    #[test]
    fn whisper_classifier_overrides_inherited_window_language() {
        let raw_json = r#"
        {
          "result": {"language": "en", "language_probability": 0.98},
          "transcription": [
            {"text": "Esta es una prueba de idioma español."}
          ]
        }
        "#;

        let segments = WhisperCppEngine::parse_segments_from_output_json(raw_json)
            .expect("classifier fixture should parse");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].language_code.as_deref(), Some("es"));
        assert!(segments[0]
            .language_confidence
            .is_some_and(|value| value.is_finite() && value > 0.0));
    }

    #[test]
    fn whisper_classifier_preserves_explicit_segment_language() {
        let raw_json = r#"
        {
          "result": {"language": "en", "language_probability": 0.98},
          "transcription": [
            {
              "text": "Esta es una prueba de idioma español.",
              "language": "it",
              "language_probability": 0.61
            }
          ]
        }
        "#;

        let segments = WhisperCppEngine::parse_segments_from_output_json(raw_json)
            .expect("explicit-label fixture should parse");
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].language_code.as_deref(), Some("it"));
        assert_eq!(segments[0].language_confidence, Some(0.61));
    }

    #[test]
    fn whisper_classifier_does_not_override_short_or_uncertain_segments() {
        let raw_json = r#"
        {
          "result": {"language": "en", "language_probability": 0.91},
          "transcription": [
            {"text": "Esta es"},
            {"text": "asdf qwer zxcv"}
          ]
        }
        "#;

        let segments = WhisperCppEngine::parse_segments_from_output_json(raw_json)
            .expect("uncertain-label fixture should parse");
        assert_eq!(segments.len(), 2);
        assert!(segments
            .iter()
            .all(|segment| segment.language_code.as_deref() == Some("en")));
        assert!(segments
            .iter()
            .all(|segment| segment.language_confidence == Some(0.91)));
    }

    #[test]
    fn overlapping_distinct_segments_are_shifted_without_losing_text() {
        let first = sbobino_domain::TimedSegment {
            text: "first".to_string(),
            start_seconds: Some(10.0),
            end_seconds: Some(10.8),
            ..sbobino_domain::TimedSegment::default()
        };
        let second = sbobino_domain::TimedSegment {
            text: "second".to_string(),
            start_seconds: Some(10.5),
            end_seconds: Some(11.2),
            ..sbobino_domain::TimedSegment::default()
        };
        let normalized = WhisperCppEngine::clamp_segment_timestamps_monotonic(vec![first, second]);
        assert_eq!(normalized[0].text, "first");
        assert_eq!(normalized[1].text, "second");
        assert_eq!(normalized[1].start_seconds, Some(10.8));
        assert_eq!(normalized[1].end_seconds, Some(11.5));
        assert!(normalized.windows(2).all(|pair| {
            pair[1].start_seconds.unwrap_or_default() + 0.0001
                >= pair[0].end_seconds.unwrap_or_default()
        }));
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
        let emit_progress_seconds = monotonic_progress_callback(emit_progress_seconds);
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
