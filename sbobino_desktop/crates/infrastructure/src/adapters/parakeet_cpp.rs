use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use tempfile::TempDir;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use sbobino_application::{ApplicationError, SpeechToTextEngine};
use sbobino_domain::{TimedSegment, TimedWord, TranscriptionOutput, WhisperOptions};

use crate::adapters::transcript_segmentation::normalize_transcript_segments;

const DELTA_REPLACE_PREFIX: &str = "\u{001F}REPLACE:";
const REALTIME_EOU_F16_MODEL: &str = "realtime_eou_120m-v1-f16.gguf";
const REALTIME_EOU_Q8_MODEL: &str = "realtime_eou_120m-v1-q8_0.gguf";
const NEMOTRON_STREAMING_PREFIX: &str = "nemotron-3.5-asr-streaming-0.6b";
const WORD_SEGMENT_GAP_BREAK_SECONDS: f32 = 1.25;
const WORD_SEGMENT_MAX_CHARS: usize = 140;
const WORD_SEGMENT_MAX_DURATION_SECONDS: f32 = 12.0;
const WORD_SEGMENT_MIN_TERMINAL_WORDS: usize = 3;
const PREVIEW_TIMEOUT: Duration = Duration::from_secs(12);
const PREVIEW_CHUNK_SECONDS: f32 = 8.0;
const PREVIEW_MAX_CHUNKS: usize = 2;
const PREVIEW_CHUNK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct ParakeetCppEngine {
    binary_path: String,
    models_dir: String,
}

#[derive(Debug, Deserialize, Default)]
struct ParakeetJsonOutput {
    #[serde(default)]
    text: String,
    #[serde(default)]
    words: Vec<ParakeetJsonWord>,
    #[serde(default)]
    segments: Vec<ParakeetJsonSegment>,
}

#[derive(Debug, Deserialize, Default)]
struct ParakeetJsonSegment {
    #[serde(default)]
    text: String,
    #[serde(default)]
    start: Option<f32>,
    #[serde(default)]
    end: Option<f32>,
    #[serde(default)]
    words: Vec<ParakeetJsonWord>,
}

#[derive(Debug, Deserialize, Default)]
struct ParakeetJsonWord {
    #[serde(default, alias = "text")]
    w: String,
    #[serde(default)]
    start: Option<f32>,
    #[serde(default)]
    end: Option<f32>,
    #[serde(default, alias = "confidence")]
    conf: Option<f32>,
}

#[derive(Default)]
struct PreviewStreamState {
    preview: String,
    delta_count: usize,
}

struct PreviewChunk {
    path: PathBuf,
    start_seconds: f32,
    end_seconds: f32,
}

impl ParakeetCppEngine {
    pub fn new(binary_path: String, models_dir: String) -> Self {
        Self {
            binary_path,
            models_dir,
        }
    }

    fn model_path(&self, model_filename: &str) -> PathBuf {
        Path::new(&self.models_dir).join(model_filename)
    }

    fn validate_model_exists(&self, model_filename: &str) -> Result<PathBuf, ApplicationError> {
        let model_path = self.model_path(model_filename);
        if model_path.exists() {
            return Ok(model_path);
        }

        let download_url = format!(
            "https://huggingface.co/mudler/parakeet-cpp-gguf/resolve/main/{model_filename}"
        );
        Err(ApplicationError::SpeechToText(format!(
            "Parakeet model file not found at {}. Download it from {}",
            model_path.display(),
            download_url
        )))
    }

    fn is_english_realtime_language(language_code: &str) -> bool {
        language_code.trim().eq_ignore_ascii_case("en")
    }

    fn is_realtime_eou_model(model_filename: &str) -> bool {
        matches!(
            model_filename,
            REALTIME_EOU_F16_MODEL | REALTIME_EOU_Q8_MODEL
        )
    }

    fn is_nemotron_streaming_model(model_filename: &str) -> bool {
        model_filename.starts_with(NEMOTRON_STREAMING_PREFIX)
    }

    fn parakeet_target_lang(language_code: &str) -> &str {
        match language_code.trim() {
            "" => "auto",
            "ja" => "ja-JP",
            value => value,
        }
    }

    fn validate_preview_model_exists(
        &self,
        final_model_filename: &str,
        language_code: &str,
    ) -> Result<PathBuf, ApplicationError> {
        if !Self::is_english_realtime_language(language_code)
            && Self::is_realtime_eou_model(final_model_filename)
        {
            return Err(ApplicationError::SpeechToText(format!(
                "The selected legacy Parakeet live model cannot transcribe language '{language_code}'. Select Fast or Multilingual Live."
            )));
        }

        self.validate_model_exists(final_model_filename)
    }

    fn configure_command_environment(command: &mut Command, binary_path: &str) {
        if let Some(binary_dir) = Path::new(binary_path)
            .canonicalize()
            .ok()
            .and_then(|path| path.parent().map(PathBuf::from))
        {
            let sibling_lib = binary_dir.join("../lib");
            let mut dyld_paths = vec![binary_dir.to_string_lossy().to_string()];
            if sibling_lib.exists() {
                dyld_paths.push(sibling_lib.to_string_lossy().to_string());
            }
            if let Ok(existing) = std::env::var("DYLD_LIBRARY_PATH") {
                dyld_paths.push(existing);
            }
            command.env("DYLD_LIBRARY_PATH", dyld_paths.join(":"));
        }
    }

    fn extract_json_payload(stdout: &str) -> Result<&str, ApplicationError> {
        let start = stdout.find('{').ok_or_else(|| {
            ApplicationError::SpeechToText("parakeet-cli produced no JSON output".to_string())
        })?;
        let end = stdout.rfind('}').ok_or_else(|| {
            ApplicationError::SpeechToText(
                "parakeet-cli produced incomplete JSON output".to_string(),
            )
        })?;
        if end < start {
            return Err(ApplicationError::SpeechToText(
                "parakeet-cli produced malformed JSON output".to_string(),
            ));
        }
        Ok(&stdout[start..=end])
    }

    fn clean_transcript_text(value: &str) -> String {
        value
            .replace("<EOU>", "")
            .replace("[EOU]", "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string()
    }

    fn parse_json_output(
        raw_stdout: &str,
        total_audio_seconds: Option<f32>,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        let payload = Self::extract_json_payload(raw_stdout)?;
        let parsed: ParakeetJsonOutput = serde_json::from_str(payload).map_err(|error| {
            ApplicationError::SpeechToText(format!("failed to parse parakeet-cli JSON: {error}"))
        })?;

        let text = Self::clean_transcript_text(&parsed.text);
        let segment_text = parsed
            .segments
            .iter()
            .map(|segment| Self::clean_transcript_text(&segment.text))
            .filter(|text| !text.is_empty())
            .collect::<Vec<String>>()
            .join(" ");
        let text = if text.is_empty() { segment_text } else { text };
        if text.is_empty() {
            return Err(ApplicationError::SpeechToText(
                "parakeet-cli produced empty output".to_string(),
            ));
        }

        let has_flat_words = !parsed.words.is_empty();
        let segment_words_available = parsed
            .segments
            .iter()
            .any(|segment| !segment.words.is_empty());
        let raw_segments =
            if parsed.segments.is_empty() || (has_flat_words && !segment_words_available) {
                Self::segments_from_words(text.clone(), parsed.words, total_audio_seconds)
            } else {
                parsed
                    .segments
                    .into_iter()
                    .filter_map(Self::segment_from_json)
                    .flat_map(|segment| Self::split_segment_if_needed(segment, total_audio_seconds))
                    .collect::<Vec<_>>()
            };

        let raw_segments = if raw_segments.is_empty() {
            vec![TimedSegment {
                text: text.clone(),
                start_seconds: None,
                end_seconds: total_audio_seconds,
                speaker_id: None,
                speaker_label: None,
                words: Vec::new(),
            }]
        } else {
            raw_segments
        };

        Ok(TranscriptionOutput {
            text: text.clone(),
            segments: normalize_transcript_segments(&text, &raw_segments, total_audio_seconds),
        })
    }

    fn segments_from_words(
        text: String,
        words: Vec<ParakeetJsonWord>,
        total_audio_seconds: Option<f32>,
    ) -> Vec<TimedSegment> {
        let words = words
            .into_iter()
            .filter_map(Self::word_from_json)
            .collect::<Vec<_>>();

        if words.is_empty() {
            return vec![TimedSegment {
                text,
                start_seconds: None,
                end_seconds: total_audio_seconds,
                speaker_id: None,
                speaker_label: None,
                words,
            }];
        }

        Self::segments_from_timed_words(text, words, total_audio_seconds)
    }

    fn split_segment_if_needed(
        segment: TimedSegment,
        total_audio_seconds: Option<f32>,
    ) -> Vec<TimedSegment> {
        if segment.words.is_empty() {
            return vec![segment];
        }

        let chars = segment.text.chars().count();
        let duration = segment
            .start_seconds
            .zip(segment.end_seconds)
            .map(|(start, end)| (end - start).max(0.0))
            .unwrap_or_default();

        if chars <= WORD_SEGMENT_MAX_CHARS && duration <= WORD_SEGMENT_MAX_DURATION_SECONDS {
            return vec![segment];
        }

        Self::segments_from_timed_words(segment.text, segment.words, total_audio_seconds)
    }

    fn segments_from_timed_words(
        text: String,
        words: Vec<TimedWord>,
        total_audio_seconds: Option<f32>,
    ) -> Vec<TimedSegment> {
        let mut segments = Vec::<TimedSegment>::new();
        let mut current_words = Vec::<TimedWord>::new();
        let mut current_text = String::new();

        for word in words {
            let next_text = word.text.trim();
            if next_text.is_empty() {
                continue;
            }

            if !current_words.is_empty()
                && Self::should_break_word_segment(&current_text, &current_words, &word)
            {
                Self::flush_word_segment(&mut segments, &mut current_text, &mut current_words);
            }

            current_text = Self::join_text_parts(&current_text, next_text);
            current_words.push(word);
        }

        Self::flush_word_segment(&mut segments, &mut current_text, &mut current_words);

        if segments.is_empty() {
            return vec![TimedSegment {
                text,
                start_seconds: None,
                end_seconds: total_audio_seconds,
                speaker_id: None,
                speaker_label: None,
                words: Vec::new(),
            }];
        }

        segments
    }

    fn should_break_word_segment(
        current_text: &str,
        current_words: &[TimedWord],
        next_word: &TimedWord,
    ) -> bool {
        let current_text = current_text.trim();
        let next_text = next_word.text.trim();
        let combined_chars = current_text.chars().count() + 1 + next_text.chars().count();
        if combined_chars > WORD_SEGMENT_MAX_CHARS {
            return true;
        }

        let current_start = current_words.iter().find_map(|word| word.start_seconds);
        let current_end = current_words.iter().rev().find_map(|word| word.end_seconds);
        if let (Some(start), Some(end)) = (current_start, current_end) {
            if end >= start && end - start >= WORD_SEGMENT_MAX_DURATION_SECONDS {
                return true;
            }
        }

        if let (Some(end), Some(next_start)) = (current_end, next_word.start_seconds) {
            if next_start > end && next_start - end > WORD_SEGMENT_GAP_BREAK_SECONDS {
                return true;
            }
        }

        current_words.len() >= WORD_SEGMENT_MIN_TERMINAL_WORDS
            && Self::ends_with_strong_boundary(current_text)
    }

    fn flush_word_segment(
        segments: &mut Vec<TimedSegment>,
        current_text: &mut String,
        current_words: &mut Vec<TimedWord>,
    ) {
        let text = current_text.trim().to_string();
        if !text.is_empty() {
            let words = std::mem::take(current_words);
            segments.push(TimedSegment {
                text,
                start_seconds: words.iter().find_map(|word| word.start_seconds),
                end_seconds: words.iter().rev().find_map(|word| word.end_seconds),
                speaker_id: None,
                speaker_label: None,
                words,
            });
        }
        current_text.clear();
    }

    fn join_text_parts(left: &str, right: &str) -> String {
        let left = left.trim();
        let right = right.trim();
        if left.is_empty() {
            return right.to_string();
        }
        if right.is_empty() {
            return left.to_string();
        }
        if left.ends_with('-') {
            return format!("{left}{right}");
        }
        format!("{left} {right}")
    }

    fn ends_with_strong_boundary(value: &str) -> bool {
        value.ends_with('.') || value.ends_with('!') || value.ends_with('?') || value.ends_with('…')
    }

    fn segment_from_json(segment: ParakeetJsonSegment) -> Option<TimedSegment> {
        let text = Self::clean_transcript_text(&segment.text);
        if text.is_empty() {
            return None;
        }
        let words = segment
            .words
            .into_iter()
            .filter_map(Self::word_from_json)
            .collect::<Vec<_>>();

        Some(TimedSegment {
            text,
            start_seconds: segment.start.filter(|value| value.is_finite()),
            end_seconds: segment.end.filter(|value| value.is_finite()),
            speaker_id: None,
            speaker_label: None,
            words,
        })
    }

    fn word_from_json(word: ParakeetJsonWord) -> Option<TimedWord> {
        let text = Self::clean_transcript_text(&word.w);
        if text.is_empty() {
            return None;
        }
        Some(TimedWord {
            text,
            start_seconds: word.start.filter(|value| value.is_finite()),
            end_seconds: word.end.filter(|value| value.is_finite()),
            confidence: word.conf.filter(|value| value.is_finite()),
        })
    }

    fn clean_stream_line(raw_line: &str) -> String {
        let cleaned = raw_line
            .replace("\u{001b}[2K", "")
            .replace("\u{001b}[0m", "")
            .replace("[2K]", "")
            .replace("[BLANK_AUDIO]", "")
            .replace("<EOU>", "")
            .trim_start_matches("[stream:final]")
            .trim_start_matches("[stream]")
            .split('\r')
            .next_back()
            .unwrap_or("")
            .trim()
            .to_string();
        Self::clean_transcript_text(&cleaned)
    }

    fn stream_line_is_noise(text: &str) -> bool {
        const PREFIXES: [&str; 14] = [
            "init:",
            "main:",
            "ggml_",
            "ggml-",
            "parakeet_",
            "system_info:",
            "load_model:",
            "backend:",
            "ggml_backend",
            "ggml_metal",
            "pk::",
            "n_threads",
            "transcribe:",
            "sampling_",
        ];

        let trimmed = text.trim();
        let trimmed = trimmed
            .strip_prefix("[parakeet]")
            .map(str::trim_start)
            .unwrap_or(trimmed);
        trimmed.is_empty()
            || trimmed.starts_with('{')
            || trimmed.ends_with('}')
            || Self::looks_like_word_timestamp_line(trimmed)
            || PREFIXES.iter().any(|prefix| trimmed.starts_with(prefix))
    }

    fn looks_like_word_timestamp_line(text: &str) -> bool {
        let Some(first_token) = text.split_whitespace().next() else {
            return false;
        };
        let Some((start, end)) = first_token.split_once('-') else {
            return false;
        };
        start.parse::<f32>().is_ok() && end.parse::<f32>().is_ok()
    }

    fn parse_timecode_seconds(value: &str) -> Option<f32> {
        let parts = value.trim().split(':').collect::<Vec<_>>();
        if parts.len() == 3 {
            let hours = parts[0].parse::<f32>().ok()?;
            let minutes = parts[1].parse::<f32>().ok()?;
            let seconds = parts[2].replace(',', ".").parse::<f32>().ok()?;
            return Some(hours * 3600.0 + minutes * 60.0 + seconds);
        }
        if parts.len() == 2 {
            let minutes = parts[0].parse::<f32>().ok()?;
            let seconds = parts[1].replace(',', ".").parse::<f32>().ok()?;
            return Some(minutes * 60.0 + seconds);
        }
        value
            .trim()
            .replace(',', ".")
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite())
    }

    fn stream_line_text_and_progress(line: &str) -> Option<(String, Option<f32>)> {
        let cleaned = Self::clean_stream_line(line);
        if Self::stream_line_is_noise(&cleaned) {
            return None;
        }

        let mut progress_seconds = None;
        let mut text = cleaned.as_str();
        if let Some(end_bracket) = cleaned.find(']') {
            if cleaned.starts_with('[') {
                let bracket = cleaned[1..end_bracket].trim();
                if let Some((_, end_value)) = bracket.split_once("-->") {
                    progress_seconds = Self::parse_timecode_seconds(end_value.trim());
                    text = cleaned[end_bracket + 1..].trim();
                }
            }
        }

        let text = if let Some(eou_index) = text.find("[EOU @") {
            let marker = &text[eou_index + "[EOU @".len()..];
            if let Some(end_marker) = marker.find('s') {
                progress_seconds = Self::parse_timecode_seconds(&marker[..end_marker]);
            }
            text[..eou_index].trim()
        } else {
            text
        };

        let text = text.trim();
        if text.is_empty() || Self::stream_line_is_noise(text) {
            return None;
        }

        Some((text.to_string(), progress_seconds))
    }

    fn merge_preview(previous: &str, incoming: &str) -> String {
        let next = incoming.trim();
        if next.is_empty() {
            return previous.to_string();
        }
        let current = previous.trim_end();
        if current.is_empty() {
            return next.to_string();
        }
        if current == next || current.contains(next) {
            return previous.to_string();
        }
        if next.starts_with(current) {
            return next.to_string();
        }

        let overlap_limit = current.len().min(next.len());
        for size in (1..=overlap_limit).rev() {
            if !current.is_char_boundary(current.len() - size) || !next.is_char_boundary(size) {
                continue;
            }
            if current.ends_with(&next[..size]) {
                return format!("{}{}", current, &next[size..]);
            }
        }

        format!("{current}\n{next}")
    }

    fn emit_final_preview_snapshots(
        result: &TranscriptionOutput,
        existing_preview_delta_count: usize,
        existing_preview_text: &str,
        emit_partial: &(dyn Fn(String) + Send + Sync),
    ) {
        if existing_preview_delta_count >= 2 && existing_preview_text.trim() == result.text.trim() {
            return;
        }

        let snapshots = Self::final_preview_snapshots(result);
        for snapshot in snapshots {
            if snapshot.trim().is_empty() || snapshot.trim() == existing_preview_text.trim() {
                continue;
            }
            emit_partial(format!("{DELTA_REPLACE_PREFIX}{snapshot}"));
        }
    }

    fn final_preview_snapshots(result: &TranscriptionOutput) -> Vec<String> {
        let mut snapshots = Vec::new();
        let mut cumulative = String::new();

        for segment in &result.segments {
            let text = segment.text.trim();
            if text.is_empty() {
                continue;
            }
            cumulative = Self::join_text_parts(&cumulative, text);
            snapshots.push(cumulative.clone());
            if snapshots.len() >= 3 {
                break;
            }
        }

        if snapshots.len() >= 2 {
            if snapshots.last().map(String::as_str) != Some(result.text.trim()) {
                snapshots.push(result.text.trim().to_string());
            }
            return Self::dedupe_snapshots(snapshots);
        }

        let words = result
            .segments
            .iter()
            .flat_map(|segment| segment.words.iter())
            .map(|word| word.text.trim())
            .filter(|word| !word.is_empty())
            .collect::<Vec<_>>();
        if words.len() >= 2 {
            let midpoint = (words.len() / 2).max(1);
            snapshots.push(words[..midpoint].join(" "));
            snapshots.push(words.join(" "));
            if snapshots.last().map(String::as_str) != Some(result.text.trim()) {
                snapshots.push(result.text.trim().to_string());
            }
            return Self::dedupe_snapshots(snapshots);
        }

        let text = result.text.trim();
        let text_words = text.split_whitespace().collect::<Vec<_>>();
        if text_words.len() >= 2 {
            let midpoint = (text_words.len() / 2).max(1);
            snapshots.push(text_words[..midpoint].join(" "));
            snapshots.push(text.to_string());
        } else if !text.is_empty() {
            snapshots.push(text.to_string());
        }

        Self::dedupe_snapshots(snapshots)
    }

    fn dedupe_snapshots(snapshots: Vec<String>) -> Vec<String> {
        let mut unique = Vec::new();
        for snapshot in snapshots {
            let snapshot = snapshot.trim().to_string();
            if snapshot.is_empty() || unique.last() == Some(&snapshot) {
                continue;
            }
            unique.push(snapshot);
        }
        unique
    }

    async fn consume_preview_stream<R>(
        reader: R,
        state: Arc<Mutex<PreviewStreamState>>,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
    ) -> Result<String, ApplicationError>
    where
        R: AsyncRead + Unpin,
    {
        let mut reader = tokio::io::BufReader::new(reader);
        let mut buffer = [0_u8; 2048];
        let mut pending = Vec::<u8>::new();
        let mut raw_output = String::new();

        loop {
            let read = reader.read(&mut buffer).await.map_err(|error| {
                ApplicationError::SpeechToText(format!(
                    "failed to read parakeet-cli stream output: {error}"
                ))
            })?;
            if read == 0 {
                break;
            }
            pending.extend_from_slice(&buffer[..read]);
            raw_output.push_str(&String::from_utf8_lossy(&buffer[..read]));

            let mut record_start = 0usize;
            let mut consumed = 0usize;
            for (index, byte) in pending.iter().copied().enumerate() {
                if byte != b'\n' && byte != b'\r' {
                    continue;
                }
                if index > record_start {
                    let raw = String::from_utf8_lossy(&pending[record_start..index]).to_string();
                    Self::process_preview_record(
                        &raw,
                        &state,
                        emit_partial.as_ref(),
                        emit_progress_seconds.as_ref(),
                    );
                }
                record_start = index + 1;
                consumed = record_start;
            }

            if consumed > 0 {
                pending.drain(0..consumed);
            }
        }

        if !pending.is_empty() {
            let raw = String::from_utf8_lossy(&pending).to_string();
            Self::process_preview_record(
                &raw,
                &state,
                emit_partial.as_ref(),
                emit_progress_seconds.as_ref(),
            );
        }

        Ok(raw_output)
    }

    fn process_preview_record(
        raw: &str,
        state: &Arc<Mutex<PreviewStreamState>>,
        emit_partial: &(dyn Fn(String) + Send + Sync),
        emit_progress_seconds: &(dyn Fn(f32) + Send + Sync),
    ) {
        let Some((text, progress_seconds)) = Self::stream_line_text_and_progress(raw) else {
            return;
        };

        let preview = {
            let mut state = state.lock().expect("parakeet preview state lock poisoned");
            let next_preview = Self::merge_preview(&state.preview, &text);
            if next_preview == state.preview {
                return;
            }
            state.preview = next_preview;
            state.delta_count += 1;
            state.preview.clone()
        };
        emit_partial(format!("{DELTA_REPLACE_PREFIX}{preview}"));
        if let Some(seconds) = progress_seconds {
            emit_progress_seconds(seconds);
        }
    }

    async fn run_progressive_preview(
        &self,
        input_wav: &Path,
        preview_model_path: &Path,
        state: Arc<Mutex<PreviewStreamState>>,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
        language_code: &str,
    ) -> Result<(), ApplicationError> {
        let mut command = Command::new(&self.binary_path);
        Self::configure_command_environment(&mut command, &self.binary_path);
        command
            .arg("transcribe")
            .arg("--model")
            .arg(preview_model_path)
            .arg("--input")
            .arg(input_wav)
            .arg("--stream")
            .arg("--timestamps");
        if preview_model_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(Self::is_nemotron_streaming_model)
            .unwrap_or(false)
        {
            command
                .arg("--lang")
                .arg(Self::parakeet_target_lang(language_code));
        }
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        command.kill_on_drop(true);

        let mut child = command.spawn().map_err(|error| {
            ApplicationError::SpeechToText(format!(
                "parakeet-cli stream preview failed to start at '{}': {error}. Configure Parakeet CLI path in Settings > Local Models.",
                self.binary_path
            ))
        })?;

        let stdout = child.stdout.take().ok_or_else(|| {
            ApplicationError::SpeechToText("missing parakeet-cli preview stdout pipe".to_string())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            ApplicationError::SpeechToText("missing parakeet-cli preview stderr pipe".to_string())
        })?;

        let stdout_task = tokio::spawn(Self::consume_preview_stream(
            stdout,
            state.clone(),
            emit_partial.clone(),
            emit_progress_seconds.clone(),
        ));
        let stderr_task = tokio::spawn(Self::consume_preview_stream(
            stderr,
            state.clone(),
            emit_partial.clone(),
            emit_progress_seconds,
        ));

        let status = child.wait().await.map_err(|error| {
            ApplicationError::SpeechToText(format!(
                "failed to wait for parakeet-cli stream preview: {error}"
            ))
        })?;
        stdout_task.await.map_err(|error| {
            ApplicationError::SpeechToText(format!(
                "parakeet-cli preview reader task failed: {error}"
            ))
        })??;
        let stderr_output = stderr_task
            .await
            .map_err(|error| {
                ApplicationError::SpeechToText(format!(
                    "parakeet-cli preview stderr reader task failed: {error}"
                ))
            })?
            .unwrap_or_default();

        if !status.success() {
            return Err(ApplicationError::SpeechToText(format!(
                "parakeet-cli stream preview failed: {}",
                if stderr_output.trim().is_empty() {
                    status.to_string()
                } else {
                    stderr_output.trim().to_string()
                }
            )));
        }

        let preview = state
            .lock()
            .expect("parakeet preview state lock poisoned")
            .preview
            .trim()
            .to_string();
        if !preview.is_empty() {
            emit_partial(format!("{DELTA_REPLACE_PREFIX}{preview}"));
        }

        Ok(())
    }

    fn prepare_preview_chunks(
        input_wav: &Path,
    ) -> Result<(TempDir, Vec<PreviewChunk>), ApplicationError> {
        let reader = hound::WavReader::open(input_wav).map_err(|error| {
            ApplicationError::SpeechToText(format!(
                "Parakeet progressive preview could not read WAV chunks from {}: {error}",
                input_wav.display()
            ))
        })?;
        let spec = reader.spec();
        let channels = u64::from(spec.channels.max(1));
        let samples_per_chunk = ((spec.sample_rate as f32 * PREVIEW_CHUNK_SECONDS).round() as u64
            * channels)
            .max(channels);
        let temp_dir = tempfile::Builder::new()
            .prefix("sbobino-parakeet-preview-")
            .tempdir()
            .map_err(|error| {
                ApplicationError::SpeechToText(format!(
                    "failed to create Parakeet preview chunk directory: {error}"
                ))
            })?;

        let chunks = match spec.sample_format {
            hound::SampleFormat::Float => Self::write_typed_preview_chunks::<f32>(
                reader,
                spec,
                temp_dir.path(),
                samples_per_chunk,
            )?,
            hound::SampleFormat::Int if spec.bits_per_sample <= 16 => {
                Self::write_typed_preview_chunks::<i16>(
                    reader,
                    spec,
                    temp_dir.path(),
                    samples_per_chunk,
                )?
            }
            hound::SampleFormat::Int => Self::write_typed_preview_chunks::<i32>(
                reader,
                spec,
                temp_dir.path(),
                samples_per_chunk,
            )?,
        };

        Ok((temp_dir, chunks))
    }

    fn write_typed_preview_chunks<T>(
        mut reader: hound::WavReader<std::io::BufReader<std::fs::File>>,
        spec: hound::WavSpec,
        temp_dir: &Path,
        samples_per_chunk: u64,
    ) -> Result<Vec<PreviewChunk>, ApplicationError>
    where
        T: hound::Sample + Copy,
    {
        let channels = u64::from(spec.channels.max(1));
        let sample_rate = spec.sample_rate.max(1) as f32;
        let mut chunks = Vec::new();
        let mut writer: Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>> = None;
        let mut chunk_path = PathBuf::new();
        let mut chunk_start_sample = 0_u64;
        let mut chunk_sample_count = 0_u64;
        let mut total_samples = 0_u64;

        for sample in reader.samples::<T>() {
            if writer.is_none() {
                chunk_start_sample = total_samples;
                chunk_sample_count = 0;
                chunk_path = temp_dir.join(format!("chunk-{:04}.wav", chunks.len()));
                writer = Some(
                    hound::WavWriter::create(&chunk_path, spec).map_err(|error| {
                        ApplicationError::SpeechToText(format!(
                            "failed to create Parakeet preview chunk {}: {error}",
                            chunk_path.display()
                        ))
                    })?,
                );
            }

            let sample = sample.map_err(|error| {
                ApplicationError::SpeechToText(format!(
                    "failed to decode WAV sample for Parakeet preview: {error}"
                ))
            })?;
            if let Some(writer) = writer.as_mut() {
                writer.write_sample(sample).map_err(|error| {
                    ApplicationError::SpeechToText(format!(
                        "failed to write Parakeet preview chunk {}: {error}",
                        chunk_path.display()
                    ))
                })?;
            }
            total_samples = total_samples.saturating_add(1);
            chunk_sample_count = chunk_sample_count.saturating_add(1);

            if chunk_sample_count >= samples_per_chunk {
                Self::finish_preview_chunk(
                    &mut chunks,
                    writer.take(),
                    &chunk_path,
                    chunk_start_sample,
                    chunk_sample_count,
                    channels,
                    sample_rate,
                )?;
            }
        }

        if writer.is_some() {
            Self::finish_preview_chunk(
                &mut chunks,
                writer.take(),
                &chunk_path,
                chunk_start_sample,
                chunk_sample_count,
                channels,
                sample_rate,
            )?;
        }

        Ok(chunks)
    }

    fn finish_preview_chunk(
        chunks: &mut Vec<PreviewChunk>,
        writer: Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>>,
        chunk_path: &Path,
        chunk_start_sample: u64,
        chunk_sample_count: u64,
        channels: u64,
        sample_rate: f32,
    ) -> Result<(), ApplicationError> {
        if chunk_sample_count == 0 {
            return Ok(());
        }
        if let Some(writer) = writer {
            writer.finalize().map_err(|error| {
                ApplicationError::SpeechToText(format!(
                    "failed to finalize Parakeet preview chunk {}: {error}",
                    chunk_path.display()
                ))
            })?;
        }
        let start_seconds = (chunk_start_sample / channels) as f32 / sample_rate;
        let end_seconds =
            ((chunk_start_sample + chunk_sample_count) / channels) as f32 / sample_rate;
        chunks.push(PreviewChunk {
            path: chunk_path.to_path_buf(),
            start_seconds,
            end_seconds,
        });
        Ok(())
    }

    async fn run_preview_json_for_chunk(
        &self,
        chunk_path: &Path,
        preview_model_path: &Path,
        language_code: &str,
    ) -> Result<String, ApplicationError> {
        let mut command = Command::new(&self.binary_path);
        Self::configure_command_environment(&mut command, &self.binary_path);
        command
            .arg("transcribe")
            .arg("--model")
            .arg(preview_model_path)
            .arg("--input")
            .arg(chunk_path)
            .arg("--json");
        if preview_model_path
            .file_name()
            .and_then(|name| name.to_str())
            .map(Self::is_nemotron_streaming_model)
            .unwrap_or(false)
        {
            command
                .arg("--lang")
                .arg(Self::parakeet_target_lang(language_code));
        }
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let output = tokio::time::timeout(PREVIEW_CHUNK_TIMEOUT, command.output())
            .await
            .map_err(|_| {
                ApplicationError::SpeechToText(format!(
                    "parakeet-cli chunk preview timed out after {PREVIEW_CHUNK_TIMEOUT:?}"
                ))
            })?
            .map_err(|error| {
                ApplicationError::SpeechToText(format!(
                    "parakeet-cli chunk preview failed to start at '{}': {error}",
                    self.binary_path
                ))
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(ApplicationError::SpeechToText(format!(
                "parakeet-cli chunk preview failed: {}",
                if stderr.is_empty() {
                    output.status.to_string()
                } else {
                    stderr
                }
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let parsed = Self::parse_json_output(&stdout, None)?;
        Ok(parsed.text)
    }

    async fn run_chunked_progressive_preview(
        &self,
        input_wav: &Path,
        preview_model_path: &Path,
        state: Arc<Mutex<PreviewStreamState>>,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
        language_code: &str,
    ) -> Result<(), ApplicationError> {
        let (_temp_dir, chunks) = match Self::prepare_preview_chunks(input_wav) {
            Ok(chunks) => chunks,
            Err(error) => {
                eprintln!(
                    "Parakeet chunked preview unavailable, falling back to stream preview: {error}"
                );
                return self
                    .run_progressive_preview(
                        input_wav,
                        preview_model_path,
                        state,
                        emit_partial,
                        emit_progress_seconds,
                        language_code,
                    )
                    .await;
            }
        };
        if chunks.is_empty() {
            return Ok(());
        }

        for chunk in chunks.into_iter().take(PREVIEW_MAX_CHUNKS) {
            let text = self
                .run_preview_json_for_chunk(&chunk.path, preview_model_path, language_code)
                .await?;
            let preview = {
                let mut state = state.lock().expect("parakeet preview state lock poisoned");
                let next_preview = Self::merge_preview(&state.preview, &text);
                if next_preview == state.preview {
                    continue;
                }
                state.preview = next_preview;
                state.delta_count += 1;
                state.preview.clone()
            };
            emit_partial(format!("{DELTA_REPLACE_PREFIX}{preview}"));
            emit_progress_seconds(chunk.end_seconds.max(chunk.start_seconds));
        }

        Ok(())
    }
}

#[async_trait]
impl SpeechToTextEngine for ParakeetCppEngine {
    async fn transcribe(
        &self,
        input_wav: &Path,
        model_filename: &str,
        language_code: &str,
        options: &WhisperOptions,
        total_audio_seconds: Option<f32>,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        if options.translate_to_english {
            return Err(ApplicationError::SpeechToText(
                "Parakeet.cpp does not support translate-to-English mode".to_string(),
            ));
        }

        let model_path = self.validate_model_exists(model_filename)?;
        let preview_model_path =
            self.validate_preview_model_exists(model_filename, language_code)?;
        let preview_state = Arc::new(Mutex::new(PreviewStreamState::default()));
        match tokio::time::timeout(
            PREVIEW_TIMEOUT,
            self.run_chunked_progressive_preview(
                input_wav,
                &preview_model_path,
                preview_state.clone(),
                emit_partial.clone(),
                emit_progress_seconds.clone(),
                language_code,
            ),
        )
        .await
        {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                eprintln!("Parakeet progressive preview unavailable: {error}");
            }
            Err(_) => {
                eprintln!("Parakeet progressive preview timed out after {PREVIEW_TIMEOUT:?}");
            }
        }

        let mut command = Command::new(&self.binary_path);
        Self::configure_command_environment(&mut command, &self.binary_path);
        command
            .arg("transcribe")
            .arg("--model")
            .arg(&model_path)
            .arg("--input")
            .arg(input_wav)
            .arg("--json");
        if Self::is_nemotron_streaming_model(model_filename) {
            command
                .arg("--lang")
                .arg(Self::parakeet_target_lang(language_code));
        }
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let output = command.output().await.map_err(|error| {
            ApplicationError::SpeechToText(format!(
                "parakeet-cli failed to start at '{}': {error}. Configure Parakeet CLI path in Settings > Local Models.",
                self.binary_path
            ))
        })?;

        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if !output.status.success() {
            return Err(ApplicationError::SpeechToText(format!(
                "parakeet-cli failed: {}",
                if stderr.is_empty() {
                    output.status.to_string()
                } else {
                    stderr
                }
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let result = Self::parse_json_output(&stdout, total_audio_seconds)?;
        let (preview_text, preview_delta_count) = {
            let preview_snapshot = preview_state
                .lock()
                .expect("parakeet preview state lock poisoned");
            (
                preview_snapshot.preview.trim().to_string(),
                preview_snapshot.delta_count,
            )
        };
        Self::emit_final_preview_snapshots(
            &result,
            preview_delta_count,
            &preview_text,
            emit_partial.as_ref(),
        );
        emit_partial(result.text.clone());
        if let Some(total) = total_audio_seconds {
            emit_progress_seconds(total);
        }
        Ok(result)
    }
}
