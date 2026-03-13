use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tokio::fs;
use tokio::io::AsyncRead;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use sbobino_application::{ApplicationError, SpeechToTextEngine};
use sbobino_domain::{
    collapse_consecutive_repeated_segments, minimize_transcript_repetitions, TimedSegment,
    TimedWord, TranscriptionOutput, WhisperOptions,
};

use crate::adapters::transcript_segmentation::normalize_transcript_segments;

static OUTPUT_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone)]
pub struct WhisperCppEngine {
    binary_path: String,
    models_dir: String,
}

#[derive(Default)]
struct TranscriptCollector {
    segments: Vec<TimedSegment>,
}

enum ParsedCliEvent {
    Segment(TimedSegment),
    ProgressPercent(f32),
}

impl WhisperCppEngine {
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
            let ss = parts[2].parse::<f32>().ok()?;
            Some((hh * 3600.0) + (mm * 60.0) + ss)
        } else if parts.len() == 2 {
            let mm = parts[0].parse::<f32>().ok()?;
            let ss = parts[1].parse::<f32>().ok()?;
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

    fn parse_cli_line(raw_line: &str) -> Option<ParsedCliEvent> {
        let cleaned = raw_line
            .replace("\u{001b}[2K", "")
            .replace("\u{001b}[0m", "")
            .replace("[2K]", "")
            .replace("[BLANK_AUDIO]", "")
            .split('\r')
            .last()
            .unwrap_or("")
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

        let words = Self::build_word_candidates(&normalized, start_seconds, end_seconds);
        let segment = TimedSegment {
            text: normalized,
            start_seconds,
            end_seconds,
            speaker_id: None,
            speaker_label: None,
            words,
        };

        Some(ParsedCliEvent::Segment(segment))
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
    ) {
        if let Ok(mut state) = collector.lock() {
            state.segments.push(segment.clone());
        }

        emit_partial(segment.text);
    }

    fn join_segment_text(segments: &[TimedSegment]) -> String {
        segments
            .iter()
            .map(|segment| segment.text.trim())
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    async fn consume_stream<R>(
        reader: R,
        collector: Arc<Mutex<TranscriptCollector>>,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
        total_audio_seconds: Option<f32>,
    ) -> Result<Vec<String>, ApplicationError>
    where
        R: AsyncRead + Unpin,
    {
        use tokio::io::AsyncBufReadExt;

        let mut lines = tokio::io::BufReader::new(reader).lines();
        let mut raw_lines = Vec::<String>::new();

        while let Ok(Some(raw)) = lines.next_line().await {
            raw_lines.push(raw.clone());
            if let Some(parsed_line) = Self::parse_cli_line(&raw) {
                match parsed_line {
                    ParsedCliEvent::Segment(segment) => {
                        if let Some(end_seconds) = segment.end_seconds {
                            emit_progress_seconds(end_seconds);
                        }
                        Self::collect_segment(&collector, &emit_partial, segment);
                    }
                    ParsedCliEvent::ProgressPercent(progress_percent) => {
                        if let Some(total_seconds) = total_audio_seconds.filter(|v| *v > 0.0) {
                            let estimated_seconds = (progress_percent / 100.0) * total_seconds;
                            emit_progress_seconds(estimated_seconds);
                        }
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

        let mut command = Command::new(&self.binary_path);

        // Homebrew-installed whisper-cli links against @rpath/libggml.0.dylib but
        // ships with no embedded rpath. We resolve this by setting DYLD_LIBRARY_PATH
        // to the sibling libexec/lib directory where the dylibs actually live.
        if let Some(binary_dir) = Path::new(&self.binary_path)
            .canonicalize()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        {
            let libexec_lib = binary_dir.join("../libexec/lib");
            let sibling_lib = binary_dir.join("../lib");

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
            // Also preserve any existing DYLD_LIBRARY_PATH
            if let Ok(existing) = std::env::var("DYLD_LIBRARY_PATH") {
                dyld_paths.push(existing);
            }
            if !dyld_paths.is_empty() {
                command.env("DYLD_LIBRARY_PATH", dyld_paths.join(":"));
            }
        }

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

        if language_code != "auto" {
            command.arg("-l").arg(language_code);
        }

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

        command
            .arg("-otxt")
            .arg("-pp")
            .arg("-of")
            .arg(&output_base)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = command.spawn().map_err(|e| {
            ApplicationError::SpeechToText(format!(
                "whisper-cli failed to start at '{}': {e}. Configure Whisper CLI path in Settings > Local Models.",
                self.binary_path
            ))
        })?;

        let stdout = child.stdout.take().ok_or_else(|| {
            ApplicationError::SpeechToText("missing whisper-cli stdout pipe".to_string())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            ApplicationError::SpeechToText("missing whisper-cli stderr pipe".to_string())
        })?;

        let collected = Arc::new(Mutex::new(TranscriptCollector::default()));

        let stdout_emit = emit_partial.clone();
        let stdout_progress = emit_progress_seconds.clone();
        let stdout_collector = collected.clone();
        let stdout_total_seconds = total_audio_seconds;
        let stdout_task = tokio::spawn(async move {
            Self::consume_stream(
                stdout,
                stdout_collector,
                stdout_emit,
                stdout_progress,
                stdout_total_seconds,
            )
            .await
        });

        let stderr_emit = emit_partial.clone();
        let stderr_progress = emit_progress_seconds.clone();
        let stderr_collector = collected.clone();
        let stderr_total_seconds = total_audio_seconds;
        let stderr_task = tokio::spawn(async move {
            Self::consume_stream(
                stderr,
                stderr_collector,
                stderr_emit,
                stderr_progress,
                stderr_total_seconds,
            )
            .await
        });

        let status = match timeout(Duration::from_secs(900), child.wait()).await {
            Ok(wait_result) => wait_result.map_err(|e| {
                ApplicationError::SpeechToText(format!("failed to wait for whisper-cli: {e}"))
            })?,
            Err(_) => {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(ApplicationError::SpeechToText(
                    "whisper-cli timed out after 900s".to_string(),
                ));
            }
        };

        let _stdout_lines = stdout_task.await.map_err(|e| {
            ApplicationError::SpeechToText(format!("stdout reader task failed: {e}"))
        })??;

        let stderr_lines = stderr_task.await.map_err(|e| {
            ApplicationError::SpeechToText(format!("stderr reader task failed: {e}"))
        })??;
        let stderr_output = stderr_lines.join("\n");

        let segments = if let Ok(state) = collected.lock() {
            collapse_consecutive_repeated_segments(&state.segments)
        } else {
            Vec::new()
        };

        if !status.success() {
            return Err(ApplicationError::SpeechToText(format!(
                "whisper-cli failed: {}",
                stderr_output.trim()
            )));
        }

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

        let transcript = transcript_from_file.unwrap_or_else(|| Self::join_segment_text(&segments));
        let transcript = minimize_transcript_repetitions(&transcript);

        let _ = fs::remove_file(&output_txt_path).await;

        if transcript.is_empty() {
            return Err(ApplicationError::SpeechToText(
                "whisper-cli produced empty output".to_string(),
            ));
        }

        let segments = normalize_transcript_segments(&transcript, &segments, total_audio_seconds);

        Ok(TranscriptionOutput {
            text: transcript,
            segments,
        })
    }
}

#[async_trait]
impl SpeechToTextEngine for WhisperCppEngine {
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
        let model_path = self.validate_model_exists(model_filename)?;
        self.transcribe_with_cli(
            input_wav,
            &model_path,
            language_code,
            options,
            total_audio_seconds,
            emit_partial,
            emit_progress_seconds,
        )
        .await
    }
}
