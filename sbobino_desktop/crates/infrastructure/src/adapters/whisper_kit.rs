use std::net::TcpListener;
use std::path::Path;
use std::process::ExitStatus;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::multipart;
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio::time::{timeout, Duration, Instant};
use tracing::warn;

use sbobino_application::{ApplicationError, SpeechToTextEngine};
use sbobino_domain::{
    collapse_consecutive_repeated_segments, minimize_transcript_repetitions, TimedSegment,
    TranscriptionOutput, WhisperOptions,
};

use crate::adapters::transcript_segmentation::normalize_transcript_segments;

#[derive(Debug, Clone)]
pub struct WhisperKitEngine {
    binary_path: String,
    models_dir: String,
}

const DELTA_REPLACE_PREFIX: &str = "\u{001F}REPLACE:";
const SERVER_STARTUP_ATTEMPTS: usize = 1200;
const SERVER_STARTUP_DELAY: Duration = Duration::from_millis(250);
const PROCESS_WAIT_POLL_INTERVAL: Duration = Duration::from_secs(5);
const PROCESS_IDLE_TIMEOUT_MIN: Duration = Duration::from_secs(900);
const PROCESS_IDLE_TIMEOUT_MAX: Duration = Duration::from_secs(3600);

#[derive(Debug, Deserialize, Default)]
struct WhisperKitSseEvent {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    delta: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    progress: Option<f32>,
    #[serde(default)]
    current_time: Option<f32>,
    #[serde(default)]
    total_time: Option<f32>,
}

impl WhisperKitEngine {
    pub fn new(binary_path: String, models_dir: String) -> Self {
        Self {
            binary_path,
            models_dir,
        }
    }

    fn resolve_model_name(model_filename: &str) -> Result<&'static str, ApplicationError> {
        let basename = Path::new(model_filename)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(model_filename)
            .trim()
            .to_ascii_lowercase();

        if basename.contains("large-v3-turbo") {
            return Ok("large-v3-turbo");
        }
        if basename == "medium"
            || basename.starts_with("medium-")
            || basename.contains("ggml-medium")
        {
            return Ok("medium");
        }
        if basename == "small" || basename.starts_with("small-") || basename.contains("ggml-small")
        {
            return Ok("small");
        }
        if basename == "base" || basename.starts_with("base-") || basename.contains("ggml-base") {
            return Ok("base");
        }
        if basename == "tiny" || basename.starts_with("tiny-") || basename.contains("ggml-tiny") {
            return Ok("tiny");
        }

        Err(ApplicationError::SpeechToText(format!(
            "unsupported WhisperKit model mapping for '{}'. Expected tiny, base, small, medium, or large-v3-turbo ggml filenames.",
            model_filename
        )))
    }

    fn map_compute_units(value: &str) -> &'static str {
        match value.trim().to_ascii_lowercase().as_str() {
            "all" => "all",
            "cpu_only" | "cpuonly" => "cpuOnly",
            "cpu_and_gpu" | "cpuandgpu" => "cpuAndGPU",
            "cpu_and_neural_engine" | "cpuandneuralengine" => "cpuAndNeuralEngine",
            "random" => "random",
            _ => "cpuAndNeuralEngine",
        }
    }

    fn map_chunking_strategy(value: &str) -> &'static str {
        if value.trim().eq_ignore_ascii_case("none") {
            "none"
        } else {
            "vad"
        }
    }

    fn format_cli_float(value: f32, fallback: f32) -> String {
        let sanitized = if value.is_finite() { value } else { fallback };
        let normalized = if sanitized.abs() < f32::EPSILON {
            0.0
        } else {
            sanitized
        };
        let mut rendered = format!("{normalized:.6}");
        while rendered.contains('.') && rendered.ends_with('0') {
            rendered.pop();
        }
        if rendered.ends_with('.') {
            rendered.push('0');
        }
        rendered
    }

    fn wav_duration_seconds(wav_path: &Path) -> Option<f32> {
        let reader = hound::WavReader::open(wav_path).ok()?;
        let spec = reader.spec();
        if spec.channels == 0 || spec.sample_rate == 0 {
            return None;
        }

        let samples = reader.duration() as f32;
        let frames = samples / f32::from(spec.channels);
        if frames <= 0.0 {
            return None;
        }

        Some(frames / (spec.sample_rate as f32))
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
        label: &str,
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
                            "failed to wait for {label}: {error}"
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
                        "{label} stopped producing output for {}s and was terminated",
                        idle_timeout.as_secs()
                    )));
                }
            }
        }
    }

    fn strip_ansi_codes(input: &str) -> String {
        let mut output = String::with_capacity(input.len());
        let mut chars = input.chars().peekable();

        while let Some(ch) = chars.next() {
            if ch == '\u{001b}' {
                if matches!(chars.peek(), Some('[')) {
                    let _ = chars.next();
                    for next in chars.by_ref() {
                        if ('@'..='~').contains(&next) {
                            break;
                        }
                    }
                }
                continue;
            }
            output.push(ch);
        }

        output
    }

    fn sanitize_output_line(raw_line: &str) -> String {
        Self::strip_ansi_codes(raw_line)
            .replace('\u{0008}', "")
            .trim()
            .to_string()
    }

    fn parse_progress_percent(line: &str) -> Option<f32> {
        if !line.contains('%') || !line.contains("Elapsed Time:") {
            return None;
        }

        line.split_whitespace()
            .find_map(|token| token.strip_suffix('%'))
            .and_then(|value| value.trim().parse::<f32>().ok())
            .map(|value| value.clamp(0.0, 100.0))
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

    fn parse_timed_segment_line(raw_line: &str) -> Option<TimedSegment> {
        let cleaned = raw_line.trim();
        if cleaned.is_empty() {
            return None;
        }

        let mut start_seconds = None;
        let mut end_seconds = None;
        let text = if cleaned.starts_with('[') {
            match cleaned.find(']') {
                Some(end_index) => {
                    let marker = cleaned[1..end_index].trim();
                    if let Some((start, end)) = marker.split_once("-->") {
                        start_seconds = Self::parse_timecode_seconds(start.trim());
                        end_seconds = Self::parse_timecode_seconds(end.trim());
                    }
                    cleaned[end_index + 1..].trim()
                }
                None => cleaned,
            }
        } else {
            cleaned
        };

        if text.is_empty() {
            return None;
        }

        Some(TimedSegment {
            text: text.to_string(),
            start_seconds,
            end_seconds,
            speaker_id: None,
            speaker_label: None,
            words: Vec::new(),
        })
    }

    fn output_from_text(
        transcript: String,
        total_audio_seconds: Option<f32>,
    ) -> TranscriptionOutput {
        let parsed_segments = transcript
            .lines()
            .filter_map(Self::parse_timed_segment_line)
            .collect::<Vec<_>>();
        let collapsed_segments = collapse_consecutive_repeated_segments(&parsed_segments);
        let normalized_text = if collapsed_segments.is_empty() {
            minimize_transcript_repetitions(&transcript)
        } else {
            minimize_transcript_repetitions(
                &collapsed_segments
                    .iter()
                    .map(|segment| segment.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
            )
        };
        let segments = normalize_transcript_segments(
            &normalized_text,
            &collapsed_segments,
            total_audio_seconds,
        );

        TranscriptionOutput {
            text: normalized_text,
            segments,
        }
    }

    fn reserve_ephemeral_port() -> Result<u16, ApplicationError> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| {
            ApplicationError::SpeechToText(format!(
                "failed to reserve local WhisperKit server port: {e}"
            ))
        })?;
        let port = listener.local_addr().map_err(|e| {
            ApplicationError::SpeechToText(format!(
                "failed to read local WhisperKit server port: {e}"
            ))
        })?;
        Ok(port.port())
    }

    fn sse_event_boundary(buffer: &str) -> Option<(usize, usize)> {
        let lf = buffer.find("\n\n").map(|idx| (idx, 2));
        let crlf = buffer.find("\r\n\r\n").map(|idx| (idx, 4));
        match (lf, crlf) {
            (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
            (Some(left), None) => Some(left),
            (None, Some(right)) => Some(right),
            (None, None) => None,
        }
    }

    fn extract_sse_payload(raw_event: &str) -> Option<String> {
        let mut data_lines = Vec::<String>::new();
        for line in raw_event.lines() {
            let normalized = line.trim_end_matches('\r');
            if let Some(value) = normalized.strip_prefix("data:") {
                data_lines.push(value.trim_start().to_string());
            }
        }

        if data_lines.is_empty() {
            None
        } else {
            Some(data_lines.join("\n"))
        }
    }

    fn process_stream_line(
        raw_line: &str,
        collected_lines: &mut Vec<String>,
        track_progress: bool,
        total_audio_seconds: Option<f32>,
        last_progress_percent: &mut Option<f32>,
        emit_progress_seconds: &Arc<dyn Fn(f32) + Send + Sync>,
    ) {
        let cleaned = Self::sanitize_output_line(raw_line);
        if cleaned.is_empty() {
            return;
        }

        if track_progress {
            if let Some(percent) = Self::parse_progress_percent(&cleaned) {
                let should_emit = match *last_progress_percent {
                    Some(last) => percent > last + 0.2,
                    None => true,
                };
                if should_emit {
                    *last_progress_percent = Some(percent);
                    if let Some(total) = total_audio_seconds {
                        emit_progress_seconds((total * (percent / 100.0)).clamp(0.0, total));
                    }
                }
            }
        }

        collected_lines.push(cleaned);
    }

    async fn consume_stream<R>(
        reader: R,
        track_progress: bool,
        total_audio_seconds: Option<f32>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
        last_activity_at_ms: Arc<AtomicU64>,
    ) -> Result<Vec<String>, ApplicationError>
    where
        R: AsyncRead + Unpin,
    {
        let mut reader = BufReader::new(reader);
        let mut chunk = [0_u8; 4096];
        let mut pending = Vec::<u8>::new();
        let mut collected_lines = Vec::<String>::new();
        let mut last_progress_percent: Option<f32> = None;

        loop {
            let read = reader.read(&mut chunk).await.map_err(|e| {
                ApplicationError::SpeechToText(format!("failed to read whisperkit-cli stream: {e}"))
            })?;
            if read == 0 {
                break;
            }

            Self::mark_activity(last_activity_at_ms.as_ref());
            pending.extend_from_slice(&chunk[..read]);
            let mut start = 0_usize;
            let mut index = 0_usize;

            while index < pending.len() {
                if pending[index] == b'\n' || pending[index] == b'\r' {
                    if index > start {
                        let raw = String::from_utf8_lossy(&pending[start..index]).to_string();
                        Self::process_stream_line(
                            &raw,
                            &mut collected_lines,
                            track_progress,
                            total_audio_seconds,
                            &mut last_progress_percent,
                            &emit_progress_seconds,
                        );
                    }

                    index += 1;
                    while index < pending.len()
                        && (pending[index] == b'\n' || pending[index] == b'\r')
                    {
                        index += 1;
                    }
                    start = index;
                    continue;
                }
                index += 1;
            }

            if start > 0 {
                pending.drain(..start);
            }
        }

        if !pending.is_empty() {
            let raw = String::from_utf8_lossy(&pending).to_string();
            Self::process_stream_line(
                &raw,
                &mut collected_lines,
                track_progress,
                total_audio_seconds,
                &mut last_progress_percent,
                &emit_progress_seconds,
            );
        }

        Ok(collected_lines)
    }

    fn extract_verbose_transcript_section(lines: &[String]) -> Option<String> {
        let marker_index = lines
            .iter()
            .rposition(|line| line.trim_start().starts_with("Transcription of "))?;

        let mut transcript_lines = Vec::<String>::new();
        for line in lines.iter().skip(marker_index + 1) {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            if trimmed.starts_with("Processing transcription result for:")
                || trimmed.starts_with("Transcription of ")
            {
                break;
            }

            transcript_lines.push(trimmed.to_string());
        }

        let transcript = transcript_lines.join("\n").trim().to_string();
        if transcript.is_empty() {
            None
        } else {
            Some(transcript)
        }
    }

    fn fallback_non_empty_join(lines: &[String]) -> Option<String> {
        let joined = lines
            .iter()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");

        let transcript = joined.trim().to_string();
        if transcript.is_empty() {
            None
        } else {
            Some(transcript)
        }
    }

    fn apply_whisperkit_options(
        command: &mut Command,
        language_code: &str,
        options: &WhisperOptions,
    ) {
        command
            .arg("--task")
            .arg(if options.translate_to_english {
                "translate"
            } else {
                "transcribe"
            })
            // Use --key=value form for numeric args so negative values are parsed correctly.
            .arg(format!(
                "--temperature={}",
                Self::format_cli_float(options.temperature, 0.0)
            ))
            .arg(format!(
                "--temperature-increment-on-fallback={}",
                Self::format_cli_float(options.temperature_increment_on_fallback, 0.2)
            ))
            .arg(format!(
                "--temperature-fallback-count={}",
                options.temperature_fallback_count
            ))
            .arg(format!("--best-of={}", options.best_of))
            .arg(format!(
                "--logprob-threshold={}",
                Self::format_cli_float(options.logprob_threshold, -1.0)
            ))
            .arg(format!(
                "--first-token-log-prob-threshold={}",
                Self::format_cli_float(options.first_token_logprob_threshold, -1.5)
            ))
            .arg(format!(
                "--no-speech-threshold={}",
                Self::format_cli_float(options.no_speech_threshold, 0.6)
            ))
            .arg(format!(
                "--concurrent-worker-count={}",
                options.concurrent_worker_count
            ))
            .arg("--chunking-strategy")
            .arg(Self::map_chunking_strategy(&options.chunking_strategy))
            .arg("--audio-encoder-compute-units")
            .arg(Self::map_compute_units(
                &options.audio_encoder_compute_units,
            ))
            .arg("--text-decoder-compute-units")
            .arg(Self::map_compute_units(&options.text_decoder_compute_units));

        if !language_code.trim().is_empty() && language_code != "auto" {
            command.arg("--language").arg(language_code);
        }

        if options.use_prefill_prompt {
            command.arg("--use-prefill-prompt");
        }
        if options.use_prefill_cache {
            command.arg("--use-prefill-cache");
        }
        if options.without_timestamps {
            command.arg("--without-timestamps");
        }
        if options.word_timestamps {
            command.arg("--word-timestamps");
        }
        if let Some(prompt) = options
            .prompt
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            command.arg("--prompt").arg(prompt);
        }
    }

    async fn transcribe_with_server(
        &self,
        input_wav: &Path,
        model_filename: &str,
        language_code: &str,
        options: &WhisperOptions,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        let model_name = Self::resolve_model_name(model_filename)?;
        tokio::fs::create_dir_all(&self.models_dir)
            .await
            .map_err(|e| {
                ApplicationError::SpeechToText(format!(
                    "failed to ensure WhisperKit models directory '{}': {e}",
                    self.models_dir
                ))
            })?;

        let total_audio_seconds = Self::wav_duration_seconds(input_wav);
        let port = Self::reserve_ephemeral_port()?;
        let host = "127.0.0.1";
        let base_url = format!("http://{host}:{port}");

        let mut command = Command::new(&self.binary_path);
        command
            .kill_on_drop(true)
            .arg("serve")
            .arg("--model")
            .arg(model_name)
            .arg("--download-model-path")
            .arg(&self.models_dir)
            .arg("--download-tokenizer-path")
            .arg(&self.models_dir)
            .arg("--host")
            .arg(host)
            .arg("--port")
            .arg(port.to_string())
            .arg("--verbose")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        Self::apply_whisperkit_options(&mut command, language_code, options);

        let mut server = command.spawn().map_err(|e| {
            ApplicationError::SpeechToText(format!(
                "whisperkit-cli server failed to start at '{}': {e}. Configure WhisperKit CLI path in Settings > Local Models.",
                self.binary_path
            ))
        })?;

        let stdout = server.stdout.take().ok_or_else(|| {
            ApplicationError::SpeechToText("missing whisperkit-cli server stdout pipe".to_string())
        })?;
        let stderr = server.stderr.take().ok_or_else(|| {
            ApplicationError::SpeechToText("missing whisperkit-cli server stderr pipe".to_string())
        })?;

        let stdout_progress = emit_progress_seconds.clone();
        let server_activity_at_ms = Arc::new(AtomicU64::new(Self::clock_now_millis()));
        let stdout_last_activity = server_activity_at_ms.clone();
        let stdout_task = tokio::spawn(async move {
            Self::consume_stream(
                stdout,
                true,
                total_audio_seconds,
                stdout_progress,
                stdout_last_activity,
            )
            .await
        });

        let stderr_progress = emit_progress_seconds.clone();
        let stderr_last_activity = server_activity_at_ms.clone();
        let stderr_task = tokio::spawn(async move {
            Self::consume_stream(stderr, false, None, stderr_progress, stderr_last_activity).await
        });

        let saw_real_progress = Arc::new(AtomicBool::new(false));
        let synthetic_progress_stop = Arc::new(AtomicBool::new(false));
        let synthetic_progress_task = total_audio_seconds.map(|total| {
            let emit = emit_progress_seconds.clone();
            let saw_real_progress = saw_real_progress.clone();
            let synthetic_progress_stop = synthetic_progress_stop.clone();
            tokio::spawn(async move {
                let started_at = Instant::now();
                // Conservative ETA: slightly faster than realtime but never too aggressive.
                let estimated_runtime = (total * 0.85).max(12.0);

                loop {
                    if synthetic_progress_stop.load(Ordering::Relaxed) {
                        break;
                    }

                    tokio::time::sleep(Duration::from_millis(500)).await;
                    if synthetic_progress_stop.load(Ordering::Relaxed) {
                        break;
                    }

                    if saw_real_progress.load(Ordering::Relaxed) {
                        continue;
                    }

                    let elapsed = started_at.elapsed().as_secs_f32();
                    let ratio = (elapsed / estimated_runtime).clamp(0.0, 0.97);
                    emit((total * ratio).clamp(0.0, total));
                }
            })
        });

        let result = async {
            let client = reqwest::Client::builder().build().map_err(|e| {
                ApplicationError::SpeechToText(format!(
                    "failed to initialize WhisperKit streaming client: {e}"
                ))
            })?;

            let mut ready = false;
            for _ in 0..SERVER_STARTUP_ATTEMPTS {
                if let Some(status) = server.try_wait().map_err(|e| {
                    ApplicationError::SpeechToText(format!(
                        "failed while checking whisperkit-cli server state: {e}"
                    ))
                })? {
                    if !status.success() {
                        return Err(ApplicationError::SpeechToText(
                            "whisperkit-cli server exited before becoming ready".to_string(),
                        ));
                    }
                }

                match timeout(
                    Duration::from_secs(1),
                    tokio::net::TcpStream::connect((host, port)),
                )
                .await
                {
                    Ok(Ok(_)) => {
                        ready = true;
                        break;
                    }
                    Ok(Err(_)) | Err(_) => tokio::time::sleep(SERVER_STARTUP_DELAY).await,
                }
            }

            if !ready {
                return Err(ApplicationError::SpeechToText(
                    "whisperkit-cli server did not become ready in time".to_string(),
                ));
            }

            let audio_bytes = tokio::fs::read(input_wav).await.map_err(|e| {
                ApplicationError::SpeechToText(format!(
                    "failed to read temporary wav for WhisperKit streaming: {e}"
                ))
            })?;

            let mut form = multipart::Form::new()
                .part(
                    "file",
                    multipart::Part::bytes(audio_bytes)
                        .file_name("audio.wav")
                        .mime_str("audio/wav")
                        .map_err(|e| {
                            ApplicationError::SpeechToText(format!(
                                "failed to prepare WhisperKit upload part: {e}"
                            ))
                        })?,
                )
                .text("model", model_name.to_string())
                .text("stream", "true".to_string());

            if !language_code.trim().is_empty() && language_code != "auto" {
                form = form.text("language", language_code.to_string());
            }
            if options.translate_to_english {
                form = form.text("task", "translate".to_string());
            }

            let response_timeout = Self::transcription_idle_timeout(total_audio_seconds);
            let response = timeout(
                response_timeout,
                client
                    .post(format!("{base_url}/v1/audio/transcriptions"))
                    .multipart(form)
                    .send(),
            )
            .await
            .map_err(|_| {
                ApplicationError::SpeechToText(format!(
                    "whisperkit-cli streaming request did not start within {}s",
                    response_timeout.as_secs()
                ))
            })?
            .map_err(|e| {
                ApplicationError::SpeechToText(format!(
                    "failed to call whisperkit-cli streaming endpoint: {e}"
                ))
            })?;

            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                let details = body.trim();
                return Err(ApplicationError::SpeechToText(format!(
                    "whisperkit-cli streaming endpoint failed ({}): {}",
                    status,
                    if details.is_empty() {
                        "empty response body".to_string()
                    } else {
                        details.to_string()
                    }
                )));
            }

            let mut transcript_snapshot = String::new();
            let mut final_text: Option<String> = None;
            let mut pending = String::new();
            let mut stream = response.bytes_stream();
            let mut done = false;

            while !done {
                let Some(next_chunk) = stream.next().await else {
                    break;
                };
                let chunk = next_chunk.map_err(|e| {
                    ApplicationError::SpeechToText(format!(
                        "failed to read whisperkit-cli streaming response: {e}"
                    ))
                })?;
                pending.push_str(&String::from_utf8_lossy(&chunk));

                while let Some((event_end, delimiter_len)) = Self::sse_event_boundary(&pending) {
                    let raw_event = pending[..event_end].to_string();
                    pending.drain(..event_end + delimiter_len);

                    let Some(payload) = Self::extract_sse_payload(&raw_event) else {
                        continue;
                    };

                    if payload.trim() == "[DONE]" {
                        done = true;
                        break;
                    }

                    let parsed = serde_json::from_str::<WhisperKitSseEvent>(&payload)
                        .unwrap_or_else(|_| WhisperKitSseEvent {
                            text: payload,
                            ..WhisperKitSseEvent::default()
                        });

                    if let Some(progress) = parsed.progress {
                        if let Some(total) = total_audio_seconds {
                            saw_real_progress.store(true, Ordering::Relaxed);
                            emit_progress_seconds(
                                (progress.clamp(0.0, 1.0) * total).clamp(0.0, total),
                            );
                        }
                    } else if let (Some(current), Some(total)) =
                        (parsed.current_time, parsed.total_time)
                    {
                        if total > 0.0 {
                            saw_real_progress.store(true, Ordering::Relaxed);
                            emit_progress_seconds(current.clamp(0.0, total));
                        }
                    }

                    match parsed.r#type.as_str() {
                        "transcript.text.delta" => {
                            if !parsed.delta.is_empty() {
                                transcript_snapshot.push_str(&parsed.delta);
                            } else if !parsed.text.is_empty() {
                                transcript_snapshot = parsed.text;
                            }

                            let normalized = transcript_snapshot.trim().to_string();
                            if !normalized.is_empty() {
                                emit_partial(format!("{DELTA_REPLACE_PREFIX}{normalized}"));
                            }
                        }
                        "transcript.text.done" => {
                            let candidate = if !parsed.text.trim().is_empty() {
                                parsed.text.trim().to_string()
                            } else {
                                transcript_snapshot.trim().to_string()
                            };
                            if !candidate.is_empty() {
                                emit_partial(format!("{DELTA_REPLACE_PREFIX}{candidate}"));
                                final_text = Some(candidate);
                            }
                        }
                        _ => {
                            if !parsed.text.trim().is_empty() {
                                transcript_snapshot = parsed.text.trim().to_string();
                                emit_partial(format!(
                                    "{DELTA_REPLACE_PREFIX}{}",
                                    transcript_snapshot
                                ));
                            }
                        }
                    }
                }
            }

            final_text
                .or_else(|| {
                    let fallback = transcript_snapshot.trim().to_string();
                    if fallback.is_empty() {
                        None
                    } else {
                        Some(fallback)
                    }
                })
                .ok_or_else(|| {
                    ApplicationError::SpeechToText(
                        "whisperkit-cli streaming produced empty output".to_string(),
                    )
                })
        }
        .await;

        synthetic_progress_stop.store(true, Ordering::Relaxed);
        if let Some(task) = synthetic_progress_task {
            let _ = task.await;
        }

        let _ = server.start_kill();
        let _ = server.wait().await;

        let stdout_lines = stdout_task.await.map_err(|e| {
            ApplicationError::SpeechToText(format!("server stdout reader task failed: {e}"))
        })??;
        let stderr_lines = stderr_task.await.map_err(|e| {
            ApplicationError::SpeechToText(format!("server stderr reader task failed: {e}"))
        })??;

        match result {
            Ok(transcript) => Ok(Self::output_from_text(transcript, total_audio_seconds)),
            Err(error) => {
                let stderr_output = stderr_lines.join("\n").trim().to_string();
                let stdout_output = stdout_lines.join("\n").trim().to_string();
                let details = if !stderr_output.is_empty() {
                    stderr_output
                } else {
                    stdout_output
                };
                if details.is_empty() {
                    Err(error)
                } else {
                    Err(ApplicationError::SpeechToText(format!(
                        "{error}; {details}"
                    )))
                }
            }
        }
    }

    async fn transcribe_with_cli(
        &self,
        input_wav: &Path,
        model_filename: &str,
        language_code: &str,
        options: &WhisperOptions,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        let model_name = Self::resolve_model_name(model_filename)?;
        tokio::fs::create_dir_all(&self.models_dir)
            .await
            .map_err(|e| {
                ApplicationError::SpeechToText(format!(
                    "failed to ensure WhisperKit models directory '{}': {e}",
                    self.models_dir
                ))
            })?;

        let total_audio_seconds = Self::wav_duration_seconds(input_wav);
        let mut command = Command::new(&self.binary_path);
        command
            .kill_on_drop(true)
            .arg("transcribe")
            .arg("--audio-path")
            .arg(input_wav)
            .arg("--model")
            .arg(model_name)
            .arg("--download-model-path")
            .arg(&self.models_dir)
            .arg("--download-tokenizer-path")
            .arg(&self.models_dir)
            .arg("--verbose")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        Self::apply_whisperkit_options(&mut command, language_code, options);

        let mut child = command.spawn().map_err(|e| {
            ApplicationError::SpeechToText(format!(
                "whisperkit-cli failed to start at '{}': {e}. Configure WhisperKit CLI path in Settings > Local Models.",
                self.binary_path
            ))
        })?;

        let stdout = child.stdout.take().ok_or_else(|| {
            ApplicationError::SpeechToText("missing whisperkit-cli stdout pipe".to_string())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            ApplicationError::SpeechToText("missing whisperkit-cli stderr pipe".to_string())
        })?;
        let last_activity_at_ms = Arc::new(AtomicU64::new(Self::clock_now_millis()));

        let stdout_progress = emit_progress_seconds.clone();
        let stdout_last_activity = last_activity_at_ms.clone();
        let stdout_task = tokio::spawn(async move {
            Self::consume_stream(
                stdout,
                true,
                total_audio_seconds,
                stdout_progress,
                stdout_last_activity,
            )
            .await
        });

        let stderr_progress = emit_progress_seconds.clone();
        let stderr_last_activity = last_activity_at_ms.clone();
        let stderr_task = tokio::spawn(async move {
            Self::consume_stream(stderr, false, None, stderr_progress, stderr_last_activity).await
        });

        let status = Self::wait_for_child_with_idle_timeout(
            &mut child,
            "whisperkit-cli",
            total_audio_seconds,
            last_activity_at_ms,
        )
        .await?;

        let stdout_lines = stdout_task.await.map_err(|e| {
            ApplicationError::SpeechToText(format!("stdout reader task failed: {e}"))
        })??;
        let stderr_lines = stderr_task.await.map_err(|e| {
            ApplicationError::SpeechToText(format!("stderr reader task failed: {e}"))
        })??;

        if !status.success() {
            let stderr_output = stderr_lines.join("\n").trim().to_string();
            let stdout_output = stdout_lines.join("\n").trim().to_string();
            let details = if !stderr_output.is_empty() {
                stderr_output
            } else {
                stdout_output
            };

            return Err(ApplicationError::SpeechToText(format!(
                "whisperkit-cli failed: {}",
                details
            )));
        }

        let transcript = Self::extract_verbose_transcript_section(&stdout_lines)
            .or_else(|| Self::fallback_non_empty_join(&stdout_lines))
            .or_else(|| Self::fallback_non_empty_join(&stderr_lines))
            .ok_or_else(|| {
                ApplicationError::SpeechToText("whisperkit-cli produced empty output".to_string())
            })?;
        let transcript = minimize_transcript_repetitions(&transcript);

        for line in transcript.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                emit_partial(trimmed.to_string());
            }
        }

        Ok(Self::output_from_text(transcript, total_audio_seconds))
    }
}

#[async_trait]
impl SpeechToTextEngine for WhisperKitEngine {
    async fn transcribe(
        &self,
        input_wav: &Path,
        model_filename: &str,
        language_code: &str,
        options: &WhisperOptions,
        _total_audio_seconds: Option<f32>,
        emit_partial: Arc<dyn Fn(String) + Send + Sync>,
        emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        match self
            .transcribe_with_server(
                input_wav,
                model_filename,
                language_code,
                options,
                emit_partial.clone(),
                emit_progress_seconds.clone(),
            )
            .await
        {
            Ok(transcript) => Ok(transcript),
            Err(server_error) => {
                warn!("WhisperKit streaming mode failed, falling back to CLI mode: {server_error}");
                self.transcribe_with_cli(
                    input_wav,
                    model_filename,
                    language_code,
                    options,
                    emit_partial,
                    emit_progress_seconds,
                )
                .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{WhisperKitEngine, PROCESS_IDLE_TIMEOUT_MAX, PROCESS_IDLE_TIMEOUT_MIN};

    #[test]
    fn transcription_idle_timeout_defaults_to_minimum_without_duration() {
        assert_eq!(
            WhisperKitEngine::transcription_idle_timeout(None),
            PROCESS_IDLE_TIMEOUT_MIN
        );
    }

    #[test]
    fn transcription_idle_timeout_scales_for_longer_audio() {
        assert_eq!(
            WhisperKitEngine::transcription_idle_timeout(Some(7_200.0)).as_secs(),
            2_100
        );
    }

    #[test]
    fn transcription_idle_timeout_caps_at_maximum() {
        assert_eq!(
            WhisperKitEngine::transcription_idle_timeout(Some(24_000.0)),
            PROCESS_IDLE_TIMEOUT_MAX
        );
    }
}
