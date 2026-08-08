use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use lingua::{Language, LanguageDetector, LanguageDetectorBuilder};
use serde::Deserialize;
use tempfile::TempDir;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::process::Command;

use sbobino_application::{ApplicationError, SpeechToTextEngine};
use sbobino_domain::{
    LanguageCode, TimedSegment, TimedWord, TranscriptionLanguagePolicy, TranscriptionOutput,
    WhisperOptions,
};

use crate::adapters::transcript_segmentation::normalize_transcript_segments;
use crate::background_process::tokio_background_command;

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
const LONG_FILE_THRESHOLD_SECONDS: f32 = 10.0 * 60.0;
// Parakeet TDT allocates its graph from the decoded clip rather than from the
// worker batch size. Keep every serialized clip safely below the graph size
// that caused the real 16 GiB Apple Silicon watchdog panic.
const LONG_FILE_INITIAL_COMMIT_WINDOW_SECONDS: f32 = 30.0;
const LONG_FILE_TARGET_COMMIT_WINDOW_SECONDS: f32 = 30.0;
const LONG_FILE_RETRY_COMMIT_WINDOW_SECONDS: [f32; 3] = [20.0, 15.0, 10.0];
const LONG_FILE_MIN_COMMIT_WINDOW_SECONDS: f32 = 10.0;
const LONG_FILE_BOUNDARY_SNAP_SECONDS: f32 = 20.0;
const LONG_FILE_BOUNDARY_RMS_WINDOW_SECONDS: f32 = 0.5;
const LONG_FILE_CONTEXT_SECONDS: f32 = 5.0;
const LONG_FILE_TAIL_PAD_SECONDS: f32 = 2.0;
const LONG_FILE_MAX_SERIALIZED_DECODE_SECONDS: f32 = 45.0;
const LONG_FILE_CHUNK_VALIDATION_TOLERANCE_SECONDS: f32 = 0.01;
const OVERLAP_DEDUPE_TOLERANCE_SECONDS: f32 = 0.05;
const WORKER_RSS_SAMPLE_INTERVAL: Duration = Duration::from_millis(250);
const DEFAULT_WORKER_RSS_LIMIT_BYTES: u64 = 6 * 1024 * 1024 * 1024;
const WORKER_RSS_LIMIT_ENV: &str = "SBOBINO_PARAKEET_WORKER_RSS_LIMIT_BYTES";

#[derive(Debug, Clone)]
pub struct ParakeetCppEngine {
    binary_path: String,
    models_dir: String,
    worker_rss_limit_override_bytes: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ParakeetJsonOutput {
    #[serde(default)]
    text: String,
    #[serde(default)]
    words: Vec<ParakeetJsonWord>,
    #[serde(default)]
    segments: Vec<ParakeetJsonSegment>,
    #[serde(default, alias = "lang", alias = "language_code")]
    language: Option<String>,
    #[serde(default, alias = "language_probability", alias = "probability")]
    language_confidence: Option<f32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ParakeetJsonSegment {
    #[serde(default)]
    text: String,
    #[serde(default)]
    start: Option<f32>,
    #[serde(default)]
    end: Option<f32>,
    #[serde(default)]
    words: Vec<ParakeetJsonWord>,
    #[serde(default, alias = "lang", alias = "language_code")]
    language: Option<String>,
    #[serde(
        default,
        alias = "language_probability",
        alias = "probability",
        alias = "confidence"
    )]
    language_confidence: Option<f32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
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

#[derive(Debug, Clone)]
struct AudioChunk {
    index: usize,
    path: PathBuf,
    decode_start_seconds: f32,
    decode_end_seconds: f32,
    commit_start_seconds: f32,
    commit_end_seconds: f32,
}

struct LongFileCallbacks {
    emit_partial: Arc<dyn Fn(String) + Send + Sync>,
    emit_progress_seconds: Arc<dyn Fn(f32) + Send + Sync>,
}

#[derive(Debug, Clone, Copy)]
struct WorkerMemoryStats {
    worker_pid: u32,
    process_group: Option<i32>,
    peak_rss_bytes: u64,
    limit_bytes: u64,
    #[cfg(windows)]
    job_handle: Option<windows_sys::Win32::Foundation::HANDLE>,
}

#[cfg(windows)]
struct WindowsWorkerJobGuard {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl WindowsWorkerJobGuard {
    fn attach(
        process_handle: windows_sys::Win32::Foundation::HANDLE,
        limit_bytes: u64,
    ) -> Result<Self, String> {
        use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_JOB_MEMORY, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };

        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(format!(
                "CreateJobObjectW failed with Windows error {}",
                unsafe { GetLastError() }
            ));
        }

        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags =
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_JOB_MEMORY;
        limits.JobMemoryLimit = usize::try_from(limit_bytes).map_err(|_| {
            unsafe { CloseHandle(handle) };
            format!("Windows job memory limit does not fit in usize: {limit_bytes}")
        })?;
        let limits_size =
            u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).map_err(
                |_| {
                    unsafe { CloseHandle(handle) };
                    "Windows job limit structure is too large".to_string()
                },
            )?;
        let configured = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                limits_size,
            )
        };
        if configured == 0 {
            let error = unsafe { GetLastError() };
            unsafe { CloseHandle(handle) };
            return Err(format!(
                "SetInformationJobObject failed with Windows error {error}"
            ));
        }

        let assigned = unsafe { AssignProcessToJobObject(handle, process_handle) };
        if assigned == 0 {
            let error = unsafe { GetLastError() };
            // KILL_ON_JOB_CLOSE also handles any process that may have been
            // created while assignment was in flight.
            unsafe {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(handle, 1);
                CloseHandle(handle);
            }
            return Err(format!(
                "AssignProcessToJobObject failed with Windows error {error}"
            ));
        }

        Ok(Self { handle })
    }

    fn peak_memory_bytes_for_handle(
        handle: windows_sys::Win32::Foundation::HANDLE,
    ) -> Result<u64, String> {
        use windows_sys::Win32::Foundation::GetLastError;
        use windows_sys::Win32::System::JobObjects::{
            JobObjectExtendedLimitInformation, QueryInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        };

        let mut information = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        let information_size =
            u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
                .map_err(|_| "Windows job information structure is too large".to_string())?;
        let queried = unsafe {
            QueryInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&mut information as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                information_size,
                std::ptr::null_mut(),
            )
        };
        if queried == 0 {
            return Err(format!(
                "QueryInformationJobObject failed with Windows error {}",
                unsafe { GetLastError() }
            ));
        }
        Ok(information.PeakJobMemoryUsed as u64)
    }

    fn terminate(&self) {
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.handle, 1);
        }
    }
}

#[cfg(windows)]
impl Drop for WindowsWorkerJobGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

struct WorkerProcessGroupGuard {
    process_group: Option<i32>,
    #[cfg(windows)]
    job: Option<WindowsWorkerJobGuard>,
}

impl WorkerProcessGroupGuard {
    #[cfg(windows)]
    fn new(process_group: Option<i32>, job: Option<WindowsWorkerJobGuard>) -> Self {
        Self { process_group, job }
    }

    #[cfg(not(windows))]
    fn new(process_group: Option<i32>) -> Self {
        Self { process_group }
    }

    fn disarm(&mut self) {
        self.process_group = None;
        #[cfg(windows)]
        drop(self.job.take());
    }
}

impl Drop for WorkerProcessGroupGuard {
    fn drop(&mut self) {
        // `kill_on_drop` reaps the direct child. This guard additionally makes
        // cancellation safe when a worker ever grows helper descendants.
        #[cfg(unix)]
        if let Some(process_group) = self.process_group {
            unsafe {
                libc::kill(-process_group, libc::SIGKILL);
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct WorkerChunkLine {
    index: usize,
    decode_start: f32,
    decode_end: f32,
    commit_start: f32,
    commit_end: f32,
    result: ParakeetJsonOutput,
}

/// Normalize a single word for overlap deduplication: lowercase, strip
/// punctuation/whitespace. Words that normalize to the same key AND overlap in
/// time are treated as the same word transcribed by two adjacent chunks.
fn normalize_word_text(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .trim_matches(|ch: char| !ch.is_alphanumeric())
        .to_string()
}

impl ParakeetCppEngine {
    pub fn new(binary_path: String, models_dir: String) -> Self {
        Self {
            binary_path,
            models_dir,
            worker_rss_limit_override_bytes: None,
        }
    }

    #[doc(hidden)]
    pub fn with_worker_rss_limit_override_for_test(mut self, limit_bytes: u64) -> Self {
        self.worker_rss_limit_override_bytes = Some(limit_bytes.max(1));
        self
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

    fn parakeet_target_lang(_language_code: &str) -> &str {
        // The persisted language is a preference for AI output.  Nemotron's
        // language marker stream must remain in automatic mode so it can
        // switch languages between utterances.
        "auto"
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
                "The selected legacy Parakeet live model cannot transcribe language '{language_code}'. Select NVIDIA Nemotron for multilingual live transcription or Parakeet TDT for file transcription."
            )));
        }

        self.validate_model_exists(final_model_filename)
    }

    fn configure_command_environment(command: &mut Command, binary_path: &str) {
        // Keep Metal enabled, but disable ggml Metal features that can make
        // the packaged runtime diverge from the dev runtime on Apple Silicon.
        // Do not auto-force CPU: CPU transcription is too heavy for the app UX
        // and must remain an explicit diagnostic override only.
        for (name, value) in Self::safe_metal_environment() {
            command.env(name, value);
        }
        if let Some(device) = Self::parakeet_device_override() {
            command.env("PARAKEET_DEVICE", device);
        }

        if let Some(binary_dir) = Path::new(binary_path)
            .canonicalize()
            .ok()
            .and_then(|path| path.parent().map(PathBuf::from))
        {
            let sibling_lib = binary_dir.join("../lib");
            let mut runtime_paths = vec![binary_dir.clone()];
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
    }

    fn parakeet_device_override() -> Option<&'static str> {
        if Self::truthy_env("SBOBINO_PARAKEET_FORCE_CPU") {
            return Some("cpu");
        }
        if Self::truthy_env("SBOBINO_PARAKEET_FORCE_METAL") {
            return None;
        }
        None
    }

    fn safe_metal_environment() -> &'static [(&'static str, &'static str)] {
        &[
            ("GGML_METAL_NO_RESIDENCY", "1"),
            ("GGML_METAL_SHARED_BUFFERS_DISABLE", "1"),
            ("GGML_METAL_CONCURRENCY_DISABLE", "1"),
        ]
    }

    fn configured_worker_rss_limit_bytes() -> u64 {
        match std::env::var(WORKER_RSS_LIMIT_ENV) {
            Ok(value) => match value.trim().parse::<u64>() {
                Ok(bytes) if bytes > 0 => bytes.min(DEFAULT_WORKER_RSS_LIMIT_BYTES),
                _ => {
                    eprintln!(
                        "Ignoring invalid {WORKER_RSS_LIMIT_ENV}={value:?}; using the safe default {} bytes",
                        DEFAULT_WORKER_RSS_LIMIT_BYTES
                    );
                    DEFAULT_WORKER_RSS_LIMIT_BYTES
                }
            },
            Err(_) => DEFAULT_WORKER_RSS_LIMIT_BYTES,
        }
    }

    fn worker_rss_limit_bytes(&self) -> u64 {
        self.worker_rss_limit_override_bytes
            .unwrap_or_else(Self::configured_worker_rss_limit_bytes)
    }

    fn allows_long_file_cli_fallback(is_windows: bool) -> bool {
        !is_windows
    }

    fn configure_worker_process_group(command: &mut Command) {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;

            // An isolated group gives cancellation and the RSS safety trip a
            // bounded target. Never signal an inherited parent group.
            command.as_std_mut().process_group(0);
        }
        #[cfg(not(unix))]
        {
            let _ = command;
        }
    }

    fn isolated_worker_process_group(worker_pid: u32) -> Option<i32> {
        #[cfg(unix)]
        {
            let worker_pid = i32::try_from(worker_pid).ok()?;
            let process_group = unsafe { libc::getpgid(worker_pid) };
            // `process_group(0)` creates a group led by the worker itself.
            // Refuse to signal anything else as a defensive guard.
            (process_group == worker_pid).then_some(process_group)
        }
        #[cfg(not(unix))]
        {
            let _ = worker_pid;
            None
        }
    }

    #[cfg(target_os = "macos")]
    fn macos_process_rss_bytes(pid: libc::pid_t) -> Option<u64> {
        let mut task_info = unsafe { std::mem::zeroed::<libc::proc_taskinfo>() };
        let expected_bytes = i32::try_from(std::mem::size_of::<libc::proc_taskinfo>()).ok()?;
        let bytes = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTASKINFO,
                0,
                (&mut task_info as *mut libc::proc_taskinfo).cast(),
                expected_bytes,
            )
        };
        (bytes == expected_bytes).then_some(task_info.pti_resident_size)
    }

    #[cfg(target_os = "macos")]
    fn macos_process_group_rss_bytes(process_group: i32) -> Option<u64> {
        let pid_size = std::mem::size_of::<libc::pid_t>();
        // `proc_listpgrppids` intentionally differs from `proc_listpids`:
        // libproc divides the underlying byte count before returning it, so
        // this value is a PID count, not a byte count.
        let required_count =
            unsafe { libc::proc_listpgrppids(process_group, std::ptr::null_mut(), 0) };
        if required_count <= 0 {
            return None;
        }
        let required_count = usize::try_from(required_count).ok()?;
        let mut pids = vec![0 as libc::pid_t; required_count];
        let buffer_bytes = i32::try_from(pids.len().checked_mul(pid_size)?).ok()?;
        let returned_count = unsafe {
            libc::proc_listpgrppids(process_group, pids.as_mut_ptr().cast(), buffer_bytes)
        };
        if returned_count <= 0 {
            return None;
        }
        let returned_count = usize::try_from(returned_count).ok()?;
        let process_count = returned_count.min(pids.len());
        let mut total = 0_u64;
        let mut found = false;
        for pid in pids.into_iter().take(process_count).filter(|pid| *pid > 0) {
            if let Some(rss) = Self::macos_process_rss_bytes(pid) {
                total = total.saturating_add(rss);
                found = true;
            }
        }
        found.then_some(total)
    }

    #[cfg(target_os = "linux")]
    fn linux_process_group_rss_bytes(process_group: i32, worker_pid: u32) -> Option<u64> {
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        let page_size = u64::try_from(page_size).ok().filter(|size| *size > 0)?;
        let mut total = 0_u64;
        let mut found = false;
        for entry in std::fs::read_dir("/proc").ok()?.flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
                continue;
            };
            let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
                continue;
            };
            let Some(close_name) = stat.rfind(')') else {
                continue;
            };
            let fields = stat[close_name + 1..]
                .split_whitespace()
                .collect::<Vec<_>>();
            // After `comm)`: state, ppid, then process group.
            let Some(candidate_group) = fields.get(2).and_then(|value| value.parse::<i32>().ok())
            else {
                continue;
            };
            if candidate_group != process_group {
                continue;
            }
            let Ok(statm) = std::fs::read_to_string(format!("/proc/{pid}/statm")) else {
                continue;
            };
            let Some(resident_pages) = statm
                .split_whitespace()
                .nth(1)
                .and_then(|value| value.parse::<u64>().ok())
            else {
                continue;
            };
            total = total.saturating_add(resident_pages.saturating_mul(page_size));
            found = true;
        }
        if found {
            Some(total)
        } else {
            let pid = i32::try_from(worker_pid).ok()?;
            let statm = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
            statm
                .split_whitespace()
                .nth(1)
                .and_then(|value| value.parse::<u64>().ok())
                .map(|resident_pages| resident_pages.saturating_mul(page_size))
        }
    }

    fn worker_process_group_rss_bytes(worker_pid: u32, process_group: Option<i32>) -> Option<u64> {
        #[cfg(target_os = "macos")]
        {
            process_group
                .and_then(Self::macos_process_group_rss_bytes)
                .or_else(|| {
                    i32::try_from(worker_pid)
                        .ok()
                        .and_then(Self::macos_process_rss_bytes)
                })
        }
        #[cfg(target_os = "linux")]
        {
            let process_group = process_group.or_else(|| i32::try_from(worker_pid).ok())?;
            Self::linux_process_group_rss_bytes(process_group, worker_pid)
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            let _ = (worker_pid, process_group);
            None
        }
    }

    fn sample_worker_memory(stats: &mut WorkerMemoryStats) -> Option<u64> {
        #[cfg(windows)]
        if let Some(job_handle) = stats.job_handle {
            // A failed query is fail-closed: report the ceiling so the caller
            // terminates the job instead of continuing without a guard.
            let rss = WindowsWorkerJobGuard::peak_memory_bytes_for_handle(job_handle)
                .unwrap_or(stats.limit_bytes);
            stats.peak_rss_bytes = stats.peak_rss_bytes.max(rss);
            return Some(rss);
        }

        let rss = Self::worker_process_group_rss_bytes(stats.worker_pid, stats.process_group)?;
        stats.peak_rss_bytes = stats.peak_rss_bytes.max(rss);
        Some(rss)
    }

    fn format_memory_bytes(bytes: u64) -> String {
        format!("{:.2} GiB", bytes as f64 / 1024_f64.powi(3))
    }

    fn worker_memory_limit_error(stats: WorkerMemoryStats) -> ApplicationError {
        ApplicationError::SpeechToText(format!(
            "SBOBINO_PARAKEET_MEMORY_LIMIT: parakeet-batch-json worker pid {}{} reached the safety RSS limit (peak {} / limit {}). The worker process group was terminated and the app will retry the complete audio coverage with smaller windows.",
            stats.worker_pid,
            stats
                .process_group
                .map(|group| format!(" (process group {group})"))
                .unwrap_or_default(),
            Self::format_memory_bytes(stats.peak_rss_bytes),
            Self::format_memory_bytes(stats.limit_bytes),
        ))
    }

    async fn terminate_and_reap_worker(
        child: &mut tokio::process::Child,
        process_group: Option<i32>,
    ) {
        #[cfg(unix)]
        if let Some(process_group) = process_group {
            unsafe {
                libc::kill(-process_group, libc::SIGTERM);
            }
        }

        if tokio::time::timeout(Duration::from_secs(2), child.wait())
            .await
            .is_ok()
        {
            return;
        }

        #[cfg(unix)]
        if let Some(process_group) = process_group {
            unsafe {
                libc::kill(-process_group, libc::SIGKILL);
            }
        }
        let _ = child.start_kill();
        let _ = child.wait().await;
    }

    async fn cleanup_worker_after_error(
        child: &mut tokio::process::Child,
        process_group_guard: &mut WorkerProcessGroupGuard,
        stdout_task: tokio::task::JoinHandle<()>,
        stderr_task: tokio::task::JoinHandle<String>,
    ) -> String {
        #[cfg(windows)]
        if let Some(job) = process_group_guard.job.as_ref() {
            job.terminate();
        }
        Self::terminate_and_reap_worker(child, process_group_guard.process_group).await;
        process_group_guard.disarm();
        let _ = stdout_task.await;
        stderr_task.await.unwrap_or_default()
    }

    fn truthy_env(name: &str) -> bool {
        std::env::var(name)
            .ok()
            .map(|value| {
                matches!(
                    value.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false)
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

    fn normalize_detected_language(value: &str) -> Option<String> {
        LanguageCode::try_from_code(value)
            .ok()
            .filter(|code| !code.is_auto() && code.as_code() != "und")
            .map(|code| code.as_code().to_string())
    }

    /// Nemotron may emit a marker in one chunk and the text in a later one,
    /// and some builds place the marker after the preceding utterance.  Keep
    /// marker parsing independent from whitespace normalization so both forms
    /// are handled without leaking service tags into transcript text.
    fn parse_language_marked_text(value: &str) -> Vec<(String, Option<String>)> {
        let mut pieces = Vec::<(String, Option<String>)>::new();
        let mut current = String::new();
        let mut current_language: Option<String> = None;
        let chars = value.chars().collect::<Vec<_>>();
        let mut index = 0;
        while index < chars.len() {
            if chars[index] == '<' {
                if let Some(close_offset) = chars[index + 1..].iter().position(|ch| *ch == '>') {
                    let close = index + 1 + close_offset;
                    let marker = chars[index + 1..close].iter().collect::<String>();
                    let marker_core = marker.trim_matches('|');
                    let marker_is_service = matches!(
                        marker_core.to_ascii_uppercase().as_str(),
                        "EOU" | "EOS" | "UNK"
                    );
                    let primary = marker_core.split(['-', '_']).next().unwrap_or_default();
                    let is_language_marker = (2..=3).contains(&primary.len())
                        && !marker_is_service
                        && marker_core
                            .chars()
                            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
                    if is_language_marker {
                        let normalized = Self::normalize_detected_language(marker_core);
                        let trailing_is_empty =
                            chars[close + 1..].iter().all(|ch| ch.is_whitespace());
                        let text = Self::clean_transcript_text(&current);
                        if !text.is_empty() {
                            // Markers emitted directly after text (or at the
                            // end of an output) label the preceding utterance;
                            // markers separated by whitespace from following
                            // text start the next utterance.
                            let marker_is_suffix = trailing_is_empty
                                || !current.chars().last().is_some_and(char::is_whitespace);
                            if marker_is_suffix {
                                let language_for_text =
                                    current_language.clone().or_else(|| normalized.clone());
                                pieces.push((text, language_for_text));
                                current.clear();
                                current_language = None;
                            } else {
                                pieces.push((text, current_language.clone()));
                                current.clear();
                                current_language = normalized;
                            }
                        } else {
                            current_language = normalized;
                        }
                        index = close + 1;
                        continue;
                    }
                }
            }
            current.push(chars[index]);
            index += 1;
        }
        let text = Self::clean_transcript_text(&current);
        if !text.is_empty() {
            pieces.push((text, current_language));
        }
        pieces
    }

    fn language_label_for_text(value: &str) -> Option<String> {
        Self::parse_language_marked_text(value)
            .into_iter()
            .find_map(|(_, language)| language)
    }

    fn tdt_language_detector() -> &'static LanguageDetector {
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
                Language::Hungarian,
                Language::Italian,
                Language::Latvian,
                Language::Lithuanian,
                Language::Polish,
                Language::Portuguese,
                Language::Romanian,
                Language::Slovak,
                Language::Slovene,
                Language::Spanish,
                Language::Swedish,
                Language::Russian,
                Language::Ukrainian,
            ];
            LanguageDetectorBuilder::from_languages(&languages)
                .with_minimum_relative_distance(0.25)
                .build()
        })
    }

    fn classify_tdt_segment(segment: &mut TimedSegment) {
        if segment.language_code.is_some() {
            return;
        }
        let alpha_tokens = segment
            .text
            .split_whitespace()
            .filter(|token| token.chars().any(char::is_alphabetic))
            .count();
        if alpha_tokens < 3 {
            return;
        }
        let detector = Self::tdt_language_detector();
        let Some(language) = detector.detect_language_of(segment.text.clone()) else {
            return;
        };
        let confidence = detector
            .compute_language_confidence_values(segment.text.clone())
            .into_iter()
            .find(|(candidate, _)| *candidate == language)
            .map(|(_, value)| value as f32)
            .filter(|value| value.is_finite());
        segment.language_code = Some(language.iso_code_639_1().to_string().to_ascii_lowercase());
        segment.language_confidence = confidence;
    }

    fn classify_tdt_output(output: &mut TranscriptionOutput) {
        for segment in &mut output.segments {
            Self::classify_tdt_segment(segment);
        }
    }

    fn parse_json_output(
        raw_stdout: &str,
        total_audio_seconds: Option<f32>,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        let payload = Self::extract_json_payload(raw_stdout)?;
        let parsed: ParakeetJsonOutput = serde_json::from_str(payload).map_err(|error| {
            ApplicationError::SpeechToText(format!("failed to parse parakeet-cli JSON: {error}"))
        })?;

        Self::transcription_from_parakeet_json(parsed, total_audio_seconds)
    }

    fn transcription_from_parakeet_json(
        parsed: ParakeetJsonOutput,
        total_audio_seconds: Option<f32>,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        let marked_text = Self::parse_language_marked_text(&parsed.text);
        let text = if marked_text.is_empty() {
            Self::clean_transcript_text(&parsed.text)
        } else {
            marked_text
                .iter()
                .map(|(text, _)| text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        };
        let segment_text = parsed
            .segments
            .iter()
            .flat_map(|segment| {
                let marked = Self::parse_language_marked_text(&segment.text);
                if marked.is_empty() {
                    let cleaned = Self::clean_transcript_text(&segment.text);
                    if cleaned.is_empty() {
                        Vec::new()
                    } else {
                        vec![cleaned]
                    }
                } else {
                    marked.into_iter().map(|(text, _)| text).collect()
                }
            })
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
        let raw_segments = if parsed.segments.is_empty() {
            if marked_text.len() > 1 {
                marked_text
                    .iter()
                    .map(|(text, language_code)| TimedSegment {
                        text: text.clone(),
                        start_seconds: None,
                        end_seconds: None,
                        speaker_id: None,
                        speaker_label: None,
                        language_code: language_code.clone(),
                        language_confidence: parsed
                            .language_confidence
                            .filter(|value| value.is_finite()),
                        words: Vec::new(),
                    })
                    .collect::<Vec<_>>()
            } else {
                Self::segments_from_words(text.clone(), parsed.words, total_audio_seconds)
            }
        } else if has_flat_words && !segment_words_available {
            Self::segments_from_words(text.clone(), parsed.words, total_audio_seconds)
        } else {
            parsed
                .segments
                .into_iter()
                .flat_map(Self::segments_from_json)
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
                language_code: None,
                language_confidence: None,
                words: Vec::new(),
            }]
        } else {
            raw_segments
        };

        let output_language = parsed
            .language
            .as_deref()
            .and_then(Self::normalize_detected_language)
            .or_else(|| Self::language_label_for_text(&parsed.text));
        // A top-level language is safe only when the model emitted no
        // per-utterance markers (or one unambiguous marked utterance).  Once
        // Nemotron has emitted multiple marker pieces, leave an unlabelled
        // piece undetermined instead of propagating the first language across
        // a boundary.
        let explicit_marker_count = marked_text
            .iter()
            .filter(|(_, language)| language.is_some())
            .count();
        let output_language_fallback = if explicit_marker_count == 0
            || (explicit_marker_count == 1 && marked_text.len() == 1)
        {
            output_language
        } else {
            None
        };
        let raw_segments = raw_segments
            .into_iter()
            .map(|mut segment| {
                if segment.language_code.is_none() {
                    segment.language_code = output_language_fallback.clone();
                }
                if segment.language_confidence.is_none() {
                    segment.language_confidence =
                        parsed.language_confidence.filter(|value| value.is_finite());
                }
                segment
            })
            .collect::<Vec<_>>();

        Ok(TranscriptionOutput {
            text: text.clone(),
            segments: normalize_transcript_segments(&text, &raw_segments, total_audio_seconds),
        })
    }

    fn transcription_from_worker_chunk_json(
        parsed: ParakeetJsonOutput,
        total_audio_seconds: Option<f32>,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        // A fully valid batch-worker row can represent a silent commit window.
        // Keep that row in coverage/progress accounting, but do not weaken the
        // single-file CLI contract: an otherwise-successful direct CLI response
        // with no transcript still remains an error through
        // `transcription_from_parakeet_json`.
        if parsed.text.trim().is_empty() && parsed.segments.is_empty() && parsed.words.is_empty() {
            return Ok(TranscriptionOutput {
                text: String::new(),
                segments: Vec::new(),
            });
        }
        Self::transcription_from_parakeet_json(parsed, total_audio_seconds)
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
                language_code: None,
                language_confidence: None,
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

        let language_code = segment.language_code.clone();
        let language_confidence = segment.language_confidence;
        Self::segments_from_timed_words(segment.text, segment.words, total_audio_seconds)
            .into_iter()
            .map(|mut split_segment| {
                split_segment.language_code = language_code.clone();
                split_segment.language_confidence = language_confidence;
                split_segment
            })
            .collect()
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
                language_code: None,
                language_confidence: None,
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
                language_code: None,
                language_confidence: None,
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

    fn segments_from_json(segment: ParakeetJsonSegment) -> Vec<TimedSegment> {
        let marked_text = Self::parse_language_marked_text(&segment.text);
        let text = if marked_text.is_empty() {
            let cleaned = Self::clean_transcript_text(&segment.text);
            if cleaned.is_empty() {
                return Vec::new();
            }
            vec![(cleaned, None)]
        } else {
            marked_text
        };
        let words = segment
            .words
            .into_iter()
            .filter_map(Self::word_from_json)
            .collect::<Vec<_>>();

        let text_count = text.len().max(1);
        text.into_iter()
            .enumerate()
            .map(|(index, (text, marker_language))| TimedSegment {
                text,
                start_seconds: segment.start.filter(|value| value.is_finite()),
                end_seconds: segment.end.filter(|value| value.is_finite()),
                speaker_id: None,
                speaker_label: None,
                language_code: marker_language.or_else(|| {
                    segment
                        .language
                        .as_deref()
                        .and_then(Self::normalize_detected_language)
                }),
                language_confidence: segment
                    .language_confidence
                    .filter(|value| value.is_finite()),
                words: if index + 1 == text_count {
                    words.clone()
                } else {
                    Vec::new()
                },
            })
            .collect()
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
        let mut command = tokio_background_command(&self.binary_path);
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
        let mut command = tokio_background_command(&self.binary_path);
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

    fn should_use_long_file_chunking(input_wav: &Path, total_audio_seconds: Option<f32>) -> bool {
        if total_audio_seconds.is_some_and(|seconds| seconds >= LONG_FILE_THRESHOLD_SECONDS) {
            return true;
        }
        let Ok(reader) = hound::WavReader::open(input_wav) else {
            return false;
        };
        let spec = reader.spec();
        let frames = reader.duration() as f32 / f32::from(spec.channels.max(1));
        (frames / spec.sample_rate.max(1) as f32) >= LONG_FILE_THRESHOLD_SECONDS
    }

    fn prepare_long_file_chunks(
        input_wav: &Path,
        target_seconds: f32,
    ) -> Result<(TempDir, Vec<AudioChunk>), ApplicationError> {
        let mut reader = hound::WavReader::open(input_wav).map_err(|error| {
            ApplicationError::SpeechToText(format!(
                "Parakeet long-file chunking could not read WAV {}: {error}",
                input_wav.display()
            ))
        })?;
        let spec = reader.spec();
        if spec.channels != 1
            || spec.sample_rate == 0
            || spec.bits_per_sample > 16
            || spec.sample_format != hound::SampleFormat::Int
        {
            return Err(ApplicationError::SpeechToText(format!(
                "Parakeet long-file chunking expected normalized PCM16 mono WAV, got channels={} rate={} bits={} format={:?}",
                spec.channels, spec.sample_rate, spec.bits_per_sample, spec.sample_format
            )));
        }

        let samples = reader
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| {
                ApplicationError::SpeechToText(format!(
                    "failed to decode WAV samples for Parakeet long-file chunking: {error}"
                ))
            })?;
        if samples.is_empty() {
            return Err(ApplicationError::SpeechToText(
                "Parakeet long-file chunking received empty WAV".to_string(),
            ));
        }

        if !target_seconds.is_finite() {
            return Err(ApplicationError::SpeechToText(
                "Parakeet long-file chunk target must be finite".to_string(),
            ));
        }

        let sample_rate = spec.sample_rate as usize;
        let total_frames = samples.len();
        let min_commit_frames = ((LONG_FILE_MIN_COMMIT_WINDOW_SECONDS * spec.sample_rate as f32)
            .round() as usize)
            .max(sample_rate);
        let context_frames = ((LONG_FILE_CONTEXT_SECONDS * spec.sample_rate as f32).round()
            as usize)
            .min(total_frames);
        let hard_max_decode_frames =
            ((LONG_FILE_MAX_SERIALIZED_DECODE_SECONDS * spec.sample_rate as f32).round() as usize)
                .max(sample_rate);
        // An interior decode needs context on both sides. Keep snapping inside
        // that budget even if a future target constant is accidentally raised.
        let max_commit_frames = hard_max_decode_frames
            .saturating_sub(context_frames.saturating_mul(2))
            .max(min_commit_frames);
        let target_seconds = target_seconds.clamp(
            LONG_FILE_MIN_COMMIT_WINDOW_SECONDS,
            LONG_FILE_TARGET_COMMIT_WINDOW_SECONDS,
        );
        let target_frames = ((target_seconds * spec.sample_rate as f32).round() as usize)
            .max(min_commit_frames)
            .min(max_commit_frames);
        let initial_target_frames = ((LONG_FILE_INITIAL_COMMIT_WINDOW_SECONDS.min(target_seconds)
            * spec.sample_rate as f32)
            .round() as usize)
            .max(min_commit_frames)
            .min(target_frames);
        let snap_radius =
            ((LONG_FILE_BOUNDARY_SNAP_SECONDS * spec.sample_rate as f32).round() as usize).max(1);
        let rms_window = ((LONG_FILE_BOUNDARY_RMS_WINDOW_SECONDS * spec.sample_rate as f32).round()
            as usize)
            .max(1);
        let tail_pad_frames = ((LONG_FILE_TAIL_PAD_SECONDS * spec.sample_rate as f32).round()
            as usize)
            .min(sample_rate * 5);

        let mut boundaries = vec![0usize];
        let mut cursor = 0usize;
        while total_frames.saturating_sub(cursor) > target_frames {
            let current_target_frames = if cursor == 0 {
                initial_target_frames
            } else {
                target_frames
            };
            let ideal = cursor + current_target_frames;
            // Search only backward from the intended commit edge. A later
            // silence would enlarge the serialized decode; an earlier silence
            // remains valid only down to the named 10 s minimum commit window.
            let min_next = cursor.saturating_add(min_commit_frames).min(total_frames);
            let snap_min = ideal.saturating_sub(snap_radius).max(min_next);
            let snap_max = ideal.min(cursor.saturating_add(max_commit_frames));
            let boundary = Self::quietest_boundary(
                &samples,
                ideal,
                snap_min.min(total_frames),
                snap_max.min(total_frames),
                rms_window,
            );
            if boundary <= cursor || boundary >= total_frames {
                return Err(ApplicationError::SpeechToText(format!(
                    "Parakeet long-file chunk planner could not choose a bounded split after {cursor} frames"
                )));
            }
            boundaries.push(boundary);
            cursor = boundary;
        }
        boundaries.push(total_frames);
        boundaries.dedup();

        let temp_dir = tempfile::Builder::new()
            .prefix("sbobino-parakeet-final-")
            .tempdir()
            .map_err(|error| {
                ApplicationError::SpeechToText(format!(
                    "failed to create Parakeet final chunk directory: {error}"
                ))
            })?;

        let mut chunks = Vec::new();
        for pair in boundaries.windows(2) {
            let index = chunks.len();
            let commit_start = pair[0];
            let commit_end = pair[1];
            if commit_end <= commit_start {
                return Err(ApplicationError::SpeechToText(
                    "Parakeet long-file chunk planner produced a non-positive commit window"
                        .to_string(),
                ));
            }
            let commit_frames = commit_end - commit_start;
            if commit_frames > hard_max_decode_frames {
                return Err(ApplicationError::SpeechToText(format!(
                    "Parakeet long-file commit window exceeds the {} s decode safety budget",
                    LONG_FILE_MAX_SERIALIZED_DECODE_SECONDS
                )));
            }
            let context_budget = hard_max_decode_frames.saturating_sub(commit_frames);
            let left_context_frames = context_frames.min(commit_start).min(context_budget);
            let remaining_context_budget = context_budget.saturating_sub(left_context_frames);
            let right_context_frames = context_frames
                .min(total_frames.saturating_sub(commit_end))
                .min(remaining_context_budget);
            let remaining_pad_budget =
                remaining_context_budget.saturating_sub(right_context_frames);
            let pad_frames = if commit_end == total_frames {
                tail_pad_frames.min(remaining_pad_budget)
            } else {
                0
            };
            let decode_start = commit_start.saturating_sub(left_context_frames);
            let decode_audio_end = commit_end.saturating_add(right_context_frames);
            let serialized_decode_frames = decode_audio_end
                .saturating_sub(decode_start)
                .saturating_add(pad_frames);
            if serialized_decode_frames > hard_max_decode_frames {
                return Err(ApplicationError::SpeechToText(format!(
                    "Parakeet long-file planner would serialize a {:.3}s decode, above the {}s safety limit",
                    serialized_decode_frames as f32 / spec.sample_rate as f32,
                    LONG_FILE_MAX_SERIALIZED_DECODE_SECONDS
                )));
            }
            let path = temp_dir.path().join(format!("chunk-{index:04}.wav"));
            let mut writer = hound::WavWriter::create(&path, spec).map_err(|error| {
                ApplicationError::SpeechToText(format!(
                    "failed to create Parakeet final chunk {}: {error}",
                    path.display()
                ))
            })?;
            for sample in &samples[decode_start..decode_audio_end] {
                writer.write_sample(*sample).map_err(|error| {
                    ApplicationError::SpeechToText(format!(
                        "failed to write Parakeet final chunk {}: {error}",
                        path.display()
                    ))
                })?;
            }
            for _ in 0..pad_frames {
                writer.write_sample(0i16).map_err(|error| {
                    ApplicationError::SpeechToText(format!(
                        "failed to pad Parakeet final chunk {}: {error}",
                        path.display()
                    ))
                })?;
            }
            writer.finalize().map_err(|error| {
                ApplicationError::SpeechToText(format!(
                    "failed to finalize Parakeet final chunk {}: {error}",
                    path.display()
                ))
            })?;
            chunks.push(AudioChunk {
                index,
                path,
                decode_start_seconds: decode_start as f32 / spec.sample_rate as f32,
                decode_end_seconds: (decode_audio_end + pad_frames) as f32
                    / spec.sample_rate as f32,
                commit_start_seconds: commit_start as f32 / spec.sample_rate as f32,
                commit_end_seconds: commit_end as f32 / spec.sample_rate as f32,
            });
        }

        Self::validate_audio_chunks(&chunks, Some(total_frames as f32 / spec.sample_rate as f32))?;

        Ok((temp_dir, chunks))
    }

    fn quietest_boundary(
        samples: &[i16],
        ideal: usize,
        min: usize,
        max: usize,
        window: usize,
    ) -> usize {
        if samples.is_empty() {
            return 0;
        }
        let min = min.min(samples.len().saturating_sub(1));
        let max = max.min(samples.len().saturating_sub(1));
        if min >= max {
            return min;
        }
        let step = (window / 2).max(1);
        let mut best = ideal.min(samples.len());
        let mut best_energy = f64::INFINITY;
        let mut pos = min;
        while pos <= max {
            let end = (pos + window).min(samples.len());
            if end > pos {
                let energy = samples[pos..end]
                    .iter()
                    .map(|sample| {
                        let value = f64::from(*sample);
                        value * value
                    })
                    .sum::<f64>()
                    / (end - pos) as f64;
                if energy < best_energy
                    || ((energy - best_energy).abs() <= f64::EPSILON
                        && pos.abs_diff(ideal) < best.abs_diff(ideal))
                {
                    best_energy = energy;
                    best = pos;
                }
            }
            let next = pos.saturating_add(step);
            if next <= pos {
                break;
            }
            pos = next;
        }
        best
    }

    fn validate_audio_chunks(
        chunks: &[AudioChunk],
        expected_commit_end_seconds: Option<f32>,
    ) -> Result<(), ApplicationError> {
        if chunks.is_empty() {
            return Err(ApplicationError::SpeechToText(
                "Parakeet long-file worker manifest cannot be empty".to_string(),
            ));
        }

        let tolerance = LONG_FILE_CHUNK_VALIDATION_TOLERANCE_SECONDS;
        let mut previous_decode_start = None;
        let mut previous_commit_end = 0.0_f32;
        for (expected_index, chunk) in chunks.iter().enumerate() {
            let values = [
                chunk.decode_start_seconds,
                chunk.decode_end_seconds,
                chunk.commit_start_seconds,
                chunk.commit_end_seconds,
            ];
            if values.iter().any(|value| !value.is_finite()) {
                return Err(ApplicationError::SpeechToText(format!(
                    "Parakeet long-file chunk {} contains a non-finite timestamp",
                    chunk.index
                )));
            }
            if chunk.index != expected_index {
                return Err(ApplicationError::SpeechToText(format!(
                    "Parakeet long-file chunk index {} is not monotonic (expected {})",
                    chunk.index, expected_index
                )));
            }
            if chunk.path.as_os_str().is_empty() {
                return Err(ApplicationError::SpeechToText(format!(
                    "Parakeet long-file chunk {} has no audio path",
                    chunk.index
                )));
            }
            if chunk.decode_start_seconds < 0.0
                || chunk.commit_start_seconds < 0.0
                || chunk.decode_end_seconds < chunk.decode_start_seconds
                || chunk.commit_end_seconds <= chunk.commit_start_seconds
                || chunk.commit_start_seconds < chunk.decode_start_seconds
                || chunk.commit_end_seconds > chunk.decode_end_seconds
            {
                return Err(ApplicationError::SpeechToText(format!(
                    "Parakeet long-file chunk {} has invalid decode/commit ordering",
                    chunk.index
                )));
            }
            if chunk.decode_end_seconds - chunk.decode_start_seconds
                > LONG_FILE_MAX_SERIALIZED_DECODE_SECONDS
            {
                return Err(ApplicationError::SpeechToText(format!(
                    "Parakeet long-file chunk {} serializes {:.3}s, above the {}s safety limit",
                    chunk.index,
                    chunk.decode_end_seconds - chunk.decode_start_seconds,
                    LONG_FILE_MAX_SERIALIZED_DECODE_SECONDS
                )));
            }
            if let Some(previous_decode_start) = previous_decode_start {
                if chunk.decode_start_seconds + tolerance < previous_decode_start {
                    return Err(ApplicationError::SpeechToText(format!(
                        "Parakeet long-file chunk {} has non-monotonic decode order",
                        chunk.index
                    )));
                }
            }
            if (chunk.commit_start_seconds - previous_commit_end).abs() > tolerance {
                return Err(ApplicationError::SpeechToText(format!(
                    "Parakeet long-file chunk {} breaks commit coverage (expected {:.3}, got {:.3})",
                    chunk.index, previous_commit_end, chunk.commit_start_seconds
                )));
            }
            previous_decode_start = Some(chunk.decode_start_seconds);
            previous_commit_end = chunk.commit_end_seconds;
        }

        if let Some(expected_commit_end_seconds) = expected_commit_end_seconds {
            if !expected_commit_end_seconds.is_finite()
                || (previous_commit_end - expected_commit_end_seconds).abs() > tolerance
            {
                return Err(ApplicationError::SpeechToText(format!(
                    "Parakeet long-file chunks do not cover the full audio (ended at {:.3}, expected {:.3})",
                    previous_commit_end, expected_commit_end_seconds
                )));
            }
        }
        Ok(())
    }

    fn parakeet_worker_path(&self) -> Option<PathBuf> {
        let cli = Path::new(&self.binary_path).canonicalize().ok()?;
        let worker_name = if cfg!(windows) {
            "parakeet-batch-json.exe"
        } else {
            "parakeet-batch-json"
        };
        let candidate = cli.parent()?.join(worker_name);
        candidate.exists().then_some(candidate)
    }

    fn write_worker_manifest(
        chunks: &[AudioChunk],
    ) -> Result<tempfile::NamedTempFile, ApplicationError> {
        // Do not hand a malformed plan to the native sidecar. The worker
        // repeats these checks independently because the manifest is a process
        // boundary, not a trusted in-memory interface.
        Self::validate_audio_chunks(chunks, None)?;
        let mut manifest = tempfile::Builder::new()
            .prefix("sbobino-parakeet-worker-")
            .suffix(".tsv")
            .tempfile()
            .map_err(|error| {
                ApplicationError::SpeechToText(format!(
                    "failed to create Parakeet worker manifest: {error}"
                ))
            })?;
        for chunk in chunks {
            writeln!(
                manifest,
                "{}\t{:.3}\t{:.3}\t{:.3}\t{:.3}\t{}",
                chunk.index,
                chunk.decode_start_seconds,
                chunk.decode_end_seconds,
                chunk.commit_start_seconds,
                chunk.commit_end_seconds,
                chunk.path.display()
            )
            .map_err(|error| {
                ApplicationError::SpeechToText(format!(
                    "failed to write Parakeet worker manifest: {error}"
                ))
            })?;
        }
        manifest.flush().map_err(|error| {
            ApplicationError::SpeechToText(format!(
                "failed to flush Parakeet worker manifest: {error}"
            ))
        })?;
        Ok(manifest)
    }

    fn validate_worker_chunk_line(
        parsed: &WorkerChunkLine,
        expected: &AudioChunk,
    ) -> Result<(), ApplicationError> {
        let values = [
            parsed.decode_start,
            parsed.decode_end,
            parsed.commit_start,
            parsed.commit_end,
        ];
        if values.iter().any(|value| !value.is_finite()) {
            return Err(ApplicationError::SpeechToText(format!(
                "parakeet-batch-json returned non-finite timestamps for chunk {}",
                parsed.index
            )));
        }
        if parsed.index != expected.index {
            return Err(ApplicationError::SpeechToText(format!(
                "parakeet-batch-json returned non-monotonic chunk index {} (expected {})",
                parsed.index, expected.index
            )));
        }
        if parsed.decode_start < 0.0
            || parsed.commit_start < 0.0
            || parsed.decode_end < parsed.decode_start
            || parsed.commit_end <= parsed.commit_start
            || parsed.commit_start < parsed.decode_start
            || parsed.commit_end > parsed.decode_end
            || parsed.decode_end - parsed.decode_start > LONG_FILE_MAX_SERIALIZED_DECODE_SECONDS
        {
            return Err(ApplicationError::SpeechToText(format!(
                "parakeet-batch-json returned invalid decode/commit bounds for chunk {}",
                parsed.index
            )));
        }

        let expected_values = [
            expected.decode_start_seconds,
            expected.decode_end_seconds,
            expected.commit_start_seconds,
            expected.commit_end_seconds,
        ];
        if values
            .iter()
            .zip(expected_values.iter())
            .any(|(actual, expected)| {
                (actual - expected).abs() > LONG_FILE_CHUNK_VALIDATION_TOLERANCE_SECONDS
            })
        {
            return Err(ApplicationError::SpeechToText(format!(
                "parakeet-batch-json returned metadata that does not match manifest chunk {}",
                parsed.index
            )));
        }
        Ok(())
    }

    async fn run_worker_for_chunks(
        &self,
        worker_path: &Path,
        model_path: &Path,
        language_code: &str,
        chunks: &[AudioChunk],
        total_audio_seconds: Option<f32>,
        callbacks: &LongFileCallbacks,
    ) -> Result<Vec<(AudioChunk, TranscriptionOutput)>, ApplicationError> {
        let manifest = Self::write_worker_manifest(chunks)?;
        let mut command = tokio_background_command(worker_path);
        Self::configure_command_environment(&mut command, worker_path.to_string_lossy().as_ref());
        Self::configure_worker_process_group(&mut command);
        command
            .arg("--model")
            .arg(model_path)
            .arg("--manifest")
            .arg(manifest.path())
            .arg("--lang")
            .arg(Self::parakeet_target_lang(language_code))
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        command.kill_on_drop(true);
        let mut child = command.spawn().map_err(|error| {
            ApplicationError::SpeechToText(format!(
                "parakeet-batch-json failed to start at '{}': {error}",
                worker_path.display()
            ))
        })?;
        let worker_pid = child.id().ok_or_else(|| {
            ApplicationError::SpeechToText(
                "parakeet-batch-json started without a visible worker PID".to_string(),
            )
        })?;
        let process_group = Self::isolated_worker_process_group(worker_pid);
        #[cfg(windows)]
        let windows_job = {
            let process_handle = match child.raw_handle() {
                Some(handle) => handle as windows_sys::Win32::Foundation::HANDLE,
                None => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    return Err(ApplicationError::SpeechToText(
                        "parakeet-batch-json started without a process handle for the Windows memory guard"
                            .to_string(),
                    ));
                }
            };
            match WindowsWorkerJobGuard::attach(process_handle, self.worker_rss_limit_bytes()) {
                Ok(job) => Some(job),
                Err(error) => {
                    let _ = child.start_kill();
                    let _ = child.wait().await;
                    return Err(ApplicationError::SpeechToText(format!(
                        "parakeet-batch-json Windows memory guard setup failed closed: {error}"
                    )));
                }
            }
        };
        #[cfg(windows)]
        let mut process_group_guard = WorkerProcessGroupGuard::new(process_group, windows_job);
        #[cfg(not(windows))]
        let mut process_group_guard = WorkerProcessGroupGuard::new(process_group);
        let mut memory_stats = WorkerMemoryStats {
            worker_pid,
            process_group,
            peak_rss_bytes: 0,
            limit_bytes: self.worker_rss_limit_bytes(),
            #[cfg(windows)]
            job_handle: process_group_guard.job.as_ref().map(|job| job.handle),
        };
        let stdout = child.stdout.take().ok_or_else(|| {
            ApplicationError::SpeechToText("parakeet-batch-json stdout was unavailable".to_string())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            ApplicationError::SpeechToText("parakeet-batch-json stderr was unavailable".to_string())
        })?;
        let (stdout_sender, mut stdout_receiver) = tokio::sync::mpsc::channel(32);
        let stdout_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        if stdout_sender.send(Ok(line)).await.is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        let _ = stdout_sender.send(Err(error.to_string())).await;
                        break;
                    }
                }
            }
        });
        let stderr_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            let mut tail = Vec::<String>::new();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                if tail.len() >= 120 {
                    tail.remove(0);
                }
                tail.push(line);
            }
            tail.join("\n")
        });

        macro_rules! abort_worker {
            ($error:expr) => {{
                let error = $error;
                drop(stdout_receiver);
                let stderr = Self::cleanup_worker_after_error(
                    &mut child,
                    &mut process_group_guard,
                    stdout_task,
                    stderr_task,
                )
                .await;
                if !stderr.trim().is_empty() {
                    eprintln!("parakeet-batch-json aborted diagnostic: {stderr}");
                }
                return Err(error);
            }};
        }

        let mut results = Vec::new();
        let mut monitor = tokio::time::interval(WORKER_RSS_SAMPLE_INTERVAL);
        monitor.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut stdout_closed = false;
        let mut status = None;
        while status.is_none() || !stdout_closed {
            tokio::select! {
                _ = monitor.tick() => {
                    if status.is_none() {
                        if let Some(rss) = Self::sample_worker_memory(&mut memory_stats) {
                            if rss >= memory_stats.limit_bytes {
                                let error = Self::worker_memory_limit_error(memory_stats);
                                abort_worker!(error);
                            }
                        }
                        match child.try_wait() {
                            Ok(Some(exit_status)) => status = Some(exit_status),
                            Ok(None) => {}
                            Err(error) => abort_worker!(ApplicationError::SpeechToText(format!(
                                "failed to poll parakeet-batch-json worker: {error}"
                            ))),
                        }
                    }
                }
                line = stdout_receiver.recv(), if !stdout_closed => {
                    match line {
                        Some(Ok(line)) => {
                            if line.trim().is_empty() {
                                continue;
                            }
                            let parsed: WorkerChunkLine = match serde_json::from_str(&line) {
                                Ok(parsed) => parsed,
                                Err(error) => abort_worker!(ApplicationError::SpeechToText(format!(
                                    "failed to parse parakeet-batch-json output: {error}"
                                ))),
                            };
                            let Some(chunk) = chunks.get(results.len()).cloned() else {
                                abort_worker!(ApplicationError::SpeechToText(format!(
                                    "parakeet-batch-json returned more chunk results than the {} planned chunks",
                                    chunks.len()
                                )));
                            };
                            if let Err(error) = Self::validate_worker_chunk_line(&parsed, &chunk) {
                                abort_worker!(error);
                            }
                            let mut output = match Self::transcription_from_worker_chunk_json(
                                parsed.result,
                                Some((chunk.decode_end_seconds - chunk.decode_start_seconds).max(0.0)),
                            ) {
                                Ok(output) => output,
                                Err(error) => abort_worker!(error),
                            };
                            Self::offset_transcription_output(&mut output, chunk.decode_start_seconds);
                            Self::filter_transcription_to_commit_window(
                                &mut output,
                                chunk.commit_start_seconds,
                                chunk.commit_end_seconds,
                            );
                            let commit_end_seconds = chunk.commit_end_seconds;
                            results.push((chunk, output));
                            if let Some(snapshot) = match Self::merge_chunk_transcriptions_snapshot(
                                &results,
                                total_audio_seconds,
                            ) {
                                Ok(snapshot) => snapshot,
                                Err(error) => abort_worker!(error),
                            } {
                                (callbacks.emit_partial)(format!("{DELTA_REPLACE_PREFIX}{}", snapshot.text));
                            }
                            // A silent but valid worker row has no partial text,
                            // yet it still completes its commit window.
                            (callbacks.emit_progress_seconds)(commit_end_seconds);
                        }
                        Some(Err(error)) => abort_worker!(ApplicationError::SpeechToText(format!(
                            "failed to read parakeet-batch-json output: {error}"
                        ))),
                        None => stdout_closed = true,
                    }
                }
            }
        }

        let status = status.expect("worker loop exits only after a child status");
        stdout_task.await.map_err(|error| {
            ApplicationError::SpeechToText(format!(
                "failed to join parakeet-batch-json stdout reader: {error}"
            ))
        })?;
        let stderr = stderr_task.await.map_err(|error| {
            ApplicationError::SpeechToText(format!(
                "failed to join parakeet-batch-json stderr reader: {error}"
            ))
        })?;
        process_group_guard.disarm();
        if memory_stats.peak_rss_bytes > 0 {
            eprintln!(
                "parakeet-batch-json worker pid {}{} peak RSS {} (limit {})",
                memory_stats.worker_pid,
                memory_stats
                    .process_group
                    .map(|group| format!(" process group {group}"))
                    .unwrap_or_default(),
                Self::format_memory_bytes(memory_stats.peak_rss_bytes),
                Self::format_memory_bytes(memory_stats.limit_bytes),
            );
        } else {
            eprintln!(
                "parakeet-batch-json worker pid {} exited before an RSS sample was available",
                memory_stats.worker_pid
            );
        }
        if !status.success() {
            return Err(Self::parakeet_command_failure(
                "parakeet-batch-json failed",
                stderr.as_bytes(),
                Some(status.to_string()),
            ));
        }
        if results.len() != chunks.len() {
            return Err(ApplicationError::SpeechToText(format!(
                "parakeet-batch-json returned {} chunk result(s), expected {}",
                results.len(),
                chunks.len()
            )));
        }
        results.sort_by_key(|(chunk, _)| chunk.index);
        Ok(results)
    }

    async fn run_cli_for_chunk(
        &self,
        chunk: &AudioChunk,
        model_path: &Path,
        model_filename: &str,
        language_code: &str,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        let mut command = Command::new(&self.binary_path);
        Self::configure_command_environment(&mut command, &self.binary_path);
        command
            .arg("transcribe")
            .arg("--model")
            .arg(model_path)
            .arg("--input")
            .arg(&chunk.path)
            .arg("--json")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        command.kill_on_drop(true);
        if Self::is_nemotron_streaming_model(model_filename) {
            command
                .arg("--lang")
                .arg(Self::parakeet_target_lang(language_code));
        }
        let output = command.output().await.map_err(|error| {
            ApplicationError::SpeechToText(format!(
                "parakeet-cli failed to start at '{}': {error}. Configure Parakeet CLI path in Settings > Local Models.",
                self.binary_path
            ))
        })?;
        if !output.status.success() {
            return Err(Self::parakeet_command_failure(
                "parakeet-cli chunk failed",
                &output.stderr,
                Some(output.status.to_string()),
            ));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut parsed = Self::parse_json_output(
            &stdout,
            Some((chunk.decode_end_seconds - chunk.decode_start_seconds).max(0.0)),
        )?;
        Self::offset_transcription_output(&mut parsed, chunk.decode_start_seconds);
        Self::filter_transcription_to_commit_window(
            &mut parsed,
            chunk.commit_start_seconds,
            chunk.commit_end_seconds,
        );
        Ok(parsed)
    }

    async fn run_cli_for_chunks(
        &self,
        chunks: &[AudioChunk],
        model_path: &Path,
        model_filename: &str,
        language_code: &str,
    ) -> Result<Vec<(AudioChunk, TranscriptionOutput)>, ApplicationError> {
        let mut results = Vec::new();
        for chunk in chunks {
            let output = self
                .run_cli_for_chunk(chunk, model_path, model_filename, language_code)
                .await?;
            results.push((chunk.clone(), output));
        }
        Ok(results)
    }

    async fn run_long_file_transcription(
        &self,
        input_wav: &Path,
        model_path: &Path,
        model_filename: &str,
        language_code: &str,
        total_audio_seconds: Option<f32>,
        callbacks: LongFileCallbacks,
    ) -> Result<TranscriptionOutput, ApplicationError> {
        let target_sizes = std::iter::once(LONG_FILE_TARGET_COMMIT_WINDOW_SECONDS)
            .chain(LONG_FILE_RETRY_COMMIT_WINDOW_SECONDS)
            .collect::<Vec<_>>();
        let mut last_error = None;
        for chunk_seconds in target_sizes {
            let (_temp_dir, chunks) = Self::prepare_long_file_chunks(input_wav, chunk_seconds)?;
            if chunks.is_empty() {
                break;
            }
            let attempt = if let Some(worker_path) = self.parakeet_worker_path() {
                match self
                    .run_worker_for_chunks(
                        &worker_path,
                        model_path,
                        language_code,
                        &chunks,
                        total_audio_seconds,
                        &callbacks,
                    )
                    .await
                {
                    Ok(results) => Ok(results),
                    Err(error) if Self::is_retryable_long_file_memory_error(&error.to_string()) => {
                        Err(error)
                    }
                    Err(error) => {
                        if Self::allows_long_file_cli_fallback(cfg!(windows)) {
                            eprintln!(
                                "Parakeet worker unavailable or failed, falling back to chunk CLI: {error}"
                            );
                            self.run_cli_for_chunks(
                                &chunks,
                                model_path,
                                model_filename,
                                language_code,
                            )
                            .await
                        } else {
                            Err(error)
                        }
                    }
                }
            } else {
                if Self::allows_long_file_cli_fallback(cfg!(windows)) {
                    self.run_cli_for_chunks(&chunks, model_path, model_filename, language_code)
                        .await
                } else {
                    Err(ApplicationError::SpeechToText(
                        "Parakeet batch worker is missing; refusing an unguarded Windows CLI fallback"
                            .to_string(),
                    ))
                }
            };

            match attempt {
                Ok(results) => {
                    return Self::merge_chunk_transcriptions(
                        results,
                        total_audio_seconds,
                        callbacks.emit_partial.as_ref(),
                        callbacks.emit_progress_seconds.as_ref(),
                    );
                }
                Err(error)
                    if Self::is_retryable_long_file_memory_error(&error.to_string())
                        && chunk_seconds > LONG_FILE_MIN_COMMIT_WINDOW_SECONDS =>
                {
                    last_error = Some(error);
                    continue;
                }
                Err(error) => return Err(error),
            }
        }
        Err(last_error.unwrap_or_else(|| {
            ApplicationError::SpeechToText(
                "Parakeet long-file transcription failed before producing chunks".to_string(),
            )
        }))
    }

    fn merge_chunk_transcriptions(
        chunks: Vec<(AudioChunk, TranscriptionOutput)>,
        total_audio_seconds: Option<f32>,
        emit_partial: &(dyn Fn(String) + Send + Sync),
        emit_progress_seconds: &(dyn Fn(f32) + Send + Sync),
    ) -> Result<TranscriptionOutput, ApplicationError> {
        let mut cumulative = Vec::new();
        for item in chunks {
            cumulative.push(item);
            if let Some(snapshot) =
                Self::merge_chunk_transcriptions_snapshot(&cumulative, total_audio_seconds)?
            {
                emit_partial(format!("{DELTA_REPLACE_PREFIX}{}", snapshot.text));
            }
            if let Some((chunk, _)) = cumulative.last() {
                emit_progress_seconds(chunk.commit_end_seconds);
            }
        }
        Self::merge_chunk_transcriptions_snapshot(&cumulative, total_audio_seconds)?.ok_or_else(
            || {
                ApplicationError::SpeechToText(
                    "Parakeet long-file transcription produced empty output".to_string(),
                )
            },
        )
    }

    fn merge_chunk_transcriptions_snapshot(
        chunks: &[(AudioChunk, TranscriptionOutput)],
        total_audio_seconds: Option<f32>,
    ) -> Result<Option<TranscriptionOutput>, ApplicationError> {
        // Lossless merge. Chunks overlap by a few seconds, so the same audio is
        // transcribed by two adjacent chunks. Dropping *every* word whose
        // timestamp falls before the previous chunk's end (the old
        // `committed_until` cutoff) loses speech whenever a chunk under-
        // transcribes its tail — the boundary word then exists only in the next
        // chunk and is silently discarded. That is exactly the bug that made
        // whole sentences vanish near every ~5min chunk boundary.
        //
        // Instead we suppress a word only when it is a genuine duplicate of one
        // already committed: same (normalized) text AND a timestamp that overlaps
        // an already-committed word within tolerance. A word in the overlap zone
        // that the previous chunk never produced always survives.
        let mut committed_word_keys: Vec<(String, f32, f32)> = Vec::new();
        let mut merged_segments = Vec::new();

        let mut sorted = chunks.to_vec();
        sorted.sort_by_key(|(chunk, _)| chunk.index);
        for (_chunk, output) in sorted {
            let mut output = output;
            let mut kept_segments = Vec::new();
            for mut segment in output.segments.drain(..) {
                if !segment.words.is_empty() {
                    let language_key = segment
                        .language_code
                        .as_deref()
                        .unwrap_or("und")
                        .trim()
                        .to_ascii_lowercase();
                    segment.words.retain(|word| {
                        let Some(seconds) = word.end_seconds.or(word.start_seconds) else {
                            // No timestamp at all: keep it (rare, never a dup).
                            return true;
                        };
                        let key =
                            format!("{language_key}\u{001f}{}", normalize_word_text(&word.text));
                        let start = word.start_seconds.unwrap_or(seconds);
                        let end = word.end_seconds.unwrap_or(seconds);
                        let is_duplicate = committed_word_keys.iter().any(
                            |(existing_key, existing_start, existing_end)| {
                                existing_key == &key
                                    && start <= existing_end + OVERLAP_DEDUPE_TOLERANCE_SECONDS
                                    && end >= existing_start - OVERLAP_DEDUPE_TOLERANCE_SECONDS
                            },
                        );
                        if is_duplicate {
                            return false;
                        }
                        committed_word_keys.push((key, start, end));
                        true
                    });
                    if segment.words.is_empty() {
                        continue;
                    }
                    segment.start_seconds =
                        segment.words.iter().find_map(|word| word.start_seconds);
                    segment.end_seconds =
                        segment.words.iter().rev().find_map(|word| word.end_seconds);
                    segment.text = segment
                        .words
                        .iter()
                        .map(|word| word.text.trim())
                        .filter(|text| !text.is_empty())
                        .collect::<Vec<_>>()
                        .join(" ");
                }
                let text = segment.text.trim();
                if !text.is_empty() {
                    kept_segments.push(segment);
                }
            }
            merged_segments.extend(kept_segments);
        }

        let text = merged_segments
            .iter()
            .map(|segment| segment.text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if text.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(TranscriptionOutput {
            text: text.clone(),
            segments: normalize_transcript_segments(&text, &merged_segments, total_audio_seconds),
        }))
    }

    fn offset_transcription_output(output: &mut TranscriptionOutput, offset_seconds: f32) {
        for segment in &mut output.segments {
            segment.start_seconds = segment.start_seconds.map(|value| value + offset_seconds);
            segment.end_seconds = segment.end_seconds.map(|value| value + offset_seconds);
            for word in &mut segment.words {
                word.start_seconds = word.start_seconds.map(|value| value + offset_seconds);
                word.end_seconds = word.end_seconds.map(|value| value + offset_seconds);
            }
        }
    }

    fn filter_transcription_to_commit_window(
        output: &mut TranscriptionOutput,
        commit_start_seconds: f32,
        commit_end_seconds: f32,
    ) {
        let tolerance = OVERLAP_DEDUPE_TOLERANCE_SECONDS;
        let mut filtered_segments = Vec::new();

        for mut segment in output.segments.drain(..) {
            if !segment.words.is_empty() {
                segment.words.retain(|word| {
                    let anchor = match (word.start_seconds, word.end_seconds) {
                        (Some(start), Some(end)) => (start + end) * 0.5,
                        (Some(start), None) => start,
                        (None, Some(end)) => end,
                        (None, None) => return true,
                    };
                    anchor >= commit_start_seconds - tolerance
                        && anchor <= commit_end_seconds + tolerance
                });

                if segment.words.is_empty() {
                    continue;
                }

                segment.start_seconds = segment.words.iter().find_map(|word| word.start_seconds);
                segment.end_seconds = segment.words.iter().rev().find_map(|word| word.end_seconds);
                segment.text = segment
                    .words
                    .iter()
                    .map(|word| word.text.trim())
                    .filter(|text| !text.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
            } else {
                let overlaps_commit = match (segment.start_seconds, segment.end_seconds) {
                    (Some(start), Some(end)) => {
                        end >= commit_start_seconds - tolerance
                            && start <= commit_end_seconds + tolerance
                    }
                    (Some(start), None) => {
                        start >= commit_start_seconds - tolerance
                            && start <= commit_end_seconds + tolerance
                    }
                    (None, Some(end)) => {
                        end >= commit_start_seconds - tolerance
                            && end <= commit_end_seconds + tolerance
                    }
                    (None, None) => true,
                };
                if !overlaps_commit {
                    continue;
                }
            }

            if !segment.text.trim().is_empty() {
                filtered_segments.push(segment);
            }
        }

        output.text = filtered_segments
            .iter()
            .map(|segment| segment.text.trim())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        output.segments = filtered_segments;
    }

    fn is_metal_oom_error(message: &str) -> bool {
        let lower = message.to_ascii_lowercase();
        lower.contains("outofmemory")
            || lower.contains("out of memory")
            || lower.contains("backend compute failed")
            || lower.contains("ggml_backend_graph_compute failed")
            || lower.contains("kiogpucommandbuffercallbackerroroutofmemory")
    }

    fn is_retryable_long_file_memory_error(message: &str) -> bool {
        message
            .to_ascii_lowercase()
            .contains("sbobino_parakeet_memory_limit")
            || Self::is_metal_oom_error(message)
    }

    fn parakeet_command_failure(
        prefix: &str,
        stderr: &[u8],
        status: Option<String>,
    ) -> ApplicationError {
        let stderr = String::from_utf8_lossy(stderr);
        if Self::is_metal_oom_error(&stderr) {
            return ApplicationError::SpeechToText(format!(
                "{prefix}: Parakeet Metal ran out of memory on this chunk. The app will retry with smaller chunks when possible."
            ));
        }
        let mut message = stderr
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .filter(|line| {
                !line.starts_with("ggml_metal_library_compile_pipeline")
                    && !line.starts_with("ggml_metal_device_init")
                    && !line.starts_with("ggml_metal_init")
            })
            .take(8)
            .collect::<Vec<_>>()
            .join("\n");
        if message.is_empty() {
            message = status.unwrap_or_else(|| "unknown failure".to_string());
        }
        ApplicationError::SpeechToText(format!("{prefix}: {message}"))
    }
}

#[async_trait]
impl SpeechToTextEngine for ParakeetCppEngine {
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
        if options.translate_to_english {
            return Err(ApplicationError::SpeechToText(
                "Parakeet.cpp does not support translate-to-English mode".to_string(),
            ));
        }

        let model_path = self.validate_model_exists(model_filename)?;
        if Self::should_use_long_file_chunking(input_wav, total_audio_seconds) {
            let mut result = self
                .run_long_file_transcription(
                    input_wav,
                    &model_path,
                    model_filename,
                    language_policy.preferred_language.as_code(),
                    total_audio_seconds,
                    LongFileCallbacks {
                        emit_partial: emit_partial.clone(),
                        emit_progress_seconds: emit_progress_seconds.clone(),
                    },
                )
                .await?;
            if !Self::is_nemotron_streaming_model(model_filename) {
                Self::classify_tdt_output(&mut result);
            }
            emit_partial(format!("{DELTA_REPLACE_PREFIX}{}", result.text));
            if let Some(total) = total_audio_seconds {
                emit_progress_seconds(total);
            }
            return Ok(result);
        }
        let preview_model_path = self.validate_preview_model_exists(
            model_filename,
            language_policy.preferred_language.as_code(),
        )?;
        let preview_state = Arc::new(Mutex::new(PreviewStreamState::default()));
        match tokio::time::timeout(
            PREVIEW_TIMEOUT,
            self.run_chunked_progressive_preview(
                input_wav,
                &preview_model_path,
                preview_state.clone(),
                emit_partial.clone(),
                emit_progress_seconds.clone(),
                language_policy.preferred_language.as_code(),
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

        let mut command = tokio_background_command(&self.binary_path);
        Self::configure_command_environment(&mut command, &self.binary_path);
        command
            .arg("transcribe")
            .arg("--model")
            .arg(&model_path)
            .arg("--input")
            .arg(input_wav)
            .arg("--json");
        if Self::is_nemotron_streaming_model(model_filename) {
            command.arg("--lang").arg(Self::parakeet_target_lang(
                language_policy.preferred_language.as_code(),
            ));
        }
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        command.kill_on_drop(true);

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
        let mut result = Self::parse_json_output(&stdout, total_audio_seconds)?;
        if !Self::is_nemotron_streaming_model(model_filename) {
            Self::classify_tdt_output(&mut result);
        }
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use sbobino_domain::TranscriptionOutput;

    use super::{AudioChunk, ParakeetCppEngine};

    #[test]
    fn parakeet_metal_safety_env_keeps_metal_enabled() {
        let env = ParakeetCppEngine::safe_metal_environment();
        assert!(env.contains(&("GGML_METAL_NO_RESIDENCY", "1")));
        assert!(env.contains(&("GGML_METAL_SHARED_BUFFERS_DISABLE", "1")));
        assert!(env.contains(&("GGML_METAL_CONCURRENCY_DISABLE", "1")));
        assert!(
            !env.iter().any(|(name, _)| *name == "PARAKEET_DEVICE"),
            "Metal safety must not force the CPU backend"
        );
    }

    #[test]
    fn windows_batch_worker_routing_fails_closed_without_cli_fallback() {
        assert!(ParakeetCppEngine::allows_long_file_cli_fallback(false));
        assert!(!ParakeetCppEngine::allows_long_file_cli_fallback(true));
    }

    #[test]
    fn nemotron_language_markers_are_cleaned_and_assigned() {
        let pieces =
            ParakeetCppEngine::parse_language_marked_text("<it-IT>Ciao mondo. <en-US>Hello world.");
        assert_eq!(pieces.len(), 2);
        assert_eq!(
            pieces[0],
            ("Ciao mondo.".to_string(), Some("it".to_string()))
        );
        assert_eq!(
            pieces[1],
            ("Hello world.".to_string(), Some("en".to_string()))
        );
    }

    #[test]
    fn suffix_marker_labels_the_preceding_utterance() {
        let pieces = ParakeetCppEngine::parse_language_marked_text("Ciao mondo. <it-IT>");
        assert_eq!(
            pieces,
            vec![("Ciao mondo.".to_string(), Some("it".to_string()))]
        );

        let pieces = ParakeetCppEngine::parse_language_marked_text("Ciao<it-IT> Hello");
        assert_eq!(
            pieces,
            vec![
                ("Ciao".to_string(), Some("it".to_string())),
                ("Hello".to_string(), None),
            ]
        );
    }

    #[test]
    fn json_output_preserves_multiple_nemotron_language_markers() {
        let output = ParakeetCppEngine::parse_json_output(
            r#"{"text":"<it-IT>Ciao <en-US>Hello"}"#,
            Some(4.0),
        )
        .expect("marker output should parse");
        assert_eq!(output.segments.len(), 2);
        assert_eq!(output.segments[0].language_code.as_deref(), Some("it"));
        assert_eq!(output.segments[1].language_code.as_deref(), Some("en"));
        assert!(!output.text.contains("<it-IT>"));
        assert!(!output.text.contains("<en-US>"));
    }

    #[test]
    fn final_long_file_merge_rejects_all_silent_worker_chunks() {
        let empty_worker_chunk = AudioChunk {
            index: 0,
            path: PathBuf::from("silent-chunk.wav"),
            decode_start_seconds: 0.0,
            decode_end_seconds: 30.0,
            commit_start_seconds: 0.0,
            commit_end_seconds: 30.0,
        };
        let emit_partial = |_line: String| {};
        let emit_progress = |_seconds: f32| {};
        let error = ParakeetCppEngine::merge_chunk_transcriptions(
            vec![(
                empty_worker_chunk,
                TranscriptionOutput {
                    text: String::new(),
                    segments: Vec::new(),
                },
            )],
            Some(30.0),
            &emit_partial,
            &emit_progress,
        )
        .expect_err("all-silent worker coverage must still fail the final transcription");

        assert!(error
            .to_string()
            .contains("Parakeet long-file transcription produced empty output"));
    }
}
