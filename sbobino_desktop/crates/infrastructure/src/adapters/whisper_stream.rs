use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncRead, AsyncReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::{timeout, Duration};

use sbobino_application::{ApplicationError, RealtimeDelta, RealtimeDeltaKind};

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

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
}

#[derive(Debug, Clone)]
pub struct WhisperStreamStopResult {
    pub transcript: String,
    pub saved_audio_path: Option<PathBuf>,
}

#[derive(Clone)]
pub struct WhisperStreamEngine {
    binary_path: String,
    models_dir: String,
    state: Arc<Mutex<StreamState>>,
}

impl WhisperStreamEngine {
    pub fn new(binary_path: String, models_dir: String) -> Self {
        Self {
            binary_path,
            models_dir,
            state: Arc::new(Mutex::new(StreamState::default())),
        }
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
        std::process::Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
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

    fn should_store_diagnostic(text: &str) -> bool {
        let lower = text.to_ascii_lowercase();
        lower.contains("failed")
            || lower.contains("error")
            || lower.contains("capture device")
            || lower.contains("audio device")
            || lower.contains("microphone")
    }

    fn is_fatal_startup_diagnostic(text: &str) -> bool {
        let lower = text.to_ascii_lowercase();
        lower.contains("audio.init() failed")
            || lower.contains("found 0 capture devices")
            || lower.contains("couldn't open an audio device")
            || lower.contains("cannot open audio device")
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
        let _ = Self::commit_line(state, preview);
    }

    fn spawn_reader_task<R>(
        shared_state: Arc<Mutex<StreamState>>,
        reader: R,
        emit_delta: Arc<dyn Fn(RealtimeDelta) + Send + Sync>,
    ) -> JoinHandle<()>
    where
        R: AsyncRead + Unpin + Send + 'static,
    {
        tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            let mut pending = Vec::<u8>::new();
            let mut buffer = [0_u8; 2048];

            let process_record =
                |raw_line: String,
                 shared_state: Arc<Mutex<StreamState>>,
                 emit_delta: Arc<dyn Fn(RealtimeDelta) + Send + Sync>| async move {
                    let is_preview = raw_line.contains("[2K]") || raw_line.contains("\u{001b}[2K");
                    let cleaned = Self::clean_line(&raw_line);
                    if cleaned.is_empty() {
                        return;
                    }

                    if Self::should_skip_line(&cleaned) {
                        if Self::should_store_diagnostic(&cleaned) {
                            let mut state = shared_state.lock().await;
                            state.diagnostics.push(cleaned);
                            if state.lines.is_empty()
                                && state.preview.trim().is_empty()
                                && state.running
                                && Self::is_fatal_startup_diagnostic(
                                    state
                                        .diagnostics
                                        .last()
                                        .map(String::as_str)
                                        .unwrap_or_default(),
                                )
                            {
                                state.running = false;
                                state.paused = false;
                            }
                        }
                        return;
                    }

                    let mut state = shared_state.lock().await;
                    if state.paused {
                        return;
                    }

                    if is_preview {
                        state.preview = cleaned.clone();
                        emit_delta(RealtimeDelta {
                            kind: RealtimeDeltaKind::UpdatePreview,
                            text: cleaned,
                        });
                        return;
                    }

                    if let Some(kind) = Self::commit_line(&mut state, cleaned.clone()) {
                        emit_delta(RealtimeDelta {
                            kind,
                            text: cleaned,
                        });
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
                                process_record(raw_line, shared_state.clone(), emit_delta.clone())
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
                process_record(raw_line, shared_state.clone(), emit_delta.clone()).await;
            }

            let mut state = shared_state.lock().await;
            state.active_readers = state.active_readers.saturating_sub(1);
            if state.active_readers == 0 {
                state.running = false;
                state.paused = false;
            }
        })
    }

    pub async fn start(
        &self,
        model_filename: &str,
        language_code: &str,
        emit_delta: Arc<dyn Fn(RealtimeDelta) + Send + Sync>,
    ) -> Result<(), ApplicationError> {
        let mut state = self.state.lock().await;
        if state.running {
            return Err(ApplicationError::Validation(
                "realtime transcription is already running".to_string(),
            ));
        }

        state.diagnostics.clear();

        let model_path = self.model_path(model_filename);
        if !model_path.exists() {
            return Err(ApplicationError::SpeechToText(format!(
                "realtime model file not found at {}",
                model_path.display()
            )));
        }

        let session_dir = Self::create_session_dir()?;
        let mut command = Command::new(&self.binary_path);
        command
            .kill_on_drop(true)
            .arg("-m")
            .arg(&model_path)
            .arg("-t")
            .arg("8")
            .arg("--step")
            .arg("500")
            .arg("--length")
            .arg("5000")
            .arg("--save-audio")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .current_dir(&session_dir);

        if language_code != "auto" {
            command.arg("-l").arg(language_code);
        }

        if let Some(bin_dir) = self.runtime_bin_dir() {
            command.env("PATH", format!("{}:/usr/bin:/bin", bin_dir.display()));
        }

        if let Some(lib_dir) = self.runtime_lib_dir().filter(|path| path.is_dir()) {
            command
                .env("DYLD_LIBRARY_PATH", &lib_dir)
                .env("DYLD_FALLBACK_LIBRARY_PATH", &lib_dir);
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
        drop(state);

        let reader_tasks = vec![
            Self::spawn_reader_task(self.state.clone(), stdout, emit_delta.clone()),
            Self::spawn_reader_task(self.state.clone(), stderr, emit_delta),
        ];

        let mut state = self.state.lock().await;
        state.reader_tasks = reader_tasks;

        Ok(())
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
        let (mut child, reader_tasks) = {
            let mut state = self.state.lock().await;
            (state.child.take(), std::mem::take(&mut state.reader_tasks))
        };

        if let Some(child) = &mut child {
            if let Some(pid) = child.id() {
                let _ = std::process::Command::new("kill")
                    .arg("-INT")
                    .arg(pid.to_string())
                    .status();
            }

            if timeout(Duration::from_millis(900), child.wait())
                .await
                .is_err()
            {
                if let Some(pid) = child.id() {
                    let _ = std::process::Command::new("kill")
                        .arg("-TERM")
                        .arg(pid.to_string())
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
            if timeout(Duration::from_millis(800), &mut task)
                .await
                .is_err()
            {
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
        drop(state);
        let saved_audio_path = Self::await_saved_audio_path(session_dir.as_deref()).await;

        Ok(WhisperStreamStopResult {
            transcript: consolidated,
            saved_audio_path,
        })
    }

    pub async fn is_running(&self) -> bool {
        let mut state = self.state.lock().await;
        if let Some(child) = state.child.as_mut() {
            if child.try_wait().ok().flatten().is_some() {
                state.running = false;
                state.paused = false;
            } else if let Some(pid) = child.id() {
                if !Self::process_is_alive(pid) {
                    state.running = false;
                    state.paused = false;
                }
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
        state.session_dir = None;
    }
}
