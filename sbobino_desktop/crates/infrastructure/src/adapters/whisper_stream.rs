use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncRead, AsyncReadExt, BufReader};
use tokio::process::Child;
use tokio::sync::{oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

use sbobino_application::{ApplicationError, RealtimeDelta, RealtimeDeltaKind};
use sbobino_domain::{LanguageCode, TimedSegment, TranscriptionComputeDevice};

#[cfg(target_os = "windows")]
use crate::background_process::std_background_command;
use crate::background_process::tokio_background_command;

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);
const LIVE_STARTUP_TIMEOUT: Duration = Duration::from_secs(60);

type StartupSignal = Arc<Mutex<Option<oneshot::Sender<Result<(), String>>>>>;

#[derive(Default)]
struct StreamState {
    child: Option<Child>,
    reader_tasks: Vec<JoinHandle<()>>,
    active_readers: usize,
    lines: Vec<String>,
    preview: String,
    diagnostics: Vec<String>,
    paused: bool,
    running: bool,
    session_dir: Option<PathBuf>,
    segments: Vec<TimedSegment>,
    language_detections: Vec<(String, f32)>,
    session_started_at: Option<Instant>,
    last_segment_end_seconds: f32,
    terminal_error: Option<String>,
    stop_requested: bool,
    telemetry_sink: Option<WhisperTelemetrySink>,
    first_preview_ms: Option<f32>,
    captured_seconds: f32,
    processed_seconds: f32,
    backlog_seconds: f32,
    last_inference_ms: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct WhisperStreamStopResult {
    pub transcript: String,
    pub segments: Vec<TimedSegment>,
    pub saved_audio_path: Option<PathBuf>,
}

/// Runtime-only live metrics emitted alongside Whisper preview deltas.  The
/// timestamps are derived from the session clock and parsed segment arrival,
/// so consumers can diagnose decoder lag without inspecting transcript text.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WhisperStreamTelemetry {
    pub captured_seconds: f32,
    pub processed_seconds: f32,
    pub backlog_seconds: f32,
    pub inference_ms: Option<f32>,
    pub first_preview_ms: Option<f32>,
    pub finalization_ms: Option<f32>,
}

pub type WhisperTelemetrySink = Arc<dyn Fn(WhisperStreamTelemetry) + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct WhisperLiveProfile {
    step_ms: u32,
    length_ms: u32,
}

impl WhisperLiveProfile {
    /// Keep the live window short enough for a responsive preview while
    /// limiting encoder work on the packaged live model. A one-second
    /// accelerated cadence maximizes the preflight throughput/latency budget
    /// while keeping step + inference within the two-second preview contract.
    fn for_model(model_filename: &str, device: TranscriptionComputeDevice) -> Self {
        let lower = model_filename.to_ascii_lowercase();
        let large_model = lower.contains("large") || lower.contains("medium");
        match device {
            TranscriptionComputeDevice::Cpu => Self {
                step_ms: 1_280,
                length_ms: if large_model { 4_800 } else { 2_000 },
            },
            TranscriptionComputeDevice::Gpu | TranscriptionComputeDevice::Auto => Self {
                step_ms: 1_000,
                length_ms: if large_model { 4_800 } else { 2_000 },
            },
        }
    }
}

#[derive(Clone)]
pub struct WhisperStreamEngine {
    binary_path: String,
    models_dir: String,
    compute_device: TranscriptionComputeDevice,
    state: Arc<Mutex<StreamState>>,
}

impl WhisperStreamEngine {
    pub fn new(binary_path: String, models_dir: String) -> Self {
        Self {
            binary_path,
            models_dir,
            compute_device: TranscriptionComputeDevice::Auto,
            state: Arc::new(Mutex::new(StreamState::default())),
        }
    }

    pub fn with_compute_device(mut self, compute_device: TranscriptionComputeDevice) -> Self {
        self.compute_device = compute_device;
        self
    }

    fn compute_device_args(device: TranscriptionComputeDevice) -> &'static [&'static str] {
        match device {
            TranscriptionComputeDevice::Cpu => &["-ng", "-nfa"],
            TranscriptionComputeDevice::Auto | TranscriptionComputeDevice::Gpu => &[],
        }
    }

    fn bounded_thread_count(available: usize) -> usize {
        available.clamp(1, 8)
    }

    fn live_thread_count() -> usize {
        let available = std::thread::available_parallelism()
            .map(std::num::NonZeroUsize::get)
            .unwrap_or(1);
        Self::bounded_thread_count(available)
    }

    fn child_exit_diagnostic(status: Option<&ExitStatus>) -> String {
        match status.and_then(ExitStatus::code) {
            Some(code) => format!("Whisper live worker exited unexpectedly with status {code}."),
            None => "Whisper live worker exited unexpectedly (signal/unknown status).".to_string(),
        }
    }

    fn mark_terminal_child_failure(state: &mut StreamState, status: Option<&ExitStatus>) {
        if status.is_some_and(|status| status.code() == Some(0)) {
            // A clean worker completion is a valid terminal state. The caller
            // may still stop and collect the complete transcript/history.
            state.running = false;
            state.paused = false;
            return;
        }
        if state.terminal_error.is_some() {
            return;
        }
        let message = Self::child_exit_diagnostic(status);
        state.terminal_error = Some(message.clone());
        state.diagnostics.push(message);
        state.running = false;
        state.paused = false;
    }

    fn create_session_dir() -> Result<PathBuf, ApplicationError> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        let process_id = std::process::id();
        let counter = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);

        for attempt in 0..16_u8 {
            let session_dir = std::env::temp_dir().join(format!(
                "sbobino-live-{timestamp}-{process_id}-{counter}-{attempt}"
            ));
            match fs::create_dir(&session_dir) {
                Ok(()) => return Ok(session_dir),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    return Err(ApplicationError::SpeechToText(format!(
                        "failed to create realtime audio session directory at {}: {error}",
                        session_dir.display()
                    )));
                }
            }
        }

        Err(ApplicationError::SpeechToText(
            "failed to create a unique realtime audio session directory".to_string(),
        ))
    }

    fn find_saved_audio_path(session_dir: &Path) -> Option<PathBuf> {
        const AUDIO_EXTENSIONS: [&str; 8] =
            ["wav", "m4a", "mp3", "ogg", "opus", "webm", "flac", "aac"];

        let mut candidates = fs::read_dir(session_dir)
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .filter(|path| {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .map(|extension| {
                        AUDIO_EXTENSIONS
                            .iter()
                            .any(|candidate| extension.eq_ignore_ascii_case(candidate))
                    })
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();

        candidates.sort_by_key(|path| {
            fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok()
        });
        candidates.pop()
    }

    async fn await_saved_audio_path(session_dir: Option<&Path>) -> Option<PathBuf> {
        let session_dir = session_dir?;

        for attempt in 0..10 {
            if let Some(path) = Self::find_saved_audio_path(session_dir) {
                return Some(path);
            }
            if attempt < 9 {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }

        None
    }

    fn process_is_alive(pid: u32) -> bool {
        #[cfg(unix)]
        {
            std::process::Command::new("kill")
                .arg("-0")
                .arg(pid.to_string())
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            true
        }
    }

    fn model_path(&self, model_filename: &str) -> PathBuf {
        Path::new(&self.models_dir).join(model_filename)
    }

    fn runtime_bin_dir(&self) -> Option<PathBuf> {
        PathBuf::from(&self.binary_path).parent().map(PathBuf::from)
    }

    fn runtime_lib_dir(&self) -> Option<PathBuf> {
        let bin_dir = self.runtime_bin_dir()?;
        if bin_dir.file_name().and_then(|name| name.to_str()) != Some("bin") {
            return None;
        }
        bin_dir.parent().map(|parent| parent.join("lib"))
    }

    fn clean_line(line: &str) -> String {
        let ansi_replaced = line
            .replace("\u{001b}[2K", "")
            .replace("\u{001b}[0m", "")
            .replace("[2K]", "")
            .replace("[BLANK_AUDIO]", "");
        ansi_replaced.trim().to_string()
    }

    fn should_skip_line(text: &str) -> bool {
        const PREFIXES: [&str; 12] = [
            "init:",
            "whisper_init",
            "whisper_context",
            "whisper_model_load:",
            "whisper_backend_init",
            "ggml_metal_init:",
            "ggml_metal_",
            "ggml_backend_",
            "ggml_",
            "main:",
            "whisper_full_with_state:",
            "whisper_print",
        ];

        PREFIXES.iter().any(|prefix| text.starts_with(prefix))
            || matches!(
                text,
                "[Start speaking]" | "[Start speaking...]" | "[BLANK_AUDIO]"
            )
    }

    fn is_start_marker(text: &str) -> bool {
        matches!(text, "[Start speaking]" | "[Start speaking...]")
    }

    async fn complete_startup(
        shared_state: &Arc<Mutex<StreamState>>,
        startup_signal: &StartupSignal,
        result: Result<(), String>,
    ) {
        if result.is_ok() {
            let mut state = shared_state.lock().await;
            state.session_started_at.get_or_insert_with(Instant::now);
        }
        if let Some(sender) = startup_signal.lock().await.take() {
            let _ = sender.send(result);
        }
    }

    fn parse_language_detection(text: &str) -> Option<(String, f32)> {
        let marker = "auto-detected language:";
        let start = text.to_ascii_lowercase().find(marker)? + marker.len();
        let remainder = text[start..].trim();
        let code = remainder.split_whitespace().next()?.trim();
        let code = LanguageCode::try_from_code(code)
            .ok()
            .filter(|language| !language.is_auto() && language.as_code() != "und")?
            .as_code()
            .to_string();
        let probability = remainder
            .split_once("p =")
            .or_else(|| remainder.split_once("p="))
            .and_then(|(_, value)| value.trim().trim_start_matches('(').split(')').next())
            .and_then(|value| value.trim().parse::<f32>().ok())
            .filter(|value| value.is_finite())
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        Some((code, probability))
    }

    fn confirmed_language(state: &StreamState) -> Option<(String, f32)> {
        let last = state
            .language_detections
            .iter()
            .rev()
            .take(2)
            .collect::<Vec<_>>();
        if last.len() < 2 || last[0].0 != last[1].0 {
            return None;
        }
        let mean = (last[0].1 + last[1].1) / 2.0;
        (mean >= 0.60).then(|| (last[0].0.clone(), mean))
    }

    fn elapsed_seconds(state: &StreamState) -> f32 {
        state
            .session_started_at
            .map(|started| started.elapsed().as_secs_f32())
            .unwrap_or(state.last_segment_end_seconds)
    }

    fn telemetry_snapshot(state: &StreamState) -> WhisperStreamTelemetry {
        let fallback_captured = Self::elapsed_seconds(state).max(state.last_segment_end_seconds);
        let has_runtime_cursors = state.captured_seconds > 0.0 || state.processed_seconds > 0.0;
        let captured_seconds = if has_runtime_cursors {
            state.captured_seconds.max(state.processed_seconds)
        } else {
            fallback_captured
        };
        let processed_seconds = if has_runtime_cursors {
            state.processed_seconds.min(captured_seconds)
        } else {
            state.last_segment_end_seconds.min(captured_seconds)
        };
        let backlog_seconds = if has_runtime_cursors {
            state.backlog_seconds.max(0.0)
        } else {
            (captured_seconds - processed_seconds).max(0.0)
        };
        WhisperStreamTelemetry {
            captured_seconds,
            processed_seconds,
            backlog_seconds,
            inference_ms: state.last_inference_ms.or(Some(backlog_seconds * 1_000.0)),
            first_preview_ms: state.first_preview_ms,
            finalization_ms: None,
        }
    }

    fn emit_telemetry(sink: Option<&WhisperTelemetrySink>, telemetry: WhisperStreamTelemetry) {
        if let Some(sink) = sink {
            sink(telemetry);
        }
    }

    fn should_store_diagnostic(text: &str) -> bool {
        let lower = text.to_ascii_lowercase();
        lower.contains("failed")
            || lower.contains("error")
            || lower.contains("capture device")
            || lower.contains("audio device")
            || lower.contains("microphone")
    }

    fn parse_runtime_metric(text: &str) -> Option<WhisperStreamTelemetry> {
        if !text.starts_with("SBOBINO_WHISPER_LIVE_METRIC ") {
            return None;
        }
        let value = |key: &str| {
            text.split_whitespace().find_map(|field| {
                let (name, value) = field.split_once('=')?;
                (name == key)
                    .then(|| value.trim_end_matches(':').parse::<f32>().ok())
                    .flatten()
                    .filter(|value| value.is_finite())
            })
        };
        Some(WhisperStreamTelemetry {
            captured_seconds: value("captured_seconds")?,
            processed_seconds: value("processed_seconds")?,
            backlog_seconds: value("backlog_seconds")?,
            inference_ms: value("inference_ms"),
            first_preview_ms: None,
            finalization_ms: None,
        })
    }

    fn preflight_rejection_message(text: &str) -> Option<String> {
        if !text.starts_with("SBOBINO_WHISPER_LIVE_PREFLIGHT ")
            || !text
                .split_whitespace()
                .any(|field| field == "status=rejected")
        {
            return None;
        }
        Some(
            "This computer cannot keep Whisper live in real time with the selected device. Use Auto/GPU, or record and transcribe the audio as a file."
                .to_string(),
        )
    }

    fn is_fatal_startup_diagnostic(text: &str) -> bool {
        let lower = text.to_ascii_lowercase();
        lower.contains("audio.init() failed")
            || lower.contains("found 0 capture devices")
            || lower.contains("couldn't open an audio device")
            || lower.contains("cannot open audio device")
            || lower.contains("failed to warm up the live transcription backend")
            || lower.contains("failed to benchmark the live transcription backend")
    }

    fn commit_line(state: &mut StreamState, cleaned: String) -> Option<RealtimeDeltaKind> {
        if state.lines.last().is_some_and(|last| last == &cleaned) {
            return None;
        }

        state.lines.push(cleaned);
        state.preview.clear();
        Some(RealtimeDeltaKind::AppendFinal)
    }

    fn flush_preview_into_lines(state: &mut StreamState) {
        if state.preview.trim().is_empty() {
            return;
        }

        let preview = state.preview.trim().to_string();
        if Self::commit_line(state, preview.clone()).is_some() {
            let end = Self::elapsed_seconds(state).max(state.last_segment_end_seconds);
            let start = state.last_segment_end_seconds.min(end);
            let (language_code, language_confidence) = Self::confirmed_language(state)
                .map(|(code, confidence)| (Some(code), Some(confidence)))
                .unwrap_or((None, None));
            if language_code.is_some() {
                state.language_detections.clear();
            }
            state.segments.push(TimedSegment {
                text: preview,
                start_seconds: Some(start),
                end_seconds: Some(end),
                speaker_id: None,
                speaker_label: None,
                language_code,
                language_confidence,
                words: Vec::new(),
            });
            state.last_segment_end_seconds = end;
        }
    }

    fn spawn_reader_task<R>(
        shared_state: Arc<Mutex<StreamState>>,
        reader: R,
        emit_delta: Arc<dyn Fn(RealtimeDelta) + Send + Sync>,
        telemetry_sink: Option<WhisperTelemetrySink>,
        startup_signal: StartupSignal,
    ) -> JoinHandle<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            let mut pending = Vec::<u8>::new();
            let mut buffer = [0_u8; 2048];

            let process_record = |raw_line: String,
                                  shared_state: Arc<Mutex<StreamState>>,
                                  emit_delta: Arc<dyn Fn(RealtimeDelta) + Send + Sync>,
                                  telemetry_sink: Option<WhisperTelemetrySink>,
                                  startup_signal: StartupSignal| async move {
                let is_preview = raw_line.contains("[2K]") || raw_line.contains("\u{001b}[2K");
                let cleaned = Self::clean_line(&raw_line);
                if cleaned.is_empty() {
                    return;
                }

                if Self::is_start_marker(&cleaned) {
                    Self::complete_startup(&shared_state, &startup_signal, Ok(())).await;
                    return;
                }

                if let Some(metric) = Self::parse_runtime_metric(&cleaned) {
                    Self::complete_startup(&shared_state, &startup_signal, Ok(())).await;
                    let mut state = shared_state.lock().await;
                    state.captured_seconds = state.captured_seconds.max(metric.captured_seconds);
                    state.processed_seconds = state.processed_seconds.max(metric.processed_seconds);
                    state.backlog_seconds = metric.backlog_seconds.max(0.0);
                    state.last_inference_ms = metric.inference_ms;
                    let mut metric = metric;
                    metric.first_preview_ms = state.first_preview_ms;
                    Self::emit_telemetry(telemetry_sink.as_ref(), metric);
                    return;
                }

                if cleaned.starts_with("SBOBINO_WHISPER_LIVE_PREFLIGHT ") {
                    let rejection = Self::preflight_rejection_message(&cleaned);
                    let mut state = shared_state.lock().await;
                    state.diagnostics.push(cleaned);
                    if let Some(message) = rejection {
                        state.terminal_error = Some(message.clone());
                        state.running = false;
                        state.paused = false;
                        drop(state);
                        Self::complete_startup(&shared_state, &startup_signal, Err(message)).await;
                    }
                    return;
                }

                if cleaned.starts_with("SBOBINO_WHISPER_LIVE_BACKLOG ") {
                    let mut state = shared_state.lock().await;
                    let message = "Whisper live could not keep up in real time. The complete captured audio was preserved; transcribe it as a file or choose a faster model/device.".to_string();
                    state.diagnostics.push(cleaned);
                    state.terminal_error = Some(message);
                    state.running = false;
                    state.paused = false;
                    state.backlog_seconds = state.backlog_seconds.max(2.001);
                    let telemetry = Self::telemetry_snapshot(&state);
                    drop(state);
                    Self::emit_telemetry(telemetry_sink.as_ref(), telemetry);
                    return;
                }

                if let Some((language, probability)) = Self::parse_language_detection(&cleaned) {
                    let mut state = shared_state.lock().await;
                    state.language_detections.push((language, probability));
                    if state.language_detections.len() > 8 {
                        let excess = state.language_detections.len() - 8;
                        state.language_detections.drain(0..excess);
                    }
                    return;
                }

                if Self::is_fatal_startup_diagnostic(&cleaned) {
                    let mut state = shared_state.lock().await;
                    state.diagnostics.push(cleaned.clone());
                    state.running = false;
                    state.paused = false;
                    state.terminal_error = Some(cleaned.clone());
                    drop(state);
                    Self::complete_startup(&shared_state, &startup_signal, Err(cleaned)).await;
                    return;
                }

                if Self::should_skip_line(&cleaned) {
                    if Self::should_store_diagnostic(&cleaned) {
                        let mut state = shared_state.lock().await;
                        state.diagnostics.push(cleaned);
                    }
                    return;
                }

                Self::complete_startup(&shared_state, &startup_signal, Ok(())).await;

                let mut state = shared_state.lock().await;
                if state.paused {
                    return;
                }

                if is_preview {
                    state.preview = cleaned.clone();
                    if state.first_preview_ms.is_none() {
                        state.first_preview_ms = Some(Self::elapsed_seconds(&state) * 1_000.0);
                    }
                    let telemetry = Self::telemetry_snapshot(&state);
                    emit_delta(RealtimeDelta {
                        kind: RealtimeDeltaKind::UpdatePreview,
                        text: cleaned,
                        start_seconds: None,
                        end_seconds: None,
                        language_code: None,
                        language_confidence: None,
                    });
                    Self::emit_telemetry(telemetry_sink.as_ref(), telemetry);
                    return;
                }

                if let Some(kind) = Self::commit_line(&mut state, cleaned.clone()) {
                    let detected_language = Self::confirmed_language(&state);
                    if detected_language.is_some() {
                        state.language_detections.clear();
                    }
                    let end_seconds =
                        Self::elapsed_seconds(&state).max(state.last_segment_end_seconds + 0.001);
                    let start_seconds = state.last_segment_end_seconds.min(end_seconds);
                    let (language_code, language_confidence) = detected_language
                        .map(|(code, confidence)| (Some(code), Some(confidence)))
                        .unwrap_or((None, None));
                    state.segments.push(TimedSegment {
                        text: cleaned.clone(),
                        start_seconds: Some(start_seconds),
                        end_seconds: Some(end_seconds),
                        speaker_id: None,
                        speaker_label: None,
                        language_code: language_code.clone(),
                        language_confidence,
                        words: Vec::new(),
                    });
                    state.last_segment_end_seconds = end_seconds;
                    let telemetry = Self::telemetry_snapshot(&state);
                    emit_delta(RealtimeDelta {
                        kind,
                        text: cleaned,
                        start_seconds: Some(start_seconds),
                        end_seconds: Some(end_seconds),
                        language_code,
                        language_confidence,
                    });
                    Self::emit_telemetry(telemetry_sink.as_ref(), telemetry);
                }
            };

            loop {
                match reader.read(&mut buffer).await {
                    Ok(0) => break,
                    Ok(read_bytes) => {
                        pending.extend_from_slice(&buffer[..read_bytes]);

                        let mut record_start = 0usize;
                        let mut separators_consumed = 0usize;
                        for (index, byte) in pending.iter().copied().enumerate() {
                            if byte != b'\n' && byte != b'\r' {
                                continue;
                            }

                            if index > record_start {
                                let raw_line =
                                    String::from_utf8_lossy(&pending[record_start..index])
                                        .to_string();
                                process_record(
                                    raw_line,
                                    shared_state.clone(),
                                    emit_delta.clone(),
                                    telemetry_sink.clone(),
                                    startup_signal.clone(),
                                )
                                .await;
                            }

                            record_start = index + 1;
                            separators_consumed = record_start;
                        }

                        if separators_consumed > 0 {
                            pending.drain(0..separators_consumed);
                        }
                    }
                    Err(_) => break,
                }
            }

            if !pending.is_empty() {
                let raw_line = String::from_utf8_lossy(&pending).to_string();
                process_record(
                    raw_line,
                    shared_state.clone(),
                    emit_delta.clone(),
                    telemetry_sink.clone(),
                    startup_signal.clone(),
                )
                .await;
            }

            let mut state = shared_state.lock().await;
            state.active_readers = state.active_readers.saturating_sub(1);
            if state.active_readers == 0 {
                let child_status = state
                    .child
                    .as_mut()
                    .and_then(|child| child.try_wait().ok().flatten());
                if state.running
                    && !state.stop_requested
                    && child_status
                        .as_ref()
                        .is_some_and(|status| status.code() != Some(0))
                {
                    Self::mark_terminal_child_failure(&mut state, child_status.as_ref());
                }
                state.running = false;
                state.paused = false;
                let error = state.terminal_error.clone().unwrap_or_else(|| {
                    "Whisper live exited before microphone capture became ready".to_string()
                });
                drop(state);
                Self::complete_startup(&shared_state, &startup_signal, Err(error)).await;
            }
        })
    }

    pub async fn start(
        &self,
        model_filename: &str,
        language_code: &str,
        emit_delta: Arc<dyn Fn(RealtimeDelta) + Send + Sync>,
    ) -> Result<(), ApplicationError> {
        self.start_with_telemetry(model_filename, language_code, emit_delta, None)
            .await
    }

    pub async fn start_with_telemetry(
        &self,
        model_filename: &str,
        language_code: &str,
        emit_delta: Arc<dyn Fn(RealtimeDelta) + Send + Sync>,
        telemetry_sink: Option<WhisperTelemetrySink>,
    ) -> Result<(), ApplicationError> {
        let mut state = self.state.lock().await;
        if state.running {
            return Err(ApplicationError::Validation(
                "realtime transcription is already running".to_string(),
            ));
        }

        state.diagnostics.clear();
        state.terminal_error = None;
        state.stop_requested = false;

        let model_path = self.model_path(model_filename);
        if !model_path.exists() {
            return Err(ApplicationError::SpeechToText(format!(
                "realtime model file not found at {}",
                model_path.display()
            )));
        }

        let session_dir = Self::create_session_dir()?;
        let mut command = tokio_background_command(&self.binary_path);
        let profile = WhisperLiveProfile::for_model(model_filename, self.compute_device);
        let thread_count = Self::live_thread_count();
        command
            .kill_on_drop(true)
            .arg("-m")
            .arg(&model_path)
            .arg("-t")
            .arg(thread_count.to_string())
            .arg("--step")
            .arg(profile.step_ms.to_string())
            .arg("--length")
            .arg(profile.length_ms.to_string())
            .arg("--no-fallback")
            .arg("--save-audio")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .current_dir(&session_dir);

        let _preferred_language = language_code;
        command.arg("-l").arg("auto");
        for argument in Self::compute_device_args(self.compute_device) {
            command.arg(argument);
        }

        if let Some(bin_dir) = self.runtime_bin_dir() {
            let lib_dir = self.runtime_lib_dir().filter(|path| path.is_dir());
            let mut runtime_paths = vec![bin_dir];
            runtime_paths.extend(lib_dir.iter().cloned());
            if let Some(existing) = std::env::var_os("PATH") {
                runtime_paths.extend(std::env::split_paths(&existing));
            }
            if let Ok(path) = std::env::join_paths(runtime_paths) {
                command.env("PATH", path);
            }
            #[cfg(target_os = "macos")]
            if let Some(lib_dir) = lib_dir {
                command
                    .env("DYLD_LIBRARY_PATH", &lib_dir)
                    .env("DYLD_FALLBACK_LIBRARY_PATH", &lib_dir);
            }
        }

        let mut child = command.spawn().map_err(|e| {
            ApplicationError::SpeechToText(format!(
                "failed to start realtime whisper stream ({}) : {e}",
                self.binary_path
            ))
        })?;

        let stdout = child.stdout.take().ok_or_else(|| {
            ApplicationError::SpeechToText("missing realtime stdout pipe".to_string())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            ApplicationError::SpeechToText("missing realtime stderr pipe".to_string())
        })?;
        state.child = Some(child);
        state.reader_tasks.clear();
        state.active_readers = 2;
        state.running = true;
        state.paused = false;
        state.session_dir = Some(session_dir);
        state.segments.clear();
        state.language_detections.clear();
        state.session_started_at = None;
        state.last_segment_end_seconds = 0.0;
        state.telemetry_sink = telemetry_sink.clone();
        state.first_preview_ms = None;
        state.captured_seconds = 0.0;
        state.processed_seconds = 0.0;
        state.backlog_seconds = 0.0;
        state.last_inference_ms = None;
        drop(state);

        let (startup_sender, startup_receiver) = oneshot::channel();
        let startup_signal = Arc::new(Mutex::new(Some(startup_sender)));
        let reader_tasks = vec![
            Self::spawn_reader_task(
                self.state.clone(),
                stdout,
                emit_delta.clone(),
                telemetry_sink.clone(),
                startup_signal.clone(),
            ),
            Self::spawn_reader_task(
                self.state.clone(),
                stderr,
                emit_delta,
                telemetry_sink,
                startup_signal,
            ),
        ];

        let mut state = self.state.lock().await;
        state.reader_tasks = reader_tasks;
        drop(state);

        let startup_result = timeout(LIVE_STARTUP_TIMEOUT, startup_receiver).await;
        match startup_result {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(error))) => {
                let _ = self.stop().await;
                Err(ApplicationError::SpeechToText(error))
            }
            Ok(Err(_)) => {
                let diagnostics = self.snapshot_diagnostics().await;
                let _ = self.stop().await;
                Err(ApplicationError::SpeechToText(if diagnostics.is_empty() {
                    "Whisper live exited before microphone capture became ready".to_string()
                } else {
                    diagnostics.join(" ")
                }))
            }
            Err(_) => {
                let message = format!(
                    "Whisper live did not become ready within {} seconds",
                    LIVE_STARTUP_TIMEOUT.as_secs()
                );
                {
                    let mut state = self.state.lock().await;
                    state.terminal_error = Some(message.clone());
                }
                let _ = self.stop().await;
                Err(ApplicationError::SpeechToText(message))
            }
        }
    }

    pub async fn pause(&self) -> Result<(), ApplicationError> {
        let mut state = self.state.lock().await;
        if !state.running {
            return Err(ApplicationError::Validation(
                "realtime transcription is not running".to_string(),
            ));
        }
        state.paused = true;
        Ok(())
    }

    pub async fn resume(&self) -> Result<(), ApplicationError> {
        let mut state = self.state.lock().await;
        if !state.running {
            return Err(ApplicationError::Validation(
                "realtime transcription is not running".to_string(),
            ));
        }
        state.paused = false;
        Ok(())
    }

    pub async fn stop(&self) -> Result<WhisperStreamStopResult, ApplicationError> {
        let finalization_started = Instant::now();
        let (mut child, reader_tasks) = {
            let mut state = self.state.lock().await;
            // Observe an already-exited child before taking ownership of it.
            // Once `child` is moved out, the reader tasks cannot reliably
            // distinguish a crash from the intentional stop signal.
            if let Some(child) = state.child.as_mut() {
                if let Ok(Some(status)) = child.try_wait() {
                    Self::mark_terminal_child_failure(&mut state, Some(&status));
                }
            }
            state.stop_requested = true;
            (state.child.take(), std::mem::take(&mut state.reader_tasks))
        };

        if let Some(child) = &mut child {
            if let Some(pid) = child.id() {
                #[cfg(unix)]
                let _ = std::process::Command::new("kill")
                    .arg("-INT")
                    .arg(pid.to_string())
                    .status();
                #[cfg(target_os = "windows")]
                let _ = std_background_command("taskkill")
                    .args(["/PID", &pid.to_string()])
                    .status();
            }

            if timeout(Duration::from_millis(900), child.wait())
                .await
                .is_err()
            {
                if let Some(pid) = child.id() {
                    #[cfg(unix)]
                    let _ = std::process::Command::new("kill")
                        .arg("-TERM")
                        .arg(pid.to_string())
                        .status();
                    #[cfg(target_os = "windows")]
                    let _ = std_background_command("taskkill")
                        .args(["/F", "/PID", &pid.to_string()])
                        .status();
                }
                if timeout(Duration::from_millis(500), child.wait())
                    .await
                    .is_err()
                {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                }
            }
        }

        for mut task in reader_tasks {
            if timeout(Duration::from_secs(3), &mut task).await.is_err() {
                task.abort();
            }
        }

        let mut state = self.state.lock().await;
        state.child = None;
        state.active_readers = 0;
        state.running = false;
        state.paused = false;
        Self::flush_preview_into_lines(&mut state);

        let session_dir = state.session_dir.take();
        let consolidated = state.lines.join("\n");
        let segments = state.segments.clone();
        let mut final_telemetry = Self::telemetry_snapshot(&state);
        let telemetry_sink = state.telemetry_sink.clone();
        let terminal_error = state.terminal_error.clone();
        state.session_started_at = None;
        state.language_detections.clear();
        state.stop_requested = false;
        drop(state);
        let saved_audio_path = Self::await_saved_audio_path(session_dir.as_deref()).await;

        final_telemetry.finalization_ms =
            Some(finalization_started.elapsed().as_secs_f32() * 1_000.0);
        Self::emit_telemetry(telemetry_sink.as_ref(), final_telemetry);

        if let Some(error) = terminal_error {
            let recovery = match saved_audio_path.as_ref() {
                Some(path) => format!(
                    "{error} The captured audio was preserved at {}. Transcribe that WAV as a file to recover the session; no partial live transcript was saved.",
                    path.display()
                ),
                None => format!(
                    "{error} No captured audio file was found, so no partial live transcript was saved. Retry the live session after checking the microphone and runtime."
                ),
            };
            return Err(ApplicationError::SpeechToText(recovery));
        }

        Ok(WhisperStreamStopResult {
            transcript: consolidated,
            segments,
            saved_audio_path,
        })
    }

    pub async fn is_running(&self) -> bool {
        let mut state = self.state.lock().await;
        let child_status = state
            .child
            .as_mut()
            .and_then(|child| child.try_wait().ok().flatten());
        if let Some(status) = child_status.as_ref() {
            if status.code() == Some(0) {
                state.running = false;
                state.paused = false;
            } else {
                Self::mark_terminal_child_failure(&mut state, Some(status));
            }
        } else if let Some(pid) = state.child.as_ref().and_then(Child::id) {
            if !Self::process_is_alive(pid) {
                Self::mark_terminal_child_failure(&mut state, None);
            }
        }
        state.running
    }

    pub async fn is_paused(&self) -> bool {
        self.state.lock().await.paused
    }

    pub async fn seed_buffer(&self, text: &str) {
        let mut state = self.state.lock().await;
        state.lines = text
            .lines()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .map(ToString::to_string)
            .collect();
        state.preview.clear();
        state.segments.clear();
        state.language_detections.clear();
        state.last_segment_end_seconds = 0.0;
        state.terminal_error = None;
        state.stop_requested = false;
        state.captured_seconds = 0.0;
        state.processed_seconds = 0.0;
        state.backlog_seconds = 0.0;
        state.last_inference_ms = None;
    }

    pub async fn snapshot_text(&self) -> String {
        self.state.lock().await.lines.join("\n")
    }

    pub async fn snapshot_diagnostics(&self) -> Vec<String> {
        self.state.lock().await.diagnostics.clone()
    }

    pub async fn reset(&self) {
        let mut state = self.state.lock().await;
        state.lines.clear();
        state.preview.clear();
        state.diagnostics.clear();
        state.terminal_error = None;
        state.session_dir = None;
        state.segments.clear();
        state.language_detections.clear();
        state.session_started_at = None;
        state.last_segment_end_seconds = 0.0;
        state.captured_seconds = 0.0;
        state.processed_seconds = 0.0;
        state.backlog_seconds = 0.0;
        state.last_inference_ms = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{StreamState, WhisperLiveProfile, WhisperStreamEngine};
    use sbobino_domain::TranscriptionComputeDevice;

    #[test]
    fn cpu_mode_disables_gpu_and_flash_attention_for_live_whisper() {
        assert_eq!(
            WhisperStreamEngine::compute_device_args(TranscriptionComputeDevice::Cpu),
            &["-ng", "-nfa"]
        );
        assert!(
            WhisperStreamEngine::compute_device_args(TranscriptionComputeDevice::Auto).is_empty()
        );
        assert!(
            WhisperStreamEngine::compute_device_args(TranscriptionComputeDevice::Gpu).is_empty()
        );
    }

    #[test]
    fn live_threads_are_bounded_to_available_parallelism() {
        assert_eq!(WhisperStreamEngine::bounded_thread_count(0), 1);
        assert_eq!(WhisperStreamEngine::bounded_thread_count(4), 4);
        assert_eq!(WhisperStreamEngine::bounded_thread_count(32), 8);
    }

    #[test]
    fn adaptive_live_profile_scales_cpu_step_without_reducing_context() {
        let cpu = WhisperLiveProfile::for_model("ggml-base.bin", TranscriptionComputeDevice::Cpu);
        let gpu = WhisperLiveProfile::for_model("ggml-base.bin", TranscriptionComputeDevice::Gpu);
        let large =
            WhisperLiveProfile::for_model("ggml-large-v3.bin", TranscriptionComputeDevice::Auto);
        assert_eq!(cpu.step_ms, 1_280);
        assert_eq!(cpu.length_ms, 2_000);
        assert_eq!(gpu.step_ms, 1_000);
        assert_eq!(cpu.length_ms, gpu.length_ms);
        assert!(cpu.step_ms > gpu.step_ms);
        assert!(large.length_ms > gpu.length_ms);
    }

    #[test]
    fn language_detection_requires_two_agreeing_windows() {
        let mut state = StreamState::default();
        state.language_detections.push(("it".to_string(), 0.7));
        assert!(WhisperStreamEngine::confirmed_language(&state).is_none());
        state.language_detections.push(("it".to_string(), 0.8));
        assert_eq!(
            WhisperStreamEngine::confirmed_language(&state),
            Some(("it".to_string(), 0.75))
        );
        state.language_detections.push(("en".to_string(), 0.9));
        assert!(WhisperStreamEngine::confirmed_language(&state).is_none());
    }

    #[test]
    fn parses_fragmented_whisper_language_log() {
        let parsed = WhisperStreamEngine::parse_language_detection(
            "whisper_full_with_state: auto-detected language: en (p = 0.83)",
        );
        assert_eq!(parsed, Some(("en".to_string(), 0.83)));
    }

    #[test]
    fn parses_packaged_live_runtime_metrics_without_exposing_them_as_text() {
        let metric = WhisperStreamEngine::parse_runtime_metric(
            "SBOBINO_WHISPER_LIVE_METRIC captured_seconds=4.640 processed_seconds=4.320 backlog_seconds=0.320 inference_ms=188.500 dropped_samples=0",
        )
        .expect("packaged runtime metric should parse");
        assert_eq!(metric.captured_seconds, 4.64);
        assert_eq!(metric.processed_seconds, 4.32);
        assert_eq!(metric.backlog_seconds, 0.32);
        assert_eq!(metric.inference_ms, Some(188.5));
    }

    #[test]
    fn rejected_live_preflight_returns_an_actionable_error() {
        let message = WhisperStreamEngine::preflight_rejection_message(
            "SBOBINO_WHISPER_LIVE_PREFLIGHT status=rejected inference_ms=1600.000 budget_ms=720.000 step_ms=1280",
        )
        .expect("rejected preflight should be actionable");
        assert!(message.contains("Auto/GPU"));
        assert!(message.contains("transcribe the audio as a file"));
        assert!(WhisperStreamEngine::preflight_rejection_message(
            "SBOBINO_WHISPER_LIVE_PREFLIGHT status=passed inference_ms=400.000 budget_ms=720.000 step_ms=1280",
        )
        .is_none());
    }

    #[test]
    fn runtime_cursors_are_not_inflated_by_wall_clock_fallback() {
        let state = StreamState {
            captured_seconds: 4.64,
            processed_seconds: 4.32,
            backlog_seconds: 0.32,
            last_segment_end_seconds: 99.0,
            ..StreamState::default()
        };

        let telemetry = WhisperStreamEngine::telemetry_snapshot(&state);
        assert_eq!(telemetry.captured_seconds, 4.64);
        assert_eq!(telemetry.processed_seconds, 4.32);
        assert_eq!(telemetry.backlog_seconds, 0.32);
    }

    #[tokio::test]
    async fn child_exit_after_start_is_terminal_and_visible() {
        let engine = WhisperStreamEngine::new("whisper-stream".to_string(), "models".to_string());
        let child = tokio::process::Command::new("sh")
            .args(["-c", "exit 7"])
            .spawn()
            .expect("fake whisper child should spawn");
        tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;
        {
            let mut state = engine.state.lock().await;
            state.child = Some(child);
            state.running = true;
            state.active_readers = 1;
        }
        let mut running = true;
        for _ in 0..20 {
            tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;
            running = engine.is_running().await;
            if !running {
                break;
            }
        }
        assert!(!running);
        let diagnostics = engine.snapshot_diagnostics().await;
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].contains("status 7"), "{diagnostics:?}");
        assert!(engine
            .state
            .lock()
            .await
            .terminal_error
            .as_deref()
            .is_some_and(|message| message.contains("status 7")));
    }

    #[tokio::test]
    async fn stop_after_child_exit_returns_recovery_error_and_keeps_audio_path() {
        let temp = tempfile::tempdir().expect("temp dir");
        let audio_path = temp.path().join("whisper-live.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&audio_path, spec).expect("create wav");
        writer.write_sample(0_i16).expect("write wav");
        writer.finalize().expect("finalize wav");

        let engine = WhisperStreamEngine::new("whisper-stream".to_string(), "models".to_string());
        let child = tokio::process::Command::new("sh")
            .args(["-c", "exit 7"])
            .spawn()
            .expect("fake whisper child should spawn");
        tokio::time::sleep(tokio::time::Duration::from_millis(25)).await;
        {
            let mut state = engine.state.lock().await;
            state.child = Some(child);
            state.running = true;
            state.active_readers = 0;
            state.session_dir = Some(temp.path().to_path_buf());
            state
                .lines
                .push("partial preview must not be saved".to_string());
        }

        let error = engine
            .stop()
            .await
            .expect_err("an exited Whisper child must fail stop");
        let message = error.to_string();
        assert!(message.contains("status 7"), "{message}");
        assert!(
            message.contains(audio_path.to_string_lossy().as_ref()),
            "{message}"
        );
        assert!(
            message.contains("Transcribe that WAV as a file"),
            "{message}"
        );
        assert!(
            message.contains("no partial live transcript was saved"),
            "{message}"
        );
    }
}
