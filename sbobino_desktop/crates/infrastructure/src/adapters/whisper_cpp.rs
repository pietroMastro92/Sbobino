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
    collapse_consecutive_repeated_segments, minimize_transcript_repetitions, TimedSegment,
    TimedWord, TranscriptionOutput, WhisperOptions,
};

use crate::adapters::transcript_segmentation::normalize_transcript_segments;

static OUTPUT_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);
const DELTA_REPLACE_PREFIX: &str = "\u{001F}REPLACE:";
const PROCESS_WAIT_POLL_INTERVAL: Duration = Duration::from_secs(5);
const PROCESS_IDLE_TIMEOUT_MIN: Duration = Duration::from_secs(900);
const PROCESS_IDLE_TIMEOUT_MAX: Duration = Duration::from_secs(3600);

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
    transcription: Vec<WhisperCliJsonSegment>,
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

                Some(TimedSegment {
                    text,
                    start_seconds,
                    end_seconds,
                    speaker_id: speaker_label.clone(),
                    speaker_label,
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

    fn should_retry_with_cpu_fallback(error: &WhisperCliAttemptError) -> bool {
        let haystack = format!(
            "{}\n{}",
            error.message.to_ascii_lowercase(),
            error.stderr_output.to_ascii_lowercase()
        );

        let metal_failure = haystack.contains("ggml_metal")
            || haystack.contains("metal buffer")
            || haystack.contains("failed to allocate buffer")
            || haystack.contains("use gpu    = 1");

        let crashed = error
            .status
            .map(|status| !status.success())
            .unwrap_or(false)
            && status_signal_is_crash(error.status);

        metal_failure || crashed
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

        let mut command = Command::new(&self.binary_path);
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
                emit_partial(
                    "Whisper GPU execution failed; retrying in CPU-safe mode.".to_string(),
                );
                self.run_whisper_cli_attempt(
                    input_wav,
                    model_path,
                    language_code,
                    options,
                    total_audio_seconds,
                    emit_partial,
                    emit_progress_seconds,
                    WhisperCliExecutionMode::CpuFallback,
                )
                .await
                .map_err(|retry_error| ApplicationError::SpeechToText(retry_error.message))
            }
            Err(error) => Err(ApplicationError::SpeechToText(error.message)),
        }
    }
}

#[cfg(unix)]
fn status_signal_is_crash(status: Option<ExitStatus>) -> bool {
    matches!(
        status.and_then(|value| value.signal()),
        Some(11) | Some(6) | Some(10)
    )
}

#[cfg(not(unix))]
fn status_signal_is_crash(_status: Option<ExitStatus>) -> bool {
    false
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{WhisperCppEngine, PROCESS_IDLE_TIMEOUT_MAX, PROCESS_IDLE_TIMEOUT_MIN};

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
