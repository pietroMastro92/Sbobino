use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

use sbobino_application::{ApplicationError, SpeechToTextEngine};
use sbobino_domain::{TimedSegment, TimedWord, TranscriptionOutput, WhisperOptions};

use crate::adapters::transcript_segmentation::normalize_transcript_segments;

const DELTA_REPLACE_PREFIX: &str = "\u{001F}REPLACE:";
const REALTIME_EOU_F16_MODEL: &str = "realtime_eou_120m-v1-f16.gguf";
const REALTIME_EOU_Q8_MODEL: &str = "realtime_eou_120m-v1-q8_0.gguf";

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

    fn validate_preview_model_exists(
        &self,
        final_model_filename: &str,
    ) -> Result<PathBuf, ApplicationError> {
        if matches!(
            final_model_filename,
            REALTIME_EOU_F16_MODEL | REALTIME_EOU_Q8_MODEL
        ) {
            return self.validate_model_exists(final_model_filename);
        }

        for candidate in [REALTIME_EOU_F16_MODEL, REALTIME_EOU_Q8_MODEL] {
            let model_path = self.model_path(candidate);
            if model_path.exists() {
                return Ok(model_path);
            }
        }

        Err(ApplicationError::SpeechToText(format!(
            "Parakeet progressive preview requires {REALTIME_EOU_F16_MODEL} or {REALTIME_EOU_Q8_MODEL} in {}. Repair Parakeet models from Settings > Local Models.",
            self.models_dir
        )))
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

    fn parse_json_output(
        raw_stdout: &str,
        total_audio_seconds: Option<f32>,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        let payload = Self::extract_json_payload(raw_stdout)?;
        let parsed: ParakeetJsonOutput = serde_json::from_str(payload).map_err(|error| {
            ApplicationError::SpeechToText(format!("failed to parse parakeet-cli JSON: {error}"))
        })?;

        let text = parsed.text.trim().to_string();
        let segment_text = parsed
            .segments
            .iter()
            .map(|segment| segment.text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let text = if text.is_empty() { segment_text } else { text };
        if text.is_empty() {
            return Err(ApplicationError::SpeechToText(
                "parakeet-cli produced empty output".to_string(),
            ));
        }

        let raw_segments = if parsed.segments.is_empty() {
            Self::segments_from_words(text.clone(), parsed.words, total_audio_seconds)
        } else {
            parsed
                .segments
                .into_iter()
                .filter_map(Self::segment_from_json)
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

        vec![TimedSegment {
            text,
            start_seconds: words.iter().find_map(|word| word.start_seconds),
            end_seconds: words.iter().rev().find_map(|word| word.end_seconds),
            speaker_id: None,
            speaker_label: None,
            words,
        }]
    }

    fn segment_from_json(segment: ParakeetJsonSegment) -> Option<TimedSegment> {
        let text = segment.text.trim().to_string();
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
        let text = word.w.trim().to_string();
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
        raw_line
            .replace("\u{001b}[2K", "")
            .replace("\u{001b}[0m", "")
            .replace("[2K]", "")
            .replace("[BLANK_AUDIO]", "")
            .split('\r')
            .next_back()
            .unwrap_or("")
            .trim()
            .to_string()
    }

    fn stream_line_is_noise(text: &str) -> bool {
        const PREFIXES: [&str; 12] = [
            "init:",
            "main:",
            "ggml_",
            "ggml-",
            "parakeet_",
            "system_info:",
            "load_model:",
            "backend:",
            "pk::",
            "n_threads",
            "transcribe:",
            "sampling_",
        ];

        let trimmed = text.trim();
        trimmed.is_empty()
            || trimmed.starts_with('{')
            || trimmed.ends_with('}')
            || PREFIXES.iter().any(|prefix| trimmed.starts_with(prefix))
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

    async fn consume_preview_stream<R>(
        reader: R,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
    ) -> Result<String, ApplicationError>
    where
        R: AsyncRead + Unpin,
    {
        let mut reader = tokio::io::BufReader::new(reader);
        let mut buffer = [0_u8; 2048];
        let mut pending = Vec::<u8>::new();
        let mut preview = String::new();

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

            let mut record_start = 0usize;
            let mut consumed = 0usize;
            for (index, byte) in pending.iter().copied().enumerate() {
                if byte != b'\n' && byte != b'\r' {
                    continue;
                }
                if index > record_start {
                    let raw = String::from_utf8_lossy(&pending[record_start..index]).to_string();
                    if let Some((text, progress_seconds)) =
                        Self::stream_line_text_and_progress(&raw)
                    {
                        preview = Self::merge_preview(&preview, &text);
                        emit_partial(format!("{DELTA_REPLACE_PREFIX}{preview}"));
                        if let Some(seconds) = progress_seconds {
                            emit_progress_seconds(seconds);
                        }
                    }
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
            if let Some((text, progress_seconds)) = Self::stream_line_text_and_progress(&raw) {
                preview = Self::merge_preview(&preview, &text);
                emit_partial(format!("{DELTA_REPLACE_PREFIX}{preview}"));
                if let Some(seconds) = progress_seconds {
                    emit_progress_seconds(seconds);
                }
            }
        }

        Ok(preview)
    }

    async fn run_progressive_preview(
        &self,
        input_wav: &Path,
        preview_model_path: &Path,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
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
            .arg("--timestamps")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

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
            emit_partial.clone(),
            emit_progress_seconds,
        ));
        let stderr_task = tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stderr);
            let mut output = String::new();
            let _ = reader.read_to_string(&mut output).await;
            output
        });

        let status = child.wait().await.map_err(|error| {
            ApplicationError::SpeechToText(format!(
                "failed to wait for parakeet-cli stream preview: {error}"
            ))
        })?;
        let preview = stdout_task.await.map_err(|error| {
            ApplicationError::SpeechToText(format!(
                "parakeet-cli preview reader task failed: {error}"
            ))
        })??;
        let stderr_output = stderr_task.await.unwrap_or_default();

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

        if !preview.trim().is_empty() {
            emit_partial(format!("{DELTA_REPLACE_PREFIX}{}", preview.trim()));
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
        _language_code: &str,
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
        let preview_model_path = self.validate_preview_model_exists(model_filename)?;
        self.run_progressive_preview(
            input_wav,
            &preview_model_path,
            emit_partial.clone(),
            emit_progress_seconds.clone(),
        )
        .await?;

        let mut command = Command::new(&self.binary_path);
        Self::configure_command_environment(&mut command, &self.binary_path);
        command
            .arg("transcribe")
            .arg("--model")
            .arg(&model_path)
            .arg("--input")
            .arg(input_wav)
            .arg("--json")
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
        emit_partial(result.text.clone());
        if let Some(total) = total_audio_seconds {
            emit_progress_seconds(total);
        }
        Ok(result)
    }
}
