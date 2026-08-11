use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex as StdMutex, OnceLock,
};
use std::time::Duration;

use chrono::Utc;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{Emitter, State};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::{
    error::CommandError,
    release_assets::{
        release_asset_url, release_tag, PyannoteReleaseAsset, PyannoteReleaseManifest,
        ReleaseAssetDescriptor, RuntimeReleaseAsset, RuntimeReleaseManifest, SetupReleaseManifest,
        PYANNOTE_COMPAT_LEVEL, PYANNOTE_MANIFEST_ASSET, PYANNOTE_MODEL_ASSET,
        PYANNOTE_RUNTIME_AARCH64_ASSET, PYANNOTE_RUNTIME_WINDOWS_X86_64_ASSET,
        PYANNOTE_RUNTIME_X86_64_ASSET, RUNTIME_AARCH64_ASSET, RUNTIME_MANIFEST_ASSET,
        RUNTIME_WINDOWS_X86_64_ASSET, RUNTIME_X86_64_ASSET, SETUP_MANIFEST_ASSET,
    },
    state::AppState,
};
use sbobino_domain::TranscriptionEngine;
use sbobino_infrastructure::{
    background_process::tokio_background_command, ManagedPyannoteManifest, ManagedRuntimeHealth,
    ReconcileManagedPyannoteReleaseOutcome, RuntimeTranscriptionFactory,
    PYANNOTE_MANIFEST_FILENAME,
};

const REQUIRED_MODELS: [&str; 5] = [
    "ggml-tiny.bin",
    "ggml-base.bin",
    "ggml-small.bin",
    "ggml-medium.bin",
    "ggml-large-v3-turbo-q8_0.bin",
];

const COREML_ENCODERS: [(&str, &str); 5] = [
    (
        "ggml-tiny-encoder.mlmodelc",
        "ggml-tiny-encoder.mlmodelc.zip",
    ),
    (
        "ggml-base-encoder.mlmodelc",
        "ggml-base-encoder.mlmodelc.zip",
    ),
    (
        "ggml-small-encoder.mlmodelc",
        "ggml-small-encoder.mlmodelc.zip",
    ),
    (
        "ggml-medium-encoder.mlmodelc",
        "ggml-medium-encoder.mlmodelc.zip",
    ),
    (
        "ggml-large-v3-turbo-encoder.mlmodelc",
        "ggml-large-v3-turbo-encoder.mlmodelc.zip",
    ),
];

const MODEL_CATALOG: [(&str, &str, &str, &str, &str); 5] = [
    (
        "tiny",
        "Tiny",
        "ggml-tiny.bin",
        "ggml-tiny-encoder.mlmodelc",
        "ggml-tiny-encoder.mlmodelc.zip",
    ),
    (
        "base",
        "Base",
        "ggml-base.bin",
        "ggml-base-encoder.mlmodelc",
        "ggml-base-encoder.mlmodelc.zip",
    ),
    (
        "small",
        "Small",
        "ggml-small.bin",
        "ggml-small-encoder.mlmodelc",
        "ggml-small-encoder.mlmodelc.zip",
    ),
    (
        "medium",
        "Medium",
        "ggml-medium.bin",
        "ggml-medium-encoder.mlmodelc",
        "ggml-medium-encoder.mlmodelc.zip",
    ),
    (
        "large_turbo",
        "Large Turbo",
        "ggml-large-v3-turbo-q8_0.bin",
        "ggml-large-v3-turbo-encoder.mlmodelc",
        "ggml-large-v3-turbo-encoder.mlmodelc.zip",
    ),
];

const MODEL_BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/";
const PARAKEET_MODEL_BASE_URL: &str =
    "https://huggingface.co/mudler/parakeet-cpp-gguf/resolve/main/";
const PARAKEET_MODEL_CATALOG: [(&str, &str, &str); 2] = [
    (
        "tdt06b_v3_q4",
        "Parakeet TDT 0.6B Q4 — file only",
        "tdt-0.6b-v3-q4_k.gguf",
    ),
    (
        "nemotron35_asr_streaming_06b_q4",
        "NVIDIA Nemotron 3.5 ASR 0.6B Q4 — live + multilingual",
        "nemotron-3.5-asr-streaming-0.6b-q4_k.gguf",
    ),
];
const LOCAL_RELEASE_ASSETS_DIR_ENV: &str = "SBOBINO_LOCAL_RELEASE_ASSETS_DIR";
const MODEL_DOWNLOAD_MAX_ATTEMPTS: usize = 3;
const MODEL_DOWNLOAD_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
const MODEL_DOWNLOAD_CHUNK_TIMEOUT: Duration = Duration::from_secs(60);
const MODEL_DOWNLOAD_RETRY_DELAY: Duration = Duration::from_millis(500);
const RELEASE_HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const RELEASE_HTTP_TOTAL_TIMEOUT: Duration = Duration::from_secs(120);
const PYANNOTE_IMPORT_LOAD_TIMEOUT: Duration = Duration::from_secs(90);
static DOWNLOAD_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);
static PROVISIONING_SWAP_COUNTER: AtomicU64 = AtomicU64::new(0);
// Runtime publication moves the `bin` and `lib` directories independently.
// Health/recovery commands must not inspect or remove the journal, stage, or
// backup while that pair is being moved.  The provisioning slot serialises
// normal app operations; this process-wide lock also covers synchronous
// readiness calls and the blocking extraction worker.
static RUNTIME_INSTALL_TRANSACTION_LOCK: OnceLock<StdMutex<()>> = OnceLock::new();

type ProvisioningSlot = Arc<Mutex<Option<CancellationToken>>>;

pub(crate) fn runtime_install_transaction_lock() -> &'static StdMutex<()> {
    RUNTIME_INSTALL_TRANSACTION_LOCK.get_or_init(|| StdMutex::new(()))
}

fn provisioning_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(RELEASE_HTTP_CONNECT_TIMEOUT)
        .timeout(RELEASE_HTTP_TOTAL_TIMEOUT)
        .build()
        .expect("release asset HTTP client configuration is valid")
}

const PYANNOTE_PYTHON_ENV_VARS_TO_CLEAR: &[&str] = &[
    "PYTHONPATH",
    "PYTHONEXECUTABLE",
    "PYTHONHOME",
    "PYTHONNOUSERSITE",
    "PYTHONUSERBASE",
    "PYTHONSTARTUP",
    "PYTHONPLATLIBDIR",
    "PYTHONPYCACHEPREFIX",
    "PYTHONBREAKPOINT",
    "__PYVENV_LAUNCHER__",
    "VIRTUAL_ENV",
    "CONDA_PREFIX",
    "CONDA_DEFAULT_ENV",
];

struct ProvisioningSlotGuard {
    slot: ProvisioningSlot,
}

struct ProvisioningDownloadBatch {
    models_dir: PathBuf,
    missing_models: Vec<String>,
    missing_encoders: Vec<(String, String)>,
    model_base_url: &'static str,
    model_asset_kind: &'static str,
}

impl Drop for ProvisioningSlotGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.slot.try_lock() {
            *active = None;
            return;
        }

        let slot = self.slot.clone();
        tauri::async_runtime::spawn(async move {
            *slot.lock().await = None;
        });
    }
}

async fn acquire_provisioning_slot(
    slot: ProvisioningSlot,
) -> Result<(CancellationToken, ProvisioningSlotGuard), CommandError> {
    let mut active = slot.lock().await;
    if active.is_some() {
        return Err(CommandError::new(
            "provisioning_busy",
            "Another provisioning operation is already running. Wait for it to finish or cancel it before starting a new one.",
        ));
    }

    let cancel_token = CancellationToken::new();
    *active = Some(cancel_token.clone());
    drop(active);

    Ok((cancel_token, ProvisioningSlotGuard { slot }))
}

#[derive(Debug, Clone)]
struct PyannoteAssetSelection {
    runtime_asset: PyannoteReleaseAsset,
    model_asset: PyannoteReleaseAsset,
    compat_level: u32,
    release_version: String,
}

#[derive(Debug, Clone)]
struct RuntimeAssetSelection {
    runtime_asset: RuntimeReleaseAsset,
    release_version: String,
}

#[derive(Debug, Clone)]
struct SetupReleaseBundle {
    setup_manifest: SetupReleaseManifest,
    runtime_manifest: RuntimeReleaseManifest,
    pyannote_manifest: PyannoteReleaseManifest,
    release_version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProvisioningStatusEvent {
    pub state: String,
    pub message: String,
    pub reason_code: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProvisioningStatusResponse {
    pub ready: bool,
    pub models_dir: String,
    pub missing_models: Vec<String>,
    pub missing_encoders: Vec<String>,
    pub pyannote: sbobino_infrastructure::PyannoteRuntimeHealth,
}

#[derive(Debug, Serialize)]
pub struct PostUpdateReconcileResponse {
    pub status: String,
    pub migration_started: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PyannoteBackgroundActionTrigger {
    Startup,
    PostUpdate,
    EnableDiarization,
    JobRequiresDiarization,
}

#[derive(Debug, Clone, Serialize)]
pub struct PyannoteBackgroundActionResponse {
    pub status: String,
    pub should_start: bool,
    pub force_reinstall: bool,
    pub reason_code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProvisioningProgressEvent {
    pub current: usize,
    pub total: usize,
    pub asset: String,
    pub asset_kind: String,
    pub stage: String,
    pub percentage: u8,
}

#[derive(Debug, Deserialize)]
pub struct ProvisioningStartPayload {
    pub include_coreml: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ProvisioningDownloadModelPayload {
    pub model: String,
    pub include_coreml: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ProvisioningInstallPyannotePayload {
    pub force: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ProvisioningInstallRuntimePayload {
    pub force: Option<bool>,
}

#[derive(Debug, Serialize)]
pub struct ProvisioningStartResponse {
    pub started: bool,
}

const PYANNOTE_INSTALL_HEADROOM_BYTES: u64 = 128 * 1024 * 1024;

fn normalize_pyannote_compat_level(level: u32) -> u32 {
    if level == 0 {
        PYANNOTE_COMPAT_LEVEL
    } else {
        level
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = UNITS[0];
    for next_unit in UNITS.iter().skip(1) {
        if value < 1024.0 {
            break;
        }
        value /= 1024.0;
        unit = next_unit;
    }
    if unit == "B" {
        format!("{bytes} {unit}")
    } else {
        format!("{value:.1} {unit}")
    }
}

trait ReleaseAssetSizeExt {
    fn size_bytes(&self) -> Option<u64>;
    fn expanded_size_bytes(&self) -> Option<u64>;
}

impl ReleaseAssetSizeExt for ReleaseAssetDescriptor {
    fn size_bytes(&self) -> Option<u64> {
        self.size_bytes
    }

    fn expanded_size_bytes(&self) -> Option<u64> {
        self.expanded_size_bytes
    }
}

impl ReleaseAssetSizeExt for RuntimeReleaseAsset {
    fn size_bytes(&self) -> Option<u64> {
        self.size_bytes
    }

    fn expanded_size_bytes(&self) -> Option<u64> {
        self.expanded_size_bytes
    }
}

impl ReleaseAssetSizeExt for PyannoteReleaseAsset {
    fn size_bytes(&self) -> Option<u64> {
        self.size_bytes
    }

    fn expanded_size_bytes(&self) -> Option<u64> {
        self.expanded_size_bytes
    }
}

fn descriptor_bytes_or_zero(descriptor: &impl ReleaseAssetSizeExt) -> u64 {
    descriptor.size_bytes().unwrap_or(0)
}

fn descriptor_expanded_bytes_or_zero(descriptor: &impl ReleaseAssetSizeExt) -> u64 {
    descriptor.expanded_size_bytes().unwrap_or(0)
}

fn estimate_pyannote_required_free_bytes(selection: &PyannoteAssetSelection) -> u64 {
    descriptor_bytes_or_zero(&selection.runtime_asset)
        + descriptor_bytes_or_zero(&selection.model_asset)
        + descriptor_expanded_bytes_or_zero(&selection.runtime_asset)
        + descriptor_expanded_bytes_or_zero(&selection.model_asset)
        + PYANNOTE_INSTALL_HEADROOM_BYTES
}

#[cfg(unix)]
fn available_disk_space_bytes(path: &Path) -> Result<u64, String> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| format!("invalid path for disk space check: '{}'", path.display()))?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    let result = unsafe { libc::statvfs(c_path.as_ptr(), stats.as_mut_ptr()) };
    if result != 0 {
        return Err(format!(
            "failed to inspect available disk space at '{}': {}",
            path.display(),
            std::io::Error::last_os_error()
        ));
    }
    let stats = unsafe { stats.assume_init() };
    Ok((stats.f_bavail as u64).saturating_mul(stats.f_frsize))
}

#[cfg(not(unix))]
fn available_disk_space_bytes(_path: &Path) -> Result<u64, String> {
    Ok(u64::MAX)
}

fn ensure_pyannote_install_has_free_space(
    runtime_dir: &Path,
    selection: &PyannoteAssetSelection,
) -> Result<(), String> {
    let required = estimate_pyannote_required_free_bytes(selection);
    if required == PYANNOTE_INSTALL_HEADROOM_BYTES {
        return Ok(());
    }

    let available = available_disk_space_bytes(runtime_dir)?;
    if available >= required {
        return Ok(());
    }

    Err(format!(
        "Pyannote install needs about {} of free disk space but only {} is available near '{}'. Install it later from Settings > Local Models after freeing some space.",
        format_bytes(required),
        format_bytes(available),
        runtime_dir.display()
    ))
}

fn cleanup_pyannote_workdir(runtime_factory: &RuntimeTranscriptionFactory) -> Result<(), String> {
    let runtime_dir = runtime_factory.managed_pyannote_runtime_dir();
    let python_dir = runtime_factory.managed_pyannote_python_dir();
    let model_dir = runtime_factory.managed_pyannote_model_dir();
    let manifest_path = runtime_factory.managed_pyannote_manifest_path();

    remove_path_if_exists(&python_dir)?;
    remove_path_if_exists(&model_dir)?;
    remove_path_if_exists(&manifest_path)?;

    if runtime_dir.is_dir() {
        for entry in std::fs::read_dir(&runtime_dir)
            .map_err(|e| format!("failed to inspect pyannote runtime directory: {e}"))?
        {
            let entry =
                entry.map_err(|e| format!("failed to inspect pyannote runtime entry: {e}"))?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(".download-") || name.starts_with(".stage-") {
                remove_path_if_exists(&path)?;
            }
        }
    }

    Ok(())
}

fn persist_pyannote_install_failure(
    runtime_factory: &RuntimeTranscriptionFactory,
    had_ready_install: bool,
    reason_code: &str,
    message: &str,
) {
    if had_ready_install {
        // A failed repair must never make an already usable installation look
        // unhealthy.  Keep the last successful status/manifest intact and
        // record the repair diagnostic separately for Settings to surface.
        let runtime_dir = runtime_factory.managed_pyannote_runtime_dir();
        let diagnostic_path = runtime_dir.join("last-install-failure.json");
        let diagnostic_tmp = provisioning_swap_path(&runtime_dir, "pyannote-failure");
        let diagnostic = serde_json::json!({
            "reason_code": reason_code.trim(),
            "message": message.trim(),
            "updated_at": Utc::now().to_rfc3339(),
        });
        match serde_json::to_vec_pretty(&diagnostic)
            .map_err(|error| format!("failed to serialize pyannote install diagnostic: {error}"))
            .and_then(|body| {
                std::fs::create_dir_all(&runtime_dir).map_err(|error| {
                    format!(
                        "failed to create pyannote runtime directory '{}': {error}",
                        runtime_dir.display()
                    )
                })?;
                std::fs::write(&diagnostic_tmp, body).map_err(|error| {
                    format!(
                        "failed to write pyannote install diagnostic '{}': {error}",
                        diagnostic_tmp.display()
                    )
                })?;
                std::fs::rename(&diagnostic_tmp, &diagnostic_path).map_err(|error| {
                    let _ = remove_path_if_exists(&diagnostic_tmp);
                    format!(
                        "failed to publish pyannote install diagnostic '{}': {error}",
                        diagnostic_path.display()
                    )
                })
            }) {
            Ok(()) => {}
            Err(error) => tracing::warn!("failed to persist pyannote repair diagnostic: {error}"),
        }
        return;
    }

    if let Err(error) = cleanup_pyannote_workdir(runtime_factory) {
        tracing::warn!("failed to clean up incomplete pyannote install: {error}");
    }
    if let Err(error) = runtime_factory.write_managed_pyannote_status(reason_code, message) {
        tracing::warn!("failed to persist pyannote failure status: {error}");
    }
}

fn prepare_pyannote_runtime_swap(
    runtime_dir: &Path,
    reset_existing_install: bool,
) -> Result<Option<PathBuf>, String> {
    if !reset_existing_install || !runtime_dir.is_dir() {
        return Ok(None);
    }

    let parent = runtime_dir.parent().ok_or_else(|| {
        format!(
            "failed to determine parent directory for '{}'.",
            runtime_dir.display()
        )
    })?;
    let backup_dir = parent.join(format!(
        ".pyannote-backup-{}",
        Utc::now().timestamp_millis()
    ));
    std::fs::rename(runtime_dir, &backup_dir).map_err(|e| {
        format!(
            "failed to stage existing pyannote runtime '{}' into backup '{}': {e}",
            runtime_dir.display(),
            backup_dir.display()
        )
    })?;
    Ok(Some(backup_dir))
}

fn rollback_pyannote_runtime_swap(
    runtime_dir: &Path,
    backup_dir: Option<&Path>,
) -> Result<(), String> {
    let Some(backup_dir) = backup_dir else {
        return Ok(());
    };

    if runtime_dir.exists() {
        remove_path_if_exists(runtime_dir)?;
    }
    std::fs::rename(backup_dir, runtime_dir).map_err(|e| {
        format!(
            "failed to restore pyannote runtime backup '{}' into '{}': {e}",
            backup_dir.display(),
            runtime_dir.display()
        )
    })
}

fn cleanup_pyannote_runtime_backup(backup_dir: Option<PathBuf>) -> Result<(), String> {
    let Some(backup_dir) = backup_dir else {
        return Ok(());
    };

    remove_path_if_exists(&backup_dir)
}

fn prepare_pyannote_runtime_stage(runtime_dir: &Path) -> Result<PathBuf, String> {
    let parent = runtime_dir.parent().ok_or_else(|| {
        format!(
            "failed to determine parent directory for '{}'.",
            runtime_dir.display()
        )
    })?;
    let stage_dir = parent.join(format!(".pyannote-stage-{}", Utc::now().timestamp_millis()));
    std::fs::create_dir_all(&stage_dir).map_err(|e| {
        format!(
            "failed to create pyannote staging directory '{}': {e}",
            stage_dir.display()
        )
    })?;
    Ok(stage_dir)
}

fn cleanup_pyannote_runtime_stage(stage_dir: &Path) -> Result<(), String> {
    remove_path_if_exists(stage_dir)
}

fn write_staged_pyannote_manifest(
    stage_dir: &Path,
    manifest: &ManagedPyannoteManifest,
) -> Result<(), String> {
    let body = serde_json::to_string_pretty(manifest)
        .map_err(|e| format!("failed to serialize pyannote manifest: {e}"))?;
    std::fs::write(stage_dir.join(PYANNOTE_MANIFEST_FILENAME), body).map_err(|e| {
        format!(
            "failed to write staged pyannote manifest in '{}': {e}",
            stage_dir.display()
        )
    })
}

fn promote_staged_pyannote_runtime(
    runtime_dir: &Path,
    stage_dir: &Path,
    reset_existing_install: bool,
) -> Result<Option<PathBuf>, String> {
    let should_swap_existing_runtime = reset_existing_install || runtime_dir.is_dir();
    let backup_runtime_dir =
        prepare_pyannote_runtime_swap(runtime_dir, should_swap_existing_runtime)?;

    if runtime_dir.exists() {
        remove_path_if_exists(runtime_dir)?;
    }

    if let Err(error) = std::fs::rename(stage_dir, runtime_dir) {
        let rollback_error =
            rollback_pyannote_runtime_swap(runtime_dir, backup_runtime_dir.as_deref()).err();
        let rollback_note = rollback_error.map_or_else(
            || String::from("previous runtime restored from backup"),
            |rollback| format!("failed to restore previous runtime backup: {rollback}"),
        );
        return Err(format!(
            "failed to promote staged pyannote runtime '{}' into '{}': {error}; {rollback_note}",
            stage_dir.display(),
            runtime_dir.display()
        ));
    }

    Ok(backup_runtime_dir)
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SetupReportStepPayload {
    pub id: String,
    pub label: String,
    pub status: String,
    pub detail: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct WriteSetupReportPayload {
    pub privacy_accepted: bool,
    pub setup_complete: bool,
    pub final_reason_code: Option<String>,
    pub final_error: Option<String>,
    pub runtime_health: Option<serde_json::Value>,
    pub steps: Vec<SetupReportStepPayload>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReadSetupReportResponse {
    pub build_version: String,
    pub privacy_accepted: bool,
    pub setup_complete: bool,
    pub final_reason_code: Option<String>,
    pub final_error: Option<String>,
    pub runtime_health: Option<serde_json::Value>,
    pub steps: Vec<SetupReportStepPayload>,
    pub updated_at: String,
    #[serde(default)]
    pub trusted_for_fast_start: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProvisioningModelCatalogEntry {
    pub key: String,
    pub label: String,
    pub model_file: String,
    pub installed: bool,
    pub coreml_installed: bool,
    pub engine: String,
    pub experimental: bool,
}

fn format_managed_runtime_install_error(health: &ManagedRuntimeHealth) -> String {
    let failing_tool = if !health.ffmpeg.available {
        Some(("FFmpeg", &health.ffmpeg))
    } else if !health.whisper_cli.available {
        Some(("Whisper CLI", &health.whisper_cli))
    } else if !health.whisper_stream.available {
        Some(("Whisper Stream", &health.whisper_stream))
    } else if !health.parakeet_cli.available {
        Some(("Parakeet CLI", &health.parakeet_cli))
    } else {
        None
    };

    if let Some((label, tool)) = failing_tool {
        let detail = if tool.failure_message.trim().is_empty() {
            "Managed runtime verification failed.".to_string()
        } else {
            tool.failure_message.trim().to_string()
        };
        return format!(
            "{label} could not be verified at '{}': {detail}",
            tool.resolved_path
        );
    }

    "Local runtime was installed but is still not runnable.".to_string()
}

fn transcription_runtime_install_complete(health: &sbobino_infrastructure::RuntimeHealth) -> bool {
    if health.managed_runtime_required {
        // All release binaries (ffmpeg, whisper CLI, whisper stream,
        // parakeet CLI) must be present regardless of which engine is
        // selected, so the user can switch engines at runtime without
        // re-running setup. v2.0.4 commit 4d0a61c made this a single
        // check; the ff5c896 merge reverted to per-engine checks.
        return health.managed_runtime.ffmpeg.available
            && health.managed_runtime.whisper_cli.available
            && health.managed_runtime.whisper_stream.available
            && health.managed_runtime.parakeet_cli.available;
    }

    match health.configured_engine {
        TranscriptionEngine::WhisperCpp => {
            health.ffmpeg_available
                && health.whisper_cli_available
                && health.whisper_stream_available
        }
        TranscriptionEngine::ParakeetCpp => {
            health.ffmpeg_available && health.parakeet_cli_available
        }
    }
}

fn collect_missing_models(models_dir: &Path) -> Vec<String> {
    REQUIRED_MODELS
        .iter()
        .filter_map(|filename| {
            let path = models_dir.join(filename);
            if path.exists() {
                None
            } else {
                Some((*filename).to_string())
            }
        })
        .collect::<Vec<_>>()
}

fn collect_missing_encoders(models_dir: &Path) -> Vec<String> {
    COREML_ENCODERS
        .iter()
        .filter_map(|(dir_name, _archive)| {
            let path = models_dir.join(dir_name);
            if path.is_dir() {
                None
            } else {
                Some((*dir_name).to_string())
            }
        })
        .collect::<Vec<_>>()
}

fn coreml_missing_for(models_dir: &Path, dir_name: &str) -> bool {
    !models_dir.join(dir_name).is_dir()
}

#[tauri::command]
pub async fn provisioning_status(
    state: State<'_, AppState>,
) -> Result<ProvisioningStatusResponse, CommandError> {
    let settings = state
        .runtime_factory
        .load_settings()
        .map_err(|e| CommandError::new("settings", e))?;

    let models_dir_value = if settings.transcription.models_dir.trim().is_empty() {
        settings.models_dir
    } else {
        settings.transcription.models_dir
    };
    let models_dir = PathBuf::from(state.runtime_factory.resolve_models_dir(&models_dir_value));
    let runtime_health = state
        .runtime_factory
        .runtime_health_preflight()
        .map_err(|e| CommandError::new("runtime_health", e))?;

    let missing_models = collect_missing_models(&models_dir);
    let missing_encoders = collect_missing_encoders(&models_dir);

    Ok(ProvisioningStatusResponse {
        ready: missing_models.is_empty() && missing_encoders.is_empty(),
        models_dir: models_dir.to_string_lossy().to_string(),
        missing_models,
        missing_encoders,
        pyannote: runtime_health.pyannote,
    })
}

#[tauri::command]
pub async fn provisioning_models(
    state: State<'_, AppState>,
) -> Result<Vec<ProvisioningModelCatalogEntry>, CommandError> {
    let settings = state
        .runtime_factory
        .load_settings()
        .map_err(|e| CommandError::new("settings", e))?;

    let models_dir_value = if settings.transcription.models_dir.trim().is_empty() {
        settings.models_dir
    } else {
        settings.transcription.models_dir
    };
    let models_dir = PathBuf::from(state.runtime_factory.resolve_models_dir(&models_dir_value));

    let parakeet_models_dir = PathBuf::from(
        state
            .runtime_factory
            .resolve_models_dir(&settings.transcription.parakeet_models_dir),
    );

    let mut entries = MODEL_CATALOG
        .iter()
        .map(|(key, label, model_file, encoder_dir, _encoder_archive)| {
            ProvisioningModelCatalogEntry {
                key: (*key).to_string(),
                label: (*label).to_string(),
                model_file: (*model_file).to_string(),
                installed: models_dir.join(model_file).exists(),
                coreml_installed: models_dir.join(encoder_dir).is_dir(),
                engine: "whisper_cpp".to_string(),
                experimental: false,
            }
        })
        .collect::<Vec<_>>();

    entries.extend(
        PARAKEET_MODEL_CATALOG
            .iter()
            .map(|(key, label, model_file)| ProvisioningModelCatalogEntry {
                key: (*key).to_string(),
                label: (*label).to_string(),
                model_file: (*model_file).to_string(),
                installed: parakeet_models_dir.join(model_file).exists(),
                coreml_installed: false,
                engine: "parakeet_cpp".to_string(),
                experimental: false,
            }),
    );

    Ok(entries)
}

fn pyannote_background_action_response(
    status: &str,
    should_start: bool,
    force_reinstall: bool,
    reason_code: &str,
    message: impl Into<String>,
) -> PyannoteBackgroundActionResponse {
    PyannoteBackgroundActionResponse {
        status: status.to_string(),
        should_start,
        force_reinstall,
        reason_code: reason_code.trim().to_string(),
        message: message.into(),
    }
}

fn should_attempt_post_update_pyannote_reconcile(
    manifest_before: Option<&ManagedPyannoteManifest>,
) -> bool {
    manifest_before
        .as_ref()
        .map(|manifest| {
            manifest.source != "bundled_override"
                && (manifest.app_version.trim() != env!("CARGO_PKG_VERSION")
                    || normalize_pyannote_compat_level(manifest.compat_level)
                        != PYANNOTE_COMPAT_LEVEL)
        })
        .unwrap_or(false)
}

fn infer_pyannote_reconcile_reason_code(message: &str) -> &'static str {
    let normalized = message.trim().to_ascii_lowercase();
    if normalized.contains("compatibility mismatch") {
        "pyannote_version_mismatch"
    } else if normalized.contains("checksum") {
        "pyannote_checksum_invalid"
    } else {
        "pyannote_repair_required"
    }
}

fn pyannote_reconcile_action(
    outcome: ReconcileManagedPyannoteReleaseOutcome,
) -> Option<PyannoteBackgroundActionResponse> {
    match outcome {
        ReconcileManagedPyannoteReleaseOutcome::NoAction => None,
        ReconcileManagedPyannoteReleaseOutcome::ManifestUpdated => {
            Some(pyannote_background_action_response(
                "migrate_manifest",
                false,
                false,
                "pyannote_manifest_migrated",
                "Pyannote metadata was updated for this app version.",
            ))
        }
        ReconcileManagedPyannoteReleaseOutcome::NeedsMigration { message } => {
            let reason_code = infer_pyannote_reconcile_reason_code(&message);
            Some(pyannote_background_action_response(
                "migrate_assets",
                true,
                true,
                reason_code,
                message,
            ))
        }
    }
}

async fn plan_pyannote_background_action_inner(
    runtime_factory: &std::sync::Arc<RuntimeTranscriptionFactory>,
    trigger: PyannoteBackgroundActionTrigger,
) -> Result<PyannoteBackgroundActionResponse, CommandError> {
    let manifest_before = runtime_factory.read_managed_pyannote_manifest();
    let status_before = runtime_factory.read_managed_pyannote_status();

    if !matches!(trigger, PyannoteBackgroundActionTrigger::PostUpdate) {
        return Ok(pyannote_background_action_response(
            "none",
            false,
            false,
            "pyannote_auto_check_disabled",
            "Pyannote is validated only during installation, repair, and app updates.",
        ));
    }

    if matches!(trigger, PyannoteBackgroundActionTrigger::PostUpdate)
        && should_attempt_post_update_pyannote_reconcile(manifest_before.as_ref())
    {
        let client = provisioning_http_client();
        if let Ok(selection) = fetch_pyannote_asset_selection(&client).await {
            let outcome = runtime_factory
                .reconcile_managed_pyannote_release_assets(
                    &selection.release_version,
                    selection.compat_level,
                    &selection.runtime_asset.name,
                    &selection.runtime_asset.sha256,
                    &selection.model_asset.name,
                    &selection.model_asset.sha256,
                )
                .map_err(|e| CommandError::new("plan_pyannote_background_action", e))?;
            if let Some(action) = pyannote_reconcile_action(outcome) {
                return Ok(action);
            }
        }
    }

    let health = runtime_factory
        .runtime_health()
        .map_err(|e| CommandError::new("plan_pyannote_background_action", e))?;

    if health.pyannote.ready {
        return Ok(pyannote_background_action_response(
            "none",
            false,
            false,
            "ok",
            "Pyannote diarization runtime is ready.",
        ));
    }

    let reason_code = health.pyannote.reason_code.trim();
    let message = health.pyannote.message.clone();
    let has_existing_pyannote_state = health.pyannote.runtime_installed
        || health.pyannote.model_installed
        || manifest_before.is_some()
        || status_before.is_some();

    if has_existing_pyannote_state {
        if matches!(
            reason_code,
            "pyannote_version_mismatch" | "pyannote_checksum_invalid"
        ) {
            return Ok(pyannote_background_action_response(
                "migrate_assets",
                true,
                true,
                reason_code,
                message,
            ));
        }

        if is_pyannote_repair_reason(reason_code)
            || matches!(
                reason_code,
                "pyannote_runtime_missing" | "pyannote_model_missing"
            )
        {
            return Ok(pyannote_background_action_response(
                "repair_existing",
                true,
                true,
                if reason_code.is_empty() {
                    "pyannote_repair_required"
                } else {
                    reason_code
                },
                message,
            ));
        }
    }

    if health.pyannote.enabled
        && matches!(
            reason_code,
            "" | "pyannote_runtime_missing" | "pyannote_model_missing"
        )
    {
        return Ok(pyannote_background_action_response(
            "install_missing",
            true,
            false,
            if reason_code.is_empty() {
                "pyannote_runtime_missing"
            } else {
                reason_code
            },
            if message.trim().is_empty() {
                "Pyannote diarization runtime is not installed yet.".to_string()
            } else {
                message
            },
        ));
    }

    if health.pyannote.enabled && is_pyannote_repair_reason(reason_code) {
        return Ok(pyannote_background_action_response(
            "repair_existing",
            true,
            true,
            reason_code,
            message,
        ));
    }

    Ok(pyannote_background_action_response(
        "none",
        false,
        false,
        if health.pyannote.enabled {
            reason_code
        } else {
            "pyannote_disabled"
        },
        if health.pyannote.enabled {
            message
        } else {
            "Speaker diarization is disabled, so pyannote does not need background work right now."
                .to_string()
        },
    ))
}

#[tauri::command]
pub async fn plan_pyannote_background_action(
    state: State<'_, AppState>,
    trigger: PyannoteBackgroundActionTrigger,
) -> Result<PyannoteBackgroundActionResponse, CommandError> {
    plan_pyannote_background_action_inner(&state.runtime_factory, trigger).await
}

#[tauri::command]
pub async fn reconcile_post_update_runtime(
    state: State<'_, AppState>,
) -> Result<PostUpdateReconcileResponse, CommandError> {
    let action = plan_pyannote_background_action_inner(
        &state.runtime_factory,
        PyannoteBackgroundActionTrigger::PostUpdate,
    )
    .await?;

    let response = match action.status.as_str() {
        "migrate_manifest" => PostUpdateReconcileResponse {
            status: "ok_migrated_manifest".to_string(),
            migration_started: false,
            message: Some(action.message),
        },
        "install_missing" | "repair_existing" | "migrate_assets" => PostUpdateReconcileResponse {
            status: "needs_auto_migration".to_string(),
            migration_started: false,
            message: Some(action.message),
        },
        _ => PostUpdateReconcileResponse {
            status: "ok_no_action".to_string(),
            migration_started: false,
            message: None,
        },
    };

    Ok(response)
}

#[tauri::command]
pub async fn provisioning_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: Option<ProvisioningStartPayload>,
) -> Result<ProvisioningStartResponse, CommandError> {
    let include_coreml = payload
        .and_then(|value| value.include_coreml)
        .unwrap_or(true);

    let settings = state
        .runtime_factory
        .load_settings()
        .map_err(|e| CommandError::new("settings", e))?;

    let models_dir_value = if settings.transcription.models_dir.trim().is_empty() {
        settings.models_dir
    } else {
        settings.transcription.models_dir
    };
    let models_dir = PathBuf::from(state.runtime_factory.resolve_models_dir(&models_dir_value));

    tokio::fs::create_dir_all(&models_dir).await.map_err(|e| {
        CommandError::new("provisioning", format!("failed to create models dir: {e}"))
    })?;

    let missing_models = collect_missing_models(&models_dir);

    let missing_encoders = if include_coreml {
        COREML_ENCODERS
            .iter()
            .filter_map(|(dir_name, archive)| {
                let path = models_dir.join(dir_name);
                if path.is_dir() {
                    None
                } else {
                    Some(((*dir_name).to_string(), (*archive).to_string()))
                }
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let total = missing_models.len() + missing_encoders.len();
    if total == 0 {
        emit_provisioning_status(
            &app,
            "completed",
            "All required models are already available.",
            None,
        );
        return Ok(ProvisioningStartResponse { started: false });
    }

    let (cancel_token, slot_guard) =
        acquire_provisioning_slot(state.provisioning.cancel_token.clone()).await?;

    spawn_provisioning_download(
        app,
        ProvisioningDownloadBatch {
            models_dir,
            missing_models,
            missing_encoders,
            model_base_url: MODEL_BASE_URL,
            model_asset_kind: "whisper_model",
        },
        cancel_token,
        slot_guard,
    );

    Ok(ProvisioningStartResponse { started: true })
}

#[tauri::command]
pub async fn provisioning_download_model(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: ProvisioningDownloadModelPayload,
) -> Result<ProvisioningStartResponse, CommandError> {
    let include_coreml = payload.include_coreml.unwrap_or(true);

    let settings = state
        .runtime_factory
        .load_settings()
        .map_err(|e| CommandError::new("settings", e))?;

    let models_dir_value = if settings.transcription.models_dir.trim().is_empty() {
        settings.models_dir
    } else {
        settings.transcription.models_dir
    };
    let models_dir = PathBuf::from(state.runtime_factory.resolve_models_dir(&models_dir_value));

    tokio::fs::create_dir_all(&models_dir).await.map_err(|e| {
        CommandError::new("provisioning", format!("failed to create models dir: {e}"))
    })?;

    if let Some((_, label, model_file)) = PARAKEET_MODEL_CATALOG
        .iter()
        .find(|(key, _, _)| *key == payload.model)
    {
        let parakeet_models_dir = PathBuf::from(
            state
                .runtime_factory
                .resolve_models_dir(&settings.transcription.parakeet_models_dir),
        );
        tokio::fs::create_dir_all(&parakeet_models_dir)
            .await
            .map_err(|e| {
                CommandError::new(
                    "provisioning",
                    format!("failed to create Parakeet models dir: {e}"),
                )
            })?;

        if parakeet_models_dir.join(model_file).exists() {
            emit_provisioning_status(
                &app,
                "completed",
                &format!("{label} is already available."),
                None,
            );
            return Ok(ProvisioningStartResponse { started: false });
        }

        let (cancel_token, slot_guard) =
            acquire_provisioning_slot(state.provisioning.cancel_token.clone()).await?;
        spawn_provisioning_download(
            app,
            ProvisioningDownloadBatch {
                models_dir: parakeet_models_dir,
                missing_models: vec![(*model_file).to_string()],
                missing_encoders: Vec::new(),
                model_base_url: PARAKEET_MODEL_BASE_URL,
                model_asset_kind: "parakeet_model",
            },
            cancel_token,
            slot_guard,
        );
        return Ok(ProvisioningStartResponse { started: true });
    }

    let Some((_, label, model_file, encoder_dir, encoder_archive)) = MODEL_CATALOG
        .iter()
        .find(|(key, _, _, _, _)| *key == payload.model)
    else {
        return Err(CommandError::new(
            "validation",
            format!("unknown model key: {}", payload.model),
        ));
    };

    let mut missing_models = Vec::new();
    if !models_dir.join(model_file).exists() {
        missing_models.push((*model_file).to_string());
    }

    let mut missing_encoders = Vec::new();
    if include_coreml && coreml_missing_for(&models_dir, encoder_dir) {
        missing_encoders.push(((*encoder_dir).to_string(), (*encoder_archive).to_string()));
    }

    let total = missing_models.len() + missing_encoders.len();
    if total == 0 {
        emit_provisioning_status(
            &app,
            "completed",
            &format!("{label} is already available."),
            None,
        );
        return Ok(ProvisioningStartResponse { started: false });
    }

    let (cancel_token, slot_guard) =
        acquire_provisioning_slot(state.provisioning.cancel_token.clone()).await?;

    spawn_provisioning_download(
        app,
        ProvisioningDownloadBatch {
            models_dir,
            missing_models,
            missing_encoders,
            model_base_url: MODEL_BASE_URL,
            model_asset_kind: "whisper_model",
        },
        cancel_token,
        slot_guard,
    );

    Ok(ProvisioningStartResponse { started: true })
}

#[tauri::command]
pub async fn provisioning_install_pyannote(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: Option<ProvisioningInstallPyannotePayload>,
) -> Result<ProvisioningStartResponse, CommandError> {
    let force = payload.and_then(|value| value.force).unwrap_or(false);
    let health = state
        .runtime_factory
        .runtime_health()
        .map_err(|e| CommandError::new("runtime_health", e))?;
    let repair_required = force || is_pyannote_repair_reason(&health.pyannote.reason_code);

    if health.pyannote.ready && !repair_required {
        emit_provisioning_status(
            &app,
            "completed",
            "Pyannote diarization runtime is already installed.",
            None,
        );
        return Ok(ProvisioningStartResponse { started: false });
    }

    let (cancel_token, slot_guard) =
        acquire_provisioning_slot(state.provisioning.cancel_token.clone()).await?;

    if state.runtime_factory.has_bundled_pyannote_override_assets() {
        spawn_pyannote_bundled_install(
            app,
            state.runtime_factory.clone(),
            cancel_token,
            slot_guard,
            health.pyannote.ready,
            repair_required,
        );
    } else {
        spawn_pyannote_provisioning_download(
            app,
            state.runtime_factory.clone(),
            cancel_token,
            slot_guard,
            health.pyannote.ready,
            repair_required,
        );
    }

    Ok(ProvisioningStartResponse { started: true })
}

#[tauri::command]
pub async fn provisioning_install_runtime(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: Option<ProvisioningInstallRuntimePayload>,
) -> Result<ProvisioningStartResponse, CommandError> {
    let force = payload.and_then(|value| value.force).unwrap_or(false);
    let health = state
        .runtime_factory
        .runtime_health()
        .map_err(|e| CommandError::new("runtime_health", e))?;
    let runtime_ready = transcription_runtime_install_complete(&health);

    if runtime_ready && !force {
        emit_provisioning_status(
            &app,
            "completed",
            "Local transcription runtime is already installed.",
            None,
        );
        return Ok(ProvisioningStartResponse { started: false });
    }

    let (cancel_token, slot_guard) =
        acquire_provisioning_slot(state.provisioning.cancel_token.clone()).await?;

    spawn_runtime_provisioning_download(
        app,
        state.runtime_factory.clone(),
        cancel_token,
        slot_guard,
    );

    Ok(ProvisioningStartResponse { started: true })
}

#[tauri::command]
pub async fn provisioning_cancel(state: State<'_, AppState>) -> Result<(), CommandError> {
    let token = {
        let guard = state.provisioning.cancel_token.lock().await;
        guard.clone()
    };

    if let Some(token) = token {
        token.cancel();
    }

    Ok(())
}

#[tauri::command]
pub async fn write_setup_report(
    state: State<'_, AppState>,
    payload: WriteSetupReportPayload,
) -> Result<(), CommandError> {
    let report_path = state.runtime_factory.data_dir().join("setup-report.json");
    let report = serde_json::json!({
        "build_version": env!("CARGO_PKG_VERSION"),
        "privacy_accepted": payload.privacy_accepted,
        "setup_complete": payload.setup_complete,
        "final_reason_code": payload.final_reason_code,
        "final_error": payload.final_error,
        "runtime_health": payload.runtime_health,
        "steps": payload.steps,
        "updated_at": Utc::now().to_rfc3339(),
    });
    let body = serde_json::to_string_pretty(&report).map_err(|e| {
        CommandError::new(
            "setup_report",
            format!("failed to serialize setup report: {e}"),
        )
    })?;
    tokio::fs::write(&report_path, body).await.map_err(|e| {
        CommandError::new(
            "setup_report",
            format!(
                "failed to write setup report '{}': {e}",
                report_path.display()
            ),
        )
    })?;
    Ok(())
}

#[tauri::command]
pub async fn read_setup_report(
    state: State<'_, AppState>,
) -> Result<Option<ReadSetupReportResponse>, CommandError> {
    let report_path = state.runtime_factory.data_dir().join("setup-report.json");
    let body = match tokio::fs::read_to_string(&report_path).await {
        Ok(body) => body,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(CommandError::new(
                "setup_report",
                format!(
                    "failed to read setup report '{}': {error}",
                    report_path.display()
                ),
            ))
        }
    };

    let mut report: ReadSetupReportResponse = serde_json::from_str(&body).map_err(|error| {
        CommandError::new(
            "setup_report",
            format!(
                "failed to parse setup report '{}': {error}",
                report_path.display()
            ),
        )
    })?;
    report.trusted_for_fast_start = report.build_version.trim() == env!("CARGO_PKG_VERSION")
        && report.privacy_accepted
        && report.setup_complete
        && report
            .final_error
            .as_deref()
            .unwrap_or("")
            .trim()
            .is_empty()
        && report.final_reason_code.as_deref() == Some("setup_complete");
    Ok(Some(report))
}

fn spawn_provisioning_download(
    app: tauri::AppHandle,
    batch: ProvisioningDownloadBatch,
    cancel_token: CancellationToken,
    slot_guard: ProvisioningSlotGuard,
) {
    let total = batch.missing_models.len() + batch.missing_encoders.len();

    tauri::async_runtime::spawn(async move {
        let _slot_guard = slot_guard;
        let ProvisioningDownloadBatch {
            models_dir,
            missing_models,
            missing_encoders,
            model_base_url,
            model_asset_kind,
        } = batch;
        let client = provisioning_http_client();
        let mut current = 0usize;

        let mut emit_progress = |asset: String, asset_kind: &str, stage: String| {
            current += 1;
            let percentage = ((current as f32 / total as f32) * 100.0).round() as u8;
            let _ = app.emit(
                "provisioning://progress",
                ProvisioningProgressEvent {
                    current,
                    total,
                    asset,
                    asset_kind: asset_kind.to_string(),
                    stage,
                    percentage,
                },
            );
        };

        for model in missing_models {
            if cancel_token.is_cancelled() {
                emit_provisioning_status(
                    &app,
                    "cancelled",
                    "Provisioning cancelled.",
                    Some("cancelled"),
                );
                return;
            }

            let url = format!("{model_base_url}{model}");
            let destination = models_dir.join(&model);
            match download_to_path(&client, &url, &destination, &cancel_token).await {
                Ok(()) => emit_progress(model, model_asset_kind, "downloaded".to_string()),
                Err(error) => {
                    if error == "cancelled" {
                        emit_provisioning_status(
                            &app,
                            "cancelled",
                            "Provisioning cancelled.",
                            Some("cancelled"),
                        );
                        return;
                    }
                    emit_provisioning_status(
                        &app,
                        "error",
                        &format!("Provisioning failed: {error}"),
                        Some("download_failed"),
                    );
                    return;
                }
            }
        }

        for (encoder_dir, archive) in missing_encoders {
            if cancel_token.is_cancelled() {
                emit_provisioning_status(
                    &app,
                    "cancelled",
                    "Provisioning cancelled.",
                    Some("cancelled"),
                );
                return;
            }

            let url = format!("{MODEL_BASE_URL}{archive}");
            let archive_path = models_dir.join(&archive);

            match download_to_path(&client, &url, &archive_path, &cancel_token).await {
                Ok(()) => {}
                Err(error) => {
                    if error == "cancelled" {
                        emit_provisioning_status(
                            &app,
                            "cancelled",
                            "Provisioning cancelled.",
                            Some("cancelled"),
                        );
                        return;
                    }
                    emit_provisioning_status(
                        &app,
                        "error",
                        &format!("Failed to download {encoder_dir}: {error}"),
                        Some("download_failed"),
                    );
                    return;
                }
            }

            let extraction = tokio::task::spawn_blocking({
                let archive_path = archive_path.clone();
                let models_dir = models_dir.clone();
                let encoder_dir = encoder_dir.clone();
                move || install_coreml_encoder_archive(&archive_path, &models_dir, &encoder_dir)
            })
            .await;

            if let Err(error) = cleanup_downloaded_archive(&archive_path).await {
                emit_provisioning_status(
                    &app,
                    "error",
                    &format!("Failed to clean up {encoder_dir} archive: {error}"),
                    Some("archive_cleanup_failed"),
                );
                return;
            }

            match extraction {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    emit_provisioning_status(
                        &app,
                        "error",
                        &format!("Failed to extract {encoder_dir}: {error}"),
                        Some("extract_failed"),
                    );
                    return;
                }
                Err(error) => {
                    emit_provisioning_status(
                        &app,
                        "error",
                        &format!("Failed to extract {encoder_dir}: task join error: {error}"),
                        Some("extract_failed"),
                    );
                    return;
                }
            }
            emit_progress(encoder_dir, "whisper_encoder", "downloaded".to_string());
        }

        emit_provisioning_status(
            &app,
            "completed",
            "Provisioning completed successfully.",
            None,
        );
    });
}

fn emit_provisioning_status(
    app: &tauri::AppHandle,
    state: &str,
    message: &str,
    reason_code: Option<&str>,
) {
    let _ = app.emit(
        "provisioning://status",
        ProvisioningStatusEvent {
            state: state.to_string(),
            message: message.to_string(),
            reason_code: reason_code.map(|value| value.to_string()),
        },
    );
}

fn managed_pyannote_python_executable(
    runtime_factory: &RuntimeTranscriptionFactory,
) -> Option<PathBuf> {
    let python_dir = runtime_factory.managed_pyannote_python_dir();
    #[cfg(target_os = "windows")]
    let candidates = [
        python_dir.join("python.exe"),
        python_dir.join("python3.exe"),
        python_dir.join("bin").join("python3"),
        python_dir.join("bin").join("python"),
    ];
    #[cfg(not(target_os = "windows"))]
    let candidates = [
        python_dir.join("bin").join("python3"),
        python_dir.join("bin").join("python"),
    ];
    candidates.into_iter().find(|candidate| candidate.is_file())
}

fn managed_pyannote_python_path_env(python_root: &Path) -> Option<std::ffi::OsString> {
    #[cfg(target_os = "windows")]
    let entries = [
        python_root.join("Lib"),
        python_root.join("DLLs"),
        python_root.join("Lib").join("site-packages"),
    ];
    #[cfg(not(target_os = "windows"))]
    let entries = {
        let version_dir = std::fs::read_dir(python_root.join("lib"))
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.is_dir()
                    && path
                        .file_name()
                        .and_then(|value| value.to_str())
                        .is_some_and(|value| value.starts_with("python3."))
            })?;
        [
            version_dir.clone(),
            version_dir.join("lib-dynload"),
            version_dir.join("site-packages"),
        ]
    };

    let entries = entries
        .into_iter()
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    (!entries.is_empty())
        .then(|| std::env::join_paths(entries).ok())
        .flatten()
}

pub(crate) async fn probe_pyannote_import_and_load(
    runtime_factory: &RuntimeTranscriptionFactory,
    cancel_token: &CancellationToken,
) -> Result<(), String> {
    if cancel_token.is_cancelled() {
        return Err("cancelled".to_string());
    }

    let python_root = runtime_factory.managed_pyannote_python_dir();
    let python_path = managed_pyannote_python_executable(runtime_factory).ok_or_else(|| {
        "Pyannote import probe could not find the managed Python executable.".to_string()
    })?;
    let model_dir = runtime_factory.managed_pyannote_model_dir();
    if !model_dir.is_dir() {
        return Err(format!(
            "Pyannote import probe could not find the managed model at '{}'.",
            model_dir.display()
        ));
    }

    let probe_script = r#"
import sys
from pyannote.audio import Pipeline
Pipeline.from_pretrained(sys.argv[1])
print("pyannote-import-load-ok")
"#;
    let mut command = tokio_background_command(&python_path);
    command
        .arg("-c")
        .arg(probe_script)
        .arg(&model_dir)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    for key in PYANNOTE_PYTHON_ENV_VARS_TO_CLEAR {
        command.env_remove(key);
    }
    command.env("PYTHONHOME", &python_root);
    if let Some(path) = managed_pyannote_python_path_env(&python_root) {
        command.env("PYTHONPATH", path);
    }
    let mut path_entries = vec![
        python_path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| python_root.join("bin")),
        runtime_factory.data_dir().join("bin"),
    ];
    if let Some(existing) = std::env::var_os("PATH") {
        path_entries.extend(std::env::split_paths(&existing));
    }
    if let Ok(path) = std::env::join_paths(path_entries) {
        command.env("PATH", path);
    }
    #[cfg(target_os = "macos")]
    {
        let mut library_entries = vec![runtime_factory.data_dir().join("lib")];
        let embedded = python_root.join("lib").join("embedded-dylibs");
        if embedded.is_dir() {
            library_entries.push(embedded);
        }
        if let Some(existing) = std::env::var_os("DYLD_LIBRARY_PATH") {
            library_entries.extend(std::env::split_paths(&existing));
        }
        if let Ok(path) = std::env::join_paths(library_entries) {
            command
                .env("DYLD_LIBRARY_PATH", &path)
                .env("DYLD_FALLBACK_LIBRARY_PATH", path);
        }
    }

    let child = command
        .spawn()
        .map_err(|error| format!("failed to start Pyannote import/load probe: {error}"))?;
    let output = tokio::select! {
        _ = cancel_token.cancelled() => return Err("cancelled".to_string()),
        result = tokio::time::timeout(PYANNOTE_IMPORT_LOAD_TIMEOUT, child.wait_with_output()) => {
            result
                .map_err(|_| format!("Pyannote import/load probe timed out after {} seconds.", PYANNOTE_IMPORT_LOAD_TIMEOUT.as_secs()))?
                .map_err(|error| format!("Pyannote import/load probe could not be collected: {error}"))?
        }
    };
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if stderr.is_empty() { stdout } else { stderr };
    let detail = if detail.len() > 600 {
        format!("{}…", detail.chars().take(600).collect::<String>())
    } else {
        detail
    };
    Err(if detail.is_empty() {
        format!(
            "Pyannote import/load probe exited with status {}.",
            output.status
        )
    } else {
        format!("Pyannote import/load probe failed: {detail}")
    })
}

fn spawn_pyannote_bundled_install(
    app: tauri::AppHandle,
    runtime_factory: std::sync::Arc<RuntimeTranscriptionFactory>,
    cancel_token: CancellationToken,
    slot_guard: ProvisioningSlotGuard,
    had_ready_install: bool,
    _repair_required: bool,
) {
    tauri::async_runtime::spawn(async move {
        let _slot_guard = slot_guard;
        if cancel_token.is_cancelled() {
            emit_provisioning_status(
                &app,
                "cancelled",
                "Pyannote installation cancelled.",
                Some("cancelled"),
            );
            persist_pyannote_install_failure(
                runtime_factory.as_ref(),
                had_ready_install,
                "cancelled",
                "Pyannote installation was cancelled before completion.",
            );
            return;
        }

        let runtime_dir = runtime_factory.managed_pyannote_runtime_dir();
        let backup_runtime_dir = if had_ready_install {
            match prepare_pyannote_runtime_swap(&runtime_dir, true) {
                Ok(value) => value,
                Err(error) => {
                    emit_provisioning_status(
                        &app,
                        "error",
                        &error,
                        Some("pyannote_install_incomplete"),
                    );
                    persist_pyannote_install_failure(
                        runtime_factory.as_ref(),
                        had_ready_install,
                        "pyannote_install_incomplete",
                        &error,
                    );
                    return;
                }
            }
        } else {
            None
        };

        let install_result = runtime_factory
            .reinstall_managed_pyannote_from_bundled_override()
            .and_then(|installed| {
                if installed {
                    Ok(())
                } else {
                    Err("Bundled pyannote runtime is not available.".to_string())
                }
            });

        if let Err(error) = install_result {
            if let Err(restore_error) =
                rollback_pyannote_runtime_swap(&runtime_dir, backup_runtime_dir.as_deref())
            {
                tracing::warn!("failed to rollback bundled pyannote runtime: {restore_error}");
            }
            emit_provisioning_status(&app, "error", &error, Some("pyannote_repair_required"));
            persist_pyannote_install_failure(
                runtime_factory.as_ref(),
                had_ready_install,
                "pyannote_repair_required",
                &error,
            );
            return;
        }

        let _ = app.emit(
            "provisioning://progress",
            ProvisioningProgressEvent {
                current: 1,
                total: 2,
                asset: "bundled-pyannote-runtime".to_string(),
                asset_kind: "pyannote_runtime".to_string(),
                stage: "installed".to_string(),
                percentage: 50,
            },
        );
        let _ = app.emit(
            "provisioning://progress",
            ProvisioningProgressEvent {
                current: 2,
                total: 2,
                asset: "bundled-pyannote-model".to_string(),
                asset_kind: "pyannote_model".to_string(),
                stage: "installed".to_string(),
                percentage: 100,
            },
        );

        match runtime_factory.validate_managed_pyannote_runtime() {
            Ok(()) => {
                if let Err(error) =
                    probe_pyannote_import_and_load(&runtime_factory, &cancel_token).await
                {
                    let reason_code = if error == "cancelled" {
                        "cancelled"
                    } else {
                        "pyannote_import_load_failed"
                    };
                    if let Err(restore_error) =
                        rollback_pyannote_runtime_swap(&runtime_dir, backup_runtime_dir.as_deref())
                    {
                        tracing::warn!(
                            "failed to rollback bundled pyannote runtime after probe error: {restore_error}"
                        );
                    }
                    emit_provisioning_status(&app, "error", &error, Some(reason_code));
                    persist_pyannote_install_failure(
                        runtime_factory.as_ref(),
                        had_ready_install,
                        reason_code,
                        &error,
                    );
                    return;
                }
                if let Err(error) = runtime_factory
                    .write_managed_pyannote_status("ok", "Pyannote diarization runtime is ready.")
                {
                    emit_provisioning_status(
                        &app,
                        "error",
                        &error,
                        Some("pyannote_install_incomplete"),
                    );
                    if let Err(restore_error) =
                        rollback_pyannote_runtime_swap(&runtime_dir, backup_runtime_dir.as_deref())
                    {
                        tracing::warn!(
                            "failed to rollback bundled pyannote runtime after status error: {restore_error}"
                        );
                    }
                    persist_pyannote_install_failure(
                        runtime_factory.as_ref(),
                        had_ready_install,
                        "pyannote_install_incomplete",
                        &error,
                    );
                    return;
                }
                emit_provisioning_status(
                    &app,
                    "completed",
                    "Pyannote diarization runtime installed successfully.",
                    None,
                );
                if let Err(cleanup_error) = cleanup_pyannote_runtime_backup(backup_runtime_dir) {
                    tracing::warn!(
                        "failed to clean up bundled pyannote runtime backup: {cleanup_error}"
                    );
                }
            }
            Err(error) => {
                if let Err(restore_error) =
                    rollback_pyannote_runtime_swap(&runtime_dir, backup_runtime_dir.as_deref())
                {
                    tracing::warn!(
                        "failed to rollback bundled pyannote runtime after validation error: {restore_error}"
                    );
                }
                emit_provisioning_status(&app, "error", &error, Some("pyannote_repair_required"));
                persist_pyannote_install_failure(
                    runtime_factory.as_ref(),
                    had_ready_install,
                    "pyannote_repair_required",
                    &error,
                )
            }
        }
    });
}

fn spawn_pyannote_provisioning_download(
    app: tauri::AppHandle,
    runtime_factory: std::sync::Arc<RuntimeTranscriptionFactory>,
    cancel_token: CancellationToken,
    slot_guard: ProvisioningSlotGuard,
    had_ready_install: bool,
    reset_existing_install: bool,
) {
    tauri::async_runtime::spawn(async move {
        let _slot_guard = slot_guard;
        let client = provisioning_http_client();
        let total = 2usize;
        let runtime_dir = runtime_factory.managed_pyannote_runtime_dir();
        let stage_dir = match prepare_pyannote_runtime_stage(&runtime_dir) {
            Ok(value) => value,
            Err(error) => {
                emit_provisioning_status(
                    &app,
                    "error",
                    &error,
                    Some("pyannote_install_incomplete"),
                );
                persist_pyannote_install_failure(
                    runtime_factory.as_ref(),
                    had_ready_install,
                    "pyannote_install_incomplete",
                    &error,
                );
                return;
            }
        };

        let selection = match fetch_pyannote_asset_selection(&client).await {
            Ok(value) => value,
            Err(error) => {
                emit_provisioning_status(
                    &app,
                    "error",
                    &error,
                    Some("pyannote_install_incomplete"),
                );
                if let Err(cleanup_error) = cleanup_pyannote_runtime_stage(&stage_dir) {
                    tracing::warn!(
                        "failed to clean up pyannote runtime stage after selection error: {cleanup_error}"
                    );
                }
                persist_pyannote_install_failure(
                    runtime_factory.as_ref(),
                    had_ready_install,
                    "pyannote_install_incomplete",
                    &error,
                );
                return;
            }
        };

        if let Err(error) = ensure_pyannote_install_has_free_space(&stage_dir, &selection) {
            emit_provisioning_status(&app, "error", &error, Some("pyannote_install_incomplete"));
            if let Err(cleanup_error) = cleanup_pyannote_runtime_stage(&stage_dir) {
                tracing::warn!(
                    "failed to clean up pyannote runtime stage after disk-space check: {cleanup_error}"
                );
            }
            persist_pyannote_install_failure(
                runtime_factory.as_ref(),
                had_ready_install,
                "pyannote_install_incomplete",
                &error,
            );
            return;
        }

        let downloads = vec![
            (
                selection.runtime_asset.clone(),
                "pyannote_runtime",
                "python",
                stage_dir.join("python"),
            ),
            (
                selection.model_asset.clone(),
                "pyannote_model",
                "model",
                stage_dir.join("model"),
            ),
        ];

        let mut completed = 0usize;
        for (asset, asset_kind, expected_root, destination) in downloads {
            if cancel_token.is_cancelled() {
                emit_provisioning_status(
                    &app,
                    "cancelled",
                    "Pyannote installation cancelled.",
                    Some("cancelled"),
                );
                if let Err(cleanup_error) = cleanup_pyannote_runtime_stage(&stage_dir) {
                    tracing::warn!(
                        "failed to clean up pyannote runtime stage after cancellation: {cleanup_error}"
                    );
                }
                persist_pyannote_install_failure(
                    runtime_factory.as_ref(),
                    had_ready_install,
                    "pyannote_install_incomplete",
                    "Pyannote installation was cancelled before completion.",
                );
                return;
            }

            let archive_path = stage_dir.join(format!(".download-{}", asset.name));
            if let Err(error) = stage_release_asset(
                &client,
                &selection.release_version,
                &asset.name,
                &archive_path,
                &cancel_token,
            )
            .await
            {
                let _ = tokio::fs::remove_file(&archive_path).await;
                if error == "cancelled" {
                    emit_provisioning_status(
                        &app,
                        "cancelled",
                        "Pyannote installation cancelled.",
                        Some("cancelled"),
                    );
                    if let Err(cleanup_error) = cleanup_pyannote_runtime_stage(&stage_dir) {
                        tracing::warn!(
                            "failed to clean up pyannote runtime stage after download cancellation: {cleanup_error}"
                        );
                    }
                    if !had_ready_install {
                        let _ = runtime_factory.write_managed_pyannote_status(
                            "pyannote_install_incomplete",
                            "Pyannote installation was cancelled before completion.",
                        );
                    }
                    return;
                }
                emit_provisioning_status(
                    &app,
                    "error",
                    &format!("Failed to download {}: {error}", asset.name),
                    Some("pyannote_install_incomplete"),
                );
                if let Err(cleanup_error) = cleanup_pyannote_runtime_stage(&stage_dir) {
                    tracing::warn!(
                        "failed to clean up pyannote runtime stage after download error: {cleanup_error}"
                    );
                }
                persist_pyannote_install_failure(
                    runtime_factory.as_ref(),
                    had_ready_install,
                    "pyannote_install_incomplete",
                    &format!("Failed to download {}: {error}", asset.name),
                );
                return;
            }

            match verify_file_sha256(&archive_path, &asset.sha256) {
                Ok(()) => {}
                Err(error) => {
                    let _ = tokio::fs::remove_file(&archive_path).await;
                    emit_provisioning_status(
                        &app,
                        "error",
                        &error,
                        Some("pyannote_checksum_invalid"),
                    );
                    if let Err(cleanup_error) = cleanup_pyannote_runtime_stage(&stage_dir) {
                        tracing::warn!(
                            "failed to clean up pyannote runtime stage after checksum error: {cleanup_error}"
                        );
                    }
                    persist_pyannote_install_failure(
                        runtime_factory.as_ref(),
                        had_ready_install,
                        "pyannote_checksum_invalid",
                        &error,
                    );
                    return;
                }
            }

            let extraction = tokio::task::spawn_blocking({
                let archive_path = archive_path.clone();
                let stage_dir = stage_dir.clone();
                let destination = destination.clone();
                let expected_root = expected_root.to_string();
                move || {
                    install_pyannote_archive(
                        &archive_path,
                        &stage_dir,
                        &expected_root,
                        &destination,
                    )
                }
            })
            .await;

            let _ = tokio::fs::remove_file(&archive_path).await;

            match extraction {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    emit_provisioning_status(
                        &app,
                        "error",
                        &error,
                        Some("pyannote_install_incomplete"),
                    );
                    if let Err(cleanup_error) = cleanup_pyannote_runtime_stage(&stage_dir) {
                        tracing::warn!(
                            "failed to clean up pyannote runtime stage after extraction error: {cleanup_error}"
                        );
                    }
                    persist_pyannote_install_failure(
                        runtime_factory.as_ref(),
                        had_ready_install,
                        "pyannote_install_incomplete",
                        &error,
                    );
                    return;
                }
                Err(error) => {
                    let message =
                        format!("Failed to install {}: task join error: {error}", asset.name);
                    emit_provisioning_status(
                        &app,
                        "error",
                        &message,
                        Some("pyannote_install_incomplete"),
                    );
                    if let Err(cleanup_error) = cleanup_pyannote_runtime_stage(&stage_dir) {
                        tracing::warn!(
                            "failed to clean up pyannote runtime stage after extraction task failure: {cleanup_error}"
                        );
                    }
                    persist_pyannote_install_failure(
                        runtime_factory.as_ref(),
                        had_ready_install,
                        "pyannote_install_incomplete",
                        &message,
                    );
                    return;
                }
            }

            completed += 1;
            let percentage = ((completed as f32 / total as f32) * 100.0).round() as u8;
            let _ = app.emit(
                "provisioning://progress",
                ProvisioningProgressEvent {
                    current: completed,
                    total,
                    asset: asset.name.clone(),
                    asset_kind: asset_kind.to_string(),
                    stage: "installed".to_string(),
                    percentage,
                },
            );
        }

        let manifest = ManagedPyannoteManifest {
            source: "release_asset".to_string(),
            app_version: selection.release_version,
            compat_level: selection.compat_level,
            runtime_asset: selection.runtime_asset.name.clone(),
            runtime_sha256: selection.runtime_asset.sha256.clone(),
            model_asset: selection.model_asset.name.clone(),
            model_sha256: selection.model_asset.sha256.clone(),
            runtime_arch: host_pyannote_arch_label().to_string(),
            installed_at: Utc::now().to_rfc3339(),
        };

        if let Err(error) = write_staged_pyannote_manifest(&stage_dir, &manifest) {
            emit_provisioning_status(&app, "error", &error, Some("pyannote_install_incomplete"));
            if let Err(cleanup_error) = cleanup_pyannote_runtime_stage(&stage_dir) {
                tracing::warn!(
                    "failed to clean up pyannote runtime stage after staged manifest error: {cleanup_error}"
                );
            }
            persist_pyannote_install_failure(
                runtime_factory.as_ref(),
                had_ready_install,
                "pyannote_install_incomplete",
                &error,
            );
            return;
        }

        let backup_runtime_dir = match promote_staged_pyannote_runtime(
            &runtime_dir,
            &stage_dir,
            reset_existing_install,
        ) {
            Ok(value) => value,
            Err(error) => {
                emit_provisioning_status(
                    &app,
                    "error",
                    &error,
                    Some("pyannote_install_incomplete"),
                );
                if let Err(cleanup_error) = cleanup_pyannote_runtime_stage(&stage_dir) {
                    tracing::warn!(
                            "failed to clean up pyannote runtime stage after promotion error: {cleanup_error}"
                        );
                }
                persist_pyannote_install_failure(
                    runtime_factory.as_ref(),
                    had_ready_install,
                    "pyannote_install_incomplete",
                    &error,
                );
                return;
            }
        };

        match runtime_factory.validate_managed_pyannote_runtime() {
            Ok(()) => {
                if let Err(error) =
                    probe_pyannote_import_and_load(&runtime_factory, &cancel_token).await
                {
                    let reason_code = if error == "cancelled" {
                        "cancelled"
                    } else {
                        "pyannote_import_load_failed"
                    };
                    emit_provisioning_status(&app, "error", &error, Some(reason_code));
                    if let Err(restore_error) =
                        rollback_pyannote_runtime_swap(&runtime_dir, backup_runtime_dir.as_deref())
                    {
                        tracing::warn!("failed to rollback pyannote runtime after import/load probe error: {restore_error}");
                    }
                    persist_pyannote_install_failure(
                        runtime_factory.as_ref(),
                        had_ready_install,
                        reason_code,
                        &error,
                    );
                    return;
                }
                if let Err(error) = runtime_factory
                    .write_managed_pyannote_status("ok", "Pyannote diarization runtime is ready.")
                {
                    emit_provisioning_status(
                        &app,
                        "error",
                        &error,
                        Some("pyannote_install_incomplete"),
                    );
                    if let Err(restore_error) =
                        rollback_pyannote_runtime_swap(&runtime_dir, backup_runtime_dir.as_deref())
                    {
                        tracing::warn!("failed to rollback pyannote runtime after status write error: {restore_error}");
                    }
                    persist_pyannote_install_failure(
                        runtime_factory.as_ref(),
                        had_ready_install,
                        "pyannote_install_incomplete",
                        &error,
                    );
                    return;
                }
                emit_provisioning_status(
                    &app,
                    "completed",
                    "Pyannote diarization runtime installed successfully.",
                    None,
                );
                if let Err(cleanup_error) = cleanup_pyannote_runtime_backup(backup_runtime_dir) {
                    tracing::warn!("failed to clean up pyannote runtime backup: {cleanup_error}");
                }
            }
            Err(error) => {
                emit_provisioning_status(
                    &app,
                    "error",
                    &error,
                    Some("pyannote_install_incomplete"),
                );
                if let Err(restore_error) =
                    rollback_pyannote_runtime_swap(&runtime_dir, backup_runtime_dir.as_deref())
                {
                    tracing::warn!("failed to rollback pyannote runtime after runtime-health error: {restore_error}");
                }
                persist_pyannote_install_failure(
                    runtime_factory.as_ref(),
                    had_ready_install,
                    "pyannote_install_incomplete",
                    &error,
                );
            }
        }
    });
}

fn spawn_runtime_provisioning_download(
    app: tauri::AppHandle,
    runtime_factory: std::sync::Arc<RuntimeTranscriptionFactory>,
    cancel_token: CancellationToken,
    slot_guard: ProvisioningSlotGuard,
) {
    tauri::async_runtime::spawn(async move {
        let _slot_guard = slot_guard;
        let client = provisioning_http_client();
        let selection = match fetch_runtime_asset_selection(&client).await {
            Ok(value) => value,
            Err(error) => {
                emit_provisioning_status(&app, "error", &error, Some("runtime_install_incomplete"));
                return;
            }
        };

        let data_dir = runtime_factory.data_dir().to_path_buf();
        let runtime_dir = data_dir.join("runtime");
        let destination = data_dir.join("bin");

        if let Err(error) = tokio::fs::create_dir_all(&runtime_dir).await {
            emit_provisioning_status(
                &app,
                "error",
                &format!("Failed to create runtime directory: {error}"),
                Some("runtime_install_incomplete"),
            );
            return;
        }

        let asset = selection.runtime_asset;
        let archive_path = runtime_dir.join(format!(".download-{}", asset.name));

        if let Err(error) = stage_release_asset(
            &client,
            &selection.release_version,
            &asset.name,
            &archive_path,
            &cancel_token,
        )
        .await
        {
            let _ = tokio::fs::remove_file(&archive_path).await;
            if error == "cancelled" {
                emit_provisioning_status(
                    &app,
                    "cancelled",
                    "Local runtime installation cancelled.",
                    Some("cancelled"),
                );
                return;
            }
            emit_provisioning_status(
                &app,
                "error",
                &format!("Failed to download {}: {error}", asset.name),
                Some("runtime_install_incomplete"),
            );
            return;
        }

        match verify_file_sha256(&archive_path, &asset.sha256) {
            Ok(()) => {}
            Err(error) => {
                let _ = tokio::fs::remove_file(&archive_path).await;
                emit_provisioning_status(&app, "error", &error, Some("runtime_checksum_invalid"));
                return;
            }
        }

        let extraction = tokio::task::spawn_blocking({
            let archive_path = archive_path.clone();
            let runtime_dir = runtime_dir.clone();
            let destination = destination.clone();
            move || install_runtime_archive(&archive_path, &runtime_dir, &destination)
        })
        .await;

        let _ = tokio::fs::remove_file(&archive_path).await;

        let transaction = match extraction {
            Ok(Ok(transaction)) => transaction,
            Ok(Err(error)) => {
                emit_provisioning_status(&app, "error", &error, Some("runtime_install_incomplete"));
                return;
            }
            Err(error) => {
                emit_provisioning_status(
                    &app,
                    "error",
                    &format!("Failed to install local runtime: task join error: {error}"),
                    Some("runtime_install_incomplete"),
                );
                return;
            }
        };

        let managed_runtime = runtime_factory.managed_runtime_health();
        if !managed_runtime.ready {
            let rollback = transaction.clone();
            let _ = tokio::task::spawn_blocking(move || {
                rollback_runtime_install_transaction(&rollback)
            })
            .await;
            emit_provisioning_status(
                &app,
                "error",
                &format_managed_runtime_install_error(&managed_runtime),
                Some("runtime_install_incomplete"),
            );
            return;
        }

        if let Err(error) = commit_runtime_install_transaction(&transaction) {
            let message = match error {
                RuntimeInstallCommitError::BeforeValidationMarker(error) => {
                    // A failed marker write leaves an unvalidated journal;
                    // restore the previous runtime before reporting the
                    // incomplete install.  Do not claim that restoration if
                    // the rollback itself fails.
                    let rollback = transaction.clone();
                    let rollback_result = tokio::task::spawn_blocking(move || {
                        rollback_runtime_install_transaction(&rollback)
                    })
                    .await;
                    match rollback_result {
                        Ok(Ok(())) => format!(
                            "Failed to finalize the local transcription runtime: {error}. Previous runtime restored."
                        ),
                        Ok(Err(rollback_error)) => format!(
                            "Failed to finalize the local transcription runtime: {error}. Automatic rollback failed: {rollback_error}. Repair the runtime from Settings > Local Models."
                        ),
                        Err(join_error) => format!(
                            "Failed to finalize the local transcription runtime: {error}. Automatic rollback task failed: {join_error}. Repair the runtime from Settings > Local Models."
                        ),
                    }
                }
                RuntimeInstallCommitError::AfterValidationMarker(error) => format!(
                    "The local transcription runtime was validated, but cleanup is incomplete: {error}. The validated runtime remains active; the install journal is retained for safe cleanup. Repair the runtime from Settings > Local Models."
                ),
            };
            tracing::error!("{message}");
            emit_provisioning_status(&app, "error", &message, Some("runtime_install_incomplete"));
            return;
        }

        let _ = app.emit(
            "provisioning://progress",
            ProvisioningProgressEvent {
                current: 1,
                total: 1,
                asset: asset.name,
                asset_kind: "speech_runtime".to_string(),
                stage: "installed".to_string(),
                percentage: 100,
            },
        );

        emit_provisioning_status(
            &app,
            "completed",
            "Local transcription runtime installed successfully.",
            None,
        );
    });
}

async fn fetch_pyannote_asset_selection(
    client: &reqwest::Client,
) -> Result<PyannoteAssetSelection, String> {
    let bundle = fetch_setup_release_bundle(client).await?;
    let target_triple = host_pyannote_arch_label();
    let runtime_kind = host_pyannote_runtime_kind();
    let runtime_asset = bundle
        .pyannote_manifest
        .assets
        .iter()
        .find(|asset| asset.kind == runtime_kind)
        .cloned()
        .ok_or_else(|| {
            format!(
                "Pyannote release manifest is missing runtime asset kind '{}'.",
                runtime_kind
            )
        })?;
    let setup_runtime_descriptor = setup_arch_descriptor(
        &bundle.setup_manifest.pyannote_runtime_assets,
        target_triple,
        "pyannote runtime asset descriptor",
    )?;
    validate_manifest_asset_descriptor(
        setup_runtime_descriptor,
        &runtime_asset.name,
        &runtime_asset.sha256,
        "pyannote runtime asset",
    )?;
    let model_asset = bundle
        .pyannote_manifest
        .assets
        .iter()
        .find(|asset| asset.kind == "pyannote_model")
        .cloned()
        .ok_or_else(|| "Pyannote release manifest is missing the model asset.".to_string())?;
    validate_manifest_asset_descriptor(
        &bundle.setup_manifest.pyannote_model_asset,
        &model_asset.name,
        &model_asset.sha256,
        "pyannote model asset",
    )?;

    Ok(PyannoteAssetSelection {
        runtime_asset,
        model_asset,
        compat_level: normalize_pyannote_compat_level(bundle.pyannote_manifest.compat_level),
        release_version: bundle.release_version,
    })
}

async fn fetch_runtime_asset_selection(
    client: &reqwest::Client,
) -> Result<RuntimeAssetSelection, String> {
    let bundle = fetch_setup_release_bundle(client).await?;
    let target_triple = host_pyannote_arch_label();
    let runtime_asset = bundle
        .runtime_manifest
        .assets
        .iter()
        .find(|asset| asset.kind == host_runtime_asset_kind())
        .cloned()
        .ok_or_else(|| {
            format!(
                "Runtime release manifest is missing the speech runtime asset kind '{}'.",
                host_runtime_asset_kind()
            )
        })?;
    let setup_runtime_descriptor = setup_arch_descriptor(
        &bundle.setup_manifest.runtime_assets,
        target_triple,
        "runtime asset descriptor",
    )?;
    validate_manifest_asset_descriptor(
        setup_runtime_descriptor,
        &runtime_asset.name,
        &runtime_asset.sha256,
        "runtime asset",
    )?;

    Ok(RuntimeAssetSelection {
        runtime_asset,
        release_version: bundle.release_version,
    })
}

fn local_release_assets_dir() -> Option<PathBuf> {
    let value = std::env::var_os(LOCAL_RELEASE_ASSETS_DIR_ENV)?;
    let path = PathBuf::from(value);
    if path.is_dir() {
        Some(path)
    } else {
        None
    }
}

async fn fetch_setup_release_bundle(
    client: &reqwest::Client,
) -> Result<SetupReleaseBundle, String> {
    let version = env!("CARGO_PKG_VERSION").to_string();
    let setup_manifest = read_release_manifest::<SetupReleaseManifest>(
        client,
        &version,
        SETUP_MANIFEST_ASSET,
        "setup release manifest",
    )
    .await?;
    validate_setup_manifest(&version, &setup_manifest)?;

    let runtime_manifest = read_release_manifest_from_descriptor::<RuntimeReleaseManifest>(
        client,
        &version,
        &setup_manifest.runtime_manifest,
        RUNTIME_MANIFEST_ASSET,
        "runtime release manifest",
    )
    .await?;
    if runtime_manifest.app_version.trim() != version {
        return Err(format!(
            "Runtime manifest version '{}' does not match app version '{}'.",
            runtime_manifest.app_version.trim(),
            version
        ));
    }

    let pyannote_manifest = read_release_manifest_from_descriptor::<PyannoteReleaseManifest>(
        client,
        &version,
        &setup_manifest.pyannote_manifest,
        PYANNOTE_MANIFEST_ASSET,
        "pyannote release manifest",
    )
    .await?;
    if pyannote_manifest.app_version.trim() != version {
        return Err(format!(
            "Pyannote manifest version '{}' does not match app version '{}'.",
            pyannote_manifest.app_version.trim(),
            version
        ));
    }
    let setup_pyannote_compat_level =
        normalize_pyannote_compat_level(setup_manifest.pyannote_compat_level);
    let pyannote_compat_level = normalize_pyannote_compat_level(pyannote_manifest.compat_level);
    if setup_pyannote_compat_level != pyannote_compat_level {
        return Err(format!(
            "Pyannote compatibility level mismatch between setup manifest ({}) and pyannote manifest ({}).",
            setup_pyannote_compat_level, pyannote_compat_level
        ));
    }
    if setup_pyannote_compat_level != PYANNOTE_COMPAT_LEVEL {
        return Err(format!(
            "Pyannote compatibility level '{}' does not match app compatibility level '{}'.",
            setup_pyannote_compat_level, PYANNOTE_COMPAT_LEVEL
        ));
    }

    Ok(SetupReleaseBundle {
        setup_manifest,
        runtime_manifest,
        pyannote_manifest,
        release_version: version,
    })
}

async fn read_release_manifest<T: DeserializeOwned>(
    client: &reqwest::Client,
    version: &str,
    asset_name: &str,
    label: &str,
) -> Result<T, String> {
    let body = read_release_asset_bytes(client, version, asset_name, label).await?;
    serde_json::from_slice::<T>(&body).map_err(|e| format!("invalid {label}: {e}"))
}

async fn read_release_manifest_from_descriptor<T: DeserializeOwned>(
    client: &reqwest::Client,
    version: &str,
    descriptor: &ReleaseAssetDescriptor,
    expected_name: &str,
    label: &str,
) -> Result<T, String> {
    validate_release_descriptor_name(descriptor, expected_name, label)?;
    let body = read_release_asset_bytes(client, version, &descriptor.name, label).await?;
    let actual_sha256 = sha256_bytes_hex(&body);
    let expected_sha256 = normalize_sha256(&descriptor.sha256);
    if actual_sha256 != expected_sha256 {
        return Err(format!(
            "Checksum mismatch for {} '{}': expected {}, got {}.",
            label, descriptor.name, expected_sha256, actual_sha256
        ));
    }
    serde_json::from_slice::<T>(&body).map_err(|e| format!("invalid {label}: {e}"))
}

async fn read_release_asset_bytes(
    client: &reqwest::Client,
    version: &str,
    asset_name: &str,
    label: &str,
) -> Result<Vec<u8>, String> {
    if let Some(local_root) = local_release_assets_dir() {
        let asset_path = local_root.join(asset_name);
        tokio::fs::read(&asset_path).await.map_err(|e| {
            format!(
                "failed to read local {label} '{}': {e}",
                asset_path.display()
            )
        })
    } else {
        let asset_url = release_asset_url(version, asset_name);
        let response = client
            .get(&asset_url)
            .send()
            .await
            .map_err(|e| format!("failed to fetch {label}: {e}"))?
            .error_for_status()
            .map_err(|e| format!("failed to download {label}: {e}"))?;
        response
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(|e| format!("failed to read {label}: {e}"))
    }
}

async fn stage_release_asset(
    client: &reqwest::Client,
    version: &str,
    asset_name: &str,
    destination: &Path,
    cancel_token: &CancellationToken,
) -> Result<(), String> {
    if let Some(local_root) = local_release_assets_dir() {
        let source = local_root.join(asset_name);
        if !source.is_file() {
            return Err(format!(
                "local release asset '{}' is missing in '{}'.",
                asset_name,
                local_root.display()
            ));
        }

        if cancel_token.is_cancelled() {
            return Err("cancelled".to_string());
        }

        // Keep the same transactional semantics for local test/dev assets as
        // for hosted downloads.  A process termination during copy must not
        // turn a partial archive into the next install candidate.
        let temp_path = download_temp_path(destination);
        cleanup_download_temp(&temp_path).await?;
        let copy_result = tokio::fs::copy(&source, &temp_path).await;
        if let Err(error) = copy_result {
            let _ = cleanup_download_temp(&temp_path).await;
            return Err(format!(
                "failed to stage local release asset '{}': {error}",
                source.display()
            ));
        }
        if cancel_token.is_cancelled() {
            let _ = cleanup_download_temp(&temp_path).await;
            return Err("cancelled".to_string());
        }
        if let Err(error) = tokio::fs::remove_file(destination).await {
            if error.kind() != std::io::ErrorKind::NotFound {
                let _ = cleanup_download_temp(&temp_path).await;
                return Err(format!(
                    "failed to replace staged release asset '{}': {error}",
                    destination.display()
                ));
            }
        }
        let result = tokio::fs::rename(&temp_path, destination).await;
        if let Err(error) = result {
            let cleanup = cleanup_download_temp(&temp_path).await.err();
            return Err(match cleanup {
                Some(cleanup_error) => format!(
                    "failed to finalize local release asset '{}': {error}; {cleanup_error}",
                    destination.display()
                ),
                None => format!(
                    "failed to finalize local release asset '{}': {error}",
                    destination.display()
                ),
            });
        }
        Ok(())
    } else {
        let url = release_asset_url(version, asset_name);
        download_to_path(client, &url, destination, cancel_token).await
    }
}

fn validate_setup_manifest(version: &str, manifest: &SetupReleaseManifest) -> Result<(), String> {
    if manifest.app_version.trim() != version {
        return Err(format!(
            "Setup manifest version '{}' does not match app version '{}'.",
            manifest.app_version.trim(),
            version
        ));
    }

    let expected_tag = release_tag(version);
    if manifest.release_tag.trim() != expected_tag {
        return Err(format!(
            "Setup manifest release tag '{}' does not match expected '{}'.",
            manifest.release_tag.trim(),
            expected_tag
        ));
    }
    if normalize_pyannote_compat_level(manifest.pyannote_compat_level) != PYANNOTE_COMPAT_LEVEL {
        return Err(format!(
            "Setup manifest pyannote compatibility level '{}' does not match expected '{}'.",
            normalize_pyannote_compat_level(manifest.pyannote_compat_level),
            PYANNOTE_COMPAT_LEVEL
        ));
    }

    validate_release_descriptor_name(
        &manifest.runtime_manifest,
        RUNTIME_MANIFEST_ASSET,
        "runtime manifest descriptor",
    )?;
    validate_arch_descriptors(
        &manifest.runtime_assets,
        &[
            ("aarch64-apple-darwin", RUNTIME_AARCH64_ASSET),
            ("x86_64-apple-darwin", RUNTIME_X86_64_ASSET),
            ("x86_64-pc-windows-msvc", RUNTIME_WINDOWS_X86_64_ASSET),
        ],
        "runtime asset descriptor",
    )?;
    validate_release_descriptor_name(
        &manifest.pyannote_manifest,
        PYANNOTE_MANIFEST_ASSET,
        "pyannote manifest descriptor",
    )?;
    validate_arch_descriptors(
        &manifest.pyannote_runtime_assets,
        &[
            ("aarch64-apple-darwin", PYANNOTE_RUNTIME_AARCH64_ASSET),
            ("x86_64-apple-darwin", PYANNOTE_RUNTIME_X86_64_ASSET),
            (
                "x86_64-pc-windows-msvc",
                PYANNOTE_RUNTIME_WINDOWS_X86_64_ASSET,
            ),
        ],
        "pyannote runtime asset descriptor",
    )?;
    validate_release_descriptor_name(
        &manifest.pyannote_model_asset,
        PYANNOTE_MODEL_ASSET,
        "pyannote model asset descriptor",
    )?;

    Ok(())
}

fn validate_arch_descriptors(
    descriptors: &std::collections::BTreeMap<String, ReleaseAssetDescriptor>,
    expected_assets: &[(&str, &str)],
    label: &str,
) -> Result<(), String> {
    for &(target_triple, expected_name) in expected_assets {
        let descriptor = setup_arch_descriptor(descriptors, target_triple, label)?;
        validate_release_descriptor_name(descriptor, expected_name, label)?;
    }
    Ok(())
}

fn setup_arch_descriptor<'a>(
    descriptors: &'a std::collections::BTreeMap<String, ReleaseAssetDescriptor>,
    target_triple: &str,
    label: &str,
) -> Result<&'a ReleaseAssetDescriptor, String> {
    descriptors.get(target_triple).ok_or_else(|| {
        format!(
            "Setup manifest is missing {} for target '{}'.",
            label, target_triple
        )
    })
}

fn validate_release_descriptor_name(
    descriptor: &ReleaseAssetDescriptor,
    expected_name: &str,
    label: &str,
) -> Result<(), String> {
    if descriptor.name.trim() != expected_name {
        return Err(format!(
            "{} name mismatch: expected '{}', got '{}'.",
            label, expected_name, descriptor.name
        ));
    }
    if normalize_sha256(&descriptor.sha256).is_empty() {
        return Err(format!(
            "{} '{}' is missing a checksum.",
            label, descriptor.name
        ));
    }
    Ok(())
}

fn validate_manifest_asset_descriptor(
    descriptor: &ReleaseAssetDescriptor,
    actual_name: &str,
    actual_sha256: &str,
    label: &str,
) -> Result<(), String> {
    if descriptor.name.trim() != actual_name.trim() {
        return Err(format!(
            "{} name mismatch: expected '{}', got '{}'.",
            label, descriptor.name, actual_name
        ));
    }

    let expected_sha256 = normalize_sha256(&descriptor.sha256);
    let actual_sha256 = normalize_sha256(actual_sha256);
    if expected_sha256 != actual_sha256 {
        return Err(format!(
            "{} checksum mismatch for '{}': expected {}, got {}.",
            label, actual_name, expected_sha256, actual_sha256
        ));
    }

    Ok(())
}

fn host_pyannote_runtime_kind() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "pyannote_runtime_macos_aarch64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "pyannote_runtime_macos_x86_64"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "pyannote_runtime_windows_x86_64"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "pyannote_runtime_macos_aarch64"
    }
}

fn host_runtime_asset_kind() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "speech_runtime_macos_aarch64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "speech_runtime_macos_x86_64"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "speech_runtime_windows_x86_64"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "speech_runtime_macos_aarch64"
    }
}

fn host_pyannote_arch_label() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        "aarch64-apple-darwin"
    }
}

fn is_pyannote_repair_reason(reason_code: &str) -> bool {
    matches!(
        reason_code.trim(),
        "pyannote_arch_mismatch"
            | "pyannote_version_mismatch"
            | "pyannote_repair_required"
            | "pyannote_install_incomplete"
            | "pyannote_checksum_invalid"
            | "pyannote_import_load_failed"
    )
}

fn normalize_sha256(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn verify_file_sha256(path: &Path, expected_sha256: &str) -> Result<(), String> {
    let expected = normalize_sha256(expected_sha256);
    if expected.is_empty() {
        return Err(format!(
            "Checksum is missing for '{}'.",
            path.file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("downloaded asset")
        ));
    }

    let actual = sha256_file_hex(path)?;
    if actual != expected {
        return Err(format!(
            "Checksum mismatch for '{}': expected {}, got {}.",
            path.file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("downloaded asset"),
            expected,
            actual
        ));
    }

    Ok(())
}

fn sha256_file_hex(path: &Path) -> Result<String, String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("failed to open file for hashing: {e}"))?;
    let mut buffer = [0_u8; 16 * 1024];
    let mut hasher = Sha256::new();

    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|e| format!("failed to read file for hashing: {e}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }

    let digest = hasher.finalize();
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn sha256_bytes_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn install_pyannote_archive(
    archive_path: &Path,
    runtime_dir: &Path,
    expected_root: &str,
    destination: &Path,
) -> Result<(), String> {
    let stage_dir = runtime_dir.join(format!(".stage-{expected_root}"));
    remove_path_if_exists(&stage_dir)?;
    std::fs::create_dir_all(&stage_dir)
        .map_err(|e| format!("failed to create pyannote staging directory: {e}"))?;

    if let Err(error) = extract_zip_archive(archive_path, &stage_dir) {
        let _ = remove_path_if_exists(&stage_dir);
        return Err(error);
    }

    let staged_root = stage_dir.join(expected_root);
    if !staged_root.exists() {
        let _ = remove_path_if_exists(&stage_dir);
        return Err(format!(
            "Pyannote archive '{}' does not contain expected '{}' directory.",
            archive_path.display(),
            expected_root
        ));
    }

    remove_path_if_exists(destination)?;
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create pyannote destination parent: {e}"))?;
    }

    std::fs::rename(&staged_root, destination)
        .map_err(|e| format!("failed to move staged pyannote asset into place: {e}"))?;

    remove_path_if_exists(&stage_dir)?;
    Ok(())
}

/// Extract a Core ML encoder into a sibling staging directory and publish it
/// with a directory rename.  The download/extraction path must never write
/// directly into the model directory: a killed extraction otherwise leaves a
/// directory that looks installed to the next readiness check.
fn install_coreml_encoder_archive(
    archive_path: &Path,
    models_dir: &Path,
    encoder_dir_name: &str,
) -> Result<(), String> {
    let stage_dir = provisioning_swap_path(models_dir, "encoder-stage");
    let destination = models_dir.join(encoder_dir_name);
    let backup_dir = provisioning_swap_path(models_dir, "encoder-backup");

    remove_path_if_exists(&stage_dir)?;
    remove_path_if_exists(&backup_dir)?;
    std::fs::create_dir_all(&stage_dir)
        .map_err(|e| format!("failed to create encoder staging directory: {e}"))?;

    if let Err(error) = extract_zip_archive(archive_path, &stage_dir) {
        let _ = remove_path_if_exists(&stage_dir);
        return Err(error);
    }

    let staged_destination = stage_dir.join(encoder_dir_name);
    if !staged_destination.is_dir() {
        let _ = remove_path_if_exists(&stage_dir);
        return Err(format!(
            "Encoder archive '{}' does not contain expected '{}' directory.",
            archive_path.display(),
            encoder_dir_name
        ));
    }

    let had_existing = destination.exists();
    if had_existing {
        if let Err(error) = std::fs::rename(&destination, &backup_dir) {
            let _ = remove_path_if_exists(&stage_dir);
            return Err(format!(
                "failed to stage existing encoder '{}' into backup '{}': {error}",
                destination.display(),
                backup_dir.display()
            ));
        }
    }

    let promote_result = std::fs::rename(&staged_destination, &destination);
    if let Err(error) = promote_result {
        let _ = remove_path_if_exists(&stage_dir);
        if had_existing {
            let _ = std::fs::rename(&backup_dir, &destination);
        }
        return Err(format!(
            "failed to promote staged encoder '{}' into '{}': {error}",
            staged_destination.display(),
            destination.display()
        ));
    }

    if let Err(error) = remove_path_if_exists(&stage_dir) {
        tracing::warn!(
            "encoder install committed but staging cleanup failed for '{}': {error}",
            stage_dir.display()
        );
    }
    if had_existing {
        if let Err(error) = remove_path_if_exists(&backup_dir) {
            tracing::warn!(
                "encoder install committed but backup cleanup failed for '{}': {error}",
                backup_dir.display()
            );
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct RuntimeInstallJournal {
    install_root: PathBuf,
    destination: PathBuf,
    lib_destination: PathBuf,
    backup_dir: PathBuf,
    stage_dir: PathBuf,
    old_bin_present: bool,
    old_lib_present: bool,
    new_bin_attempted: bool,
    new_lib_attempted: bool,
    /// Set only after the replacement passed managed-runtime validation.  A
    /// crash after this marker is written should finish cleanup, not roll
    /// back an already validated runtime.
    #[serde(default)]
    validated: bool,
}

/// A published runtime whose binaries have not yet passed the managed-runtime
/// probe.  The backup and journal intentionally stay on disk until the caller
/// commits this transaction after validation.  If validation fails, rollback
/// restores the complete previous `bin`/`lib` pair.
#[derive(Debug, Clone)]
struct RuntimeInstallTransaction {
    journal: RuntimeInstallJournal,
    install_root: PathBuf,
    backup_dir: PathBuf,
    stage_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RuntimeInstallCommitError {
    BeforeValidationMarker(String),
    AfterValidationMarker(String),
}

impl std::fmt::Display for RuntimeInstallCommitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BeforeValidationMarker(message) | Self::AfterValidationMarker(message) => {
                formatter.write_str(message)
            }
        }
    }
}

fn runtime_install_journal_path(install_root: &Path) -> PathBuf {
    install_root.join(".runtime-install-journal.json")
}

fn runtime_install_journal_backup_path(install_root: &Path) -> PathBuf {
    install_root.join(".runtime-install-journal.previous.json")
}

fn write_runtime_install_journal(journal: &RuntimeInstallJournal) -> Result<(), String> {
    let path = runtime_install_journal_path(&journal.install_root);
    let temp_path = provisioning_swap_path(&journal.install_root, "runtime-journal");
    let body = serde_json::to_vec_pretty(journal)
        .map_err(|error| format!("failed to serialize runtime install journal: {error}"))?;
    std::fs::write(&temp_path, body).map_err(|error| {
        format!(
            "failed to write runtime install journal '{}': {error}",
            temp_path.display()
        )
    })?;
    if std::fs::rename(&temp_path, &path).is_ok() {
        return Ok(());
    }

    // Windows does not replace an existing file with rename(2).  Keep a
    // recoverable previous journal while swapping the new one in, and let
    // recovery consume the fallback if the process is terminated mid-swap.
    let previous_path = runtime_install_journal_backup_path(&journal.install_root);
    remove_path_if_exists(&previous_path)?;
    if let Err(error) = std::fs::rename(&path, &previous_path) {
        let _ = remove_path_if_exists(&temp_path);
        return Err(format!(
            "failed to publish runtime install journal '{}': {error}",
            path.display()
        ));
    }
    if let Err(error) = std::fs::rename(&temp_path, &path) {
        let _ = std::fs::rename(&previous_path, &path);
        let _ = remove_path_if_exists(&temp_path);
        return Err(format!(
            "failed to publish runtime install journal '{}': {error}",
            path.display()
        ));
    }
    remove_path_if_exists(&previous_path).map_err(|error| {
        format!(
            "failed to remove previous runtime install journal '{}': {error}",
            previous_path.display()
        )
    })?;
    Ok(())
}

fn clear_runtime_install_journal(install_root: &Path) -> Result<(), String> {
    clear_runtime_install_journal_with(install_root, &remove_path_if_exists)
}

fn read_runtime_install_journal(install_root: &Path) -> Result<RuntimeInstallJournal, String> {
    let path = runtime_install_journal_path(install_root);
    let body = std::fs::read(&path).map_err(|error| {
        format!(
            "failed to read runtime install journal '{}': {error}",
            path.display()
        )
    })?;
    serde_json::from_slice(&body).map_err(|error| {
        format!(
            "failed to parse runtime install journal '{}': {error}",
            path.display()
        )
    })
}

fn clear_runtime_install_journal_with<F>(install_root: &Path, remove: &F) -> Result<(), String>
where
    F: Fn(&Path) -> Result<(), String>,
{
    // Remove the fallback first.  If this fails, the validated primary stays
    // authoritative; deleting it first could expose a stale unvalidated
    // fallback on the next recovery pass.
    remove(&runtime_install_journal_backup_path(install_root))?;
    remove(&runtime_install_journal_path(install_root))
}

/// Recover a runtime publish interrupted between the two component renames.
/// The journal is written before any existing component is moved, so a crash
/// can never silently discard the previous runtime.  If both new components
/// are present we finish the commit; otherwise every backed-up component is
/// restored and untouched components are left in place.
pub(crate) fn recover_interrupted_runtime_install(data_dir: &Path) -> Result<(), String> {
    let _transaction_guard = runtime_install_transaction_lock()
        .lock()
        .map_err(|_| "runtime install transaction lock is poisoned".to_string())?;
    recover_interrupted_runtime_install_locked(data_dir)
}

fn recover_interrupted_runtime_install_locked(data_dir: &Path) -> Result<(), String> {
    let primary_journal_path = runtime_install_journal_path(data_dir);
    let journal_path = if primary_journal_path.is_file() {
        primary_journal_path
    } else {
        let fallback_path = runtime_install_journal_backup_path(data_dir);
        if !fallback_path.is_file() {
            return Ok(());
        }
        fallback_path
    };

    if !journal_path.is_file() {
        return Ok(());
    }

    let body = std::fs::read(&journal_path).map_err(|error| {
        format!(
            "failed to read runtime install journal '{}': {error}",
            journal_path.display()
        )
    })?;
    let journal: RuntimeInstallJournal = serde_json::from_slice(&body).map_err(|error| {
        format!(
            "failed to parse runtime install journal '{}': {error}",
            journal_path.display()
        )
    })?;
    if journal.install_root != data_dir {
        return Err(format!(
            "runtime install journal points to unexpected root '{}', expected '{}'; refusing recovery",
            journal.install_root.display(),
            data_dir.display()
        ));
    }

    if journal.validated {
        // Validation completed and the process stopped during cleanup.  Do
        // not roll back a known-good replacement; simply finish deleting the
        // transaction artifacts.
        remove_path_if_exists(&journal.stage_dir)?;
        remove_path_if_exists(&journal.backup_dir)?;
        return clear_runtime_install_journal(data_dir);
    }

    // A journal is deliberately kept until the replacement has passed a real
    // managed-runtime probe.  Therefore an unvalidated journal found during
    // startup or readiness recovery always represents a failed/incomplete
    // transaction, even when both replacement directories happen to exist.
    let backup_bin = journal.backup_dir.join("bin");
    let backup_lib = journal.backup_dir.join("lib");

    if backup_bin.exists() {
        remove_path_if_exists(&journal.destination)?;
        std::fs::rename(&backup_bin, &journal.destination).map_err(|error| {
            format!(
                "failed to restore runtime binaries from '{}': {error}",
                backup_bin.display()
            )
        })?;
    } else if journal.new_bin_attempted && !journal.old_bin_present {
        remove_path_if_exists(&journal.destination)?;
    }

    if backup_lib.exists() {
        remove_path_if_exists(&journal.lib_destination)?;
        std::fs::rename(&backup_lib, &journal.lib_destination).map_err(|error| {
            format!(
                "failed to restore runtime libraries from '{}': {error}",
                backup_lib.display()
            )
        })?;
    } else if journal.new_lib_attempted && !journal.old_lib_present {
        remove_path_if_exists(&journal.lib_destination)?;
    }

    remove_path_if_exists(&journal.stage_dir)?;
    remove_path_if_exists(&journal.backup_dir)?;
    clear_runtime_install_journal(data_dir)
}

fn install_runtime_archive(
    archive_path: &Path,
    runtime_dir: &Path,
    destination: &Path,
) -> Result<RuntimeInstallTransaction, String> {
    let install_root = destination.parent().ok_or_else(|| {
        format!(
            "failed to determine runtime install root from '{}'.",
            destination.display()
        )
    })?;
    let _transaction_guard = runtime_install_transaction_lock()
        .lock()
        .map_err(|_| "runtime install transaction lock is poisoned".to_string())?;
    recover_interrupted_runtime_install_locked(install_root)?;

    let stage_dir = runtime_dir.join(format!(
        ".stage-runtime-{}",
        PROVISIONING_SWAP_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    remove_path_if_exists(&stage_dir)?;
    std::fs::create_dir_all(&stage_dir)
        .map_err(|e| format!("failed to create runtime staging directory: {e}"))?;

    if let Err(error) = extract_zip_archive(archive_path, &stage_dir) {
        let _ = remove_path_if_exists(&stage_dir);
        return Err(error);
    }

    let staged_root = stage_dir.join("runtime");
    if !staged_root.exists() {
        let _ = remove_path_if_exists(&stage_dir);
        return Err(format!(
            "Runtime archive '{}' does not contain expected 'runtime' directory.",
            archive_path.display()
        ));
    }

    let staged_bin = staged_root.join("bin");
    let staged_lib = staged_root.join("lib");
    if !staged_bin.is_dir() || !staged_lib.is_dir() {
        let _ = remove_path_if_exists(&stage_dir);
        return Err(format!(
            "Runtime archive '{}' is missing expected 'bin' or 'lib' directories.",
            archive_path.display()
        ));
    }

    let lib_destination = install_root.join("lib");

    std::fs::create_dir_all(install_root)
        .map_err(|e| format!("failed to create runtime install root: {e}"))?;

    // Keep both components of the managed runtime together.  Renaming bin and
    // lib independently without a backup can leave a mixed-version runtime if
    // the second rename fails (or if the process is terminated between them).
    let backup_dir = provisioning_swap_path(install_root, "runtime-backup");
    remove_path_if_exists(&backup_dir)?;
    std::fs::create_dir_all(&backup_dir)
        .map_err(|e| format!("failed to create runtime backup directory: {e}"))?;
    let existing_bin = destination.exists();
    let existing_lib = lib_destination.exists();

    // The two component renames below are intentionally journaled.  A
    // process termination between them is recovered on the next health or
    // install operation, restoring the previous pair unless both new
    // components were committed.  The journal is written only after the
    // complete archive has been staged, so the previous runtime remains
    // usable until publication starts.
    let mut journal = RuntimeInstallJournal {
        install_root: install_root.to_path_buf(),
        destination: destination.to_path_buf(),
        lib_destination: lib_destination.clone(),
        backup_dir: backup_dir.clone(),
        stage_dir: stage_dir.clone(),
        old_bin_present: existing_bin,
        old_lib_present: existing_lib,
        new_bin_attempted: false,
        new_lib_attempted: false,
        validated: false,
    };
    if let Err(error) = write_runtime_install_journal(&journal) {
        let _ = remove_path_if_exists(&stage_dir);
        let _ = remove_path_if_exists(&backup_dir);
        return Err(error);
    }

    if existing_bin {
        if let Err(error) = std::fs::rename(destination, backup_dir.join("bin")) {
            let _ = remove_path_if_exists(&stage_dir);
            let _ = remove_path_if_exists(&backup_dir);
            let _ = clear_runtime_install_journal(install_root);
            return Err(format!(
                "failed to stage existing runtime binaries into backup '{}': {error}",
                backup_dir.display()
            ));
        }
    }
    if existing_lib {
        if let Err(error) = std::fs::rename(&lib_destination, backup_dir.join("lib")) {
            // The failed rename leaves the existing library in place.  Never
            // remove it while rolling back the binary move; recovery restores
            // only the component that was actually moved.
            if let Err(recovery_error) = recover_interrupted_runtime_install_locked(install_root) {
                tracing::warn!(
                    "failed to recover runtime after library backup failure: {recovery_error}"
                );
            }
            return Err(format!(
                "failed to stage existing runtime libraries into backup '{}': {error}",
                backup_dir.display()
            ));
        }
    }

    journal.new_bin_attempted = true;
    if let Err(error) = write_runtime_install_journal(&journal) {
        let _ = recover_interrupted_runtime_install_locked(install_root);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&staged_bin, destination) {
        let _ = recover_interrupted_runtime_install_locked(install_root);
        return Err(format!(
            "failed to promote staged runtime into '{}': {error}; previous runtime restored",
            install_root.display()
        ));
    }

    journal.new_lib_attempted = true;
    if let Err(error) = write_runtime_install_journal(&journal) {
        let _ = recover_interrupted_runtime_install_locked(install_root);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&staged_lib, &lib_destination) {
        let _ = recover_interrupted_runtime_install_locked(install_root);
        return Err(format!(
            "failed to promote staged runtime into '{}': {error}; previous runtime restored",
            install_root.display()
        ));
    }

    // Keep the journal and old runtime backup until the caller validates the
    // newly published binaries.  This is what lets a checksum-valid but
    // unrunnable archive roll back to the previous working runtime.
    Ok(RuntimeInstallTransaction {
        journal,
        install_root: install_root.to_path_buf(),
        backup_dir,
        stage_dir,
    })
}

fn commit_runtime_install_transaction(
    transaction: &RuntimeInstallTransaction,
) -> Result<(), RuntimeInstallCommitError> {
    commit_runtime_install_transaction_with_cleanup(transaction, remove_path_if_exists)
}

fn commit_runtime_install_transaction_with_cleanup<F>(
    transaction: &RuntimeInstallTransaction,
    remove: F,
) -> Result<(), RuntimeInstallCommitError>
where
    F: Fn(&Path) -> Result<(), String>,
{
    let _transaction_guard = runtime_install_transaction_lock().lock().map_err(|_| {
        RuntimeInstallCommitError::BeforeValidationMarker(
            "runtime install transaction lock is poisoned".to_string(),
        )
    })?;
    // Publish the validation marker before deleting the backup.  If cleanup
    // is interrupted, startup recovery can then finish cleanup without
    // mistaking a validated runtime for an incomplete replacement.
    let mut journal = transaction.journal.clone();
    journal.validated = true;
    if let Err(error) = write_runtime_install_journal(&journal) {
        // Journal publication can fail after the new primary has already been
        // installed (for example when removing the Windows fallback fails).
        // Inspect the durable primary before deciding whether rollback is
        // safe; a validated primary means the replacement must be retained
        // and cleanup retried rather than restoring an older runtime.
        let marker_published = read_runtime_install_journal(&journal.install_root)
            .map(|value| value.validated)
            .unwrap_or(false);
        return Err(if marker_published {
            RuntimeInstallCommitError::AfterValidationMarker(error)
        } else {
            RuntimeInstallCommitError::BeforeValidationMarker(error)
        });
    }
    remove(&transaction.stage_dir).map_err(|error| {
        RuntimeInstallCommitError::AfterValidationMarker(format!(
            "runtime validation completed, but staging cleanup failed for '{}': {error}",
            transaction.stage_dir.display()
        ))
    })?;
    remove(&transaction.backup_dir).map_err(|error| {
        RuntimeInstallCommitError::AfterValidationMarker(format!(
            "runtime validation completed, but backup cleanup failed for '{}': {error}",
            transaction.backup_dir.display()
        ))
    })?;
    clear_runtime_install_journal_with(&transaction.install_root, &remove).map_err(|error| {
        RuntimeInstallCommitError::AfterValidationMarker(format!(
            "runtime validation completed, but install journal cleanup failed: {error}"
        ))
    })
}

fn rollback_runtime_install_transaction(
    transaction: &RuntimeInstallTransaction,
) -> Result<(), String> {
    let _transaction_guard = runtime_install_transaction_lock()
        .lock()
        .map_err(|_| "runtime install transaction lock is poisoned".to_string())?;
    recover_interrupted_runtime_install_locked(&transaction.install_root)
}

fn remove_path_if_exists(path: &Path) -> Result<(), String> {
    if path.is_dir() {
        std::fs::remove_dir_all(path)
            .map_err(|e| format!("failed to remove directory '{}': {e}", path.display()))?;
    } else if path.is_file() {
        std::fs::remove_file(path)
            .map_err(|e| format!("failed to remove file '{}': {e}", path.display()))?;
    }
    Ok(())
}

#[derive(Debug)]
enum DownloadAttemptError {
    Cancelled,
    NonRetryable(String),
    Retryable(String),
}

impl std::fmt::Display for DownloadAttemptError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("cancelled"),
            Self::NonRetryable(message) | Self::Retryable(message) => formatter.write_str(message),
        }
    }
}

fn download_temp_path(destination: &Path) -> PathBuf {
    let filename = destination
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("download");
    let sequence = DOWNLOAD_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    destination.with_file_name(format!(
        ".{filename}.part-{}-{sequence}",
        std::process::id()
    ))
}

fn provisioning_swap_path(parent: &Path, prefix: &str) -> PathBuf {
    let sequence = PROVISIONING_SWAP_COUNTER.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(".{prefix}-{}-{sequence}", std::process::id()))
}

fn response_looks_like_html(prefix: &[u8]) -> bool {
    let trimmed = prefix
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .map_or(&[][..], |offset| &prefix[offset..]);
    if !trimmed.starts_with(b"<") {
        return false;
    }

    let text = String::from_utf8_lossy(trimmed).to_ascii_lowercase();
    text.starts_with("<!doctype html")
        || text.starts_with("<html")
        || text.starts_with("<?xml")
        || text.contains("<html")
}

async fn cleanup_download_temp(temp_path: &Path) -> Result<(), String> {
    match tokio::fs::remove_file(temp_path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to clean up temporary download '{}': {error}",
            temp_path.display()
        )),
    }
}

async fn cleanup_downloaded_archive(archive_path: &Path) -> Result<(), String> {
    match tokio::fs::remove_file(archive_path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to remove downloaded archive '{}': {error}",
            archive_path.display()
        )),
    }
}

async fn download_to_temp(
    client: &reqwest::Client,
    url: &str,
    temp_path: &Path,
    cancel_token: &CancellationToken,
) -> Result<(), DownloadAttemptError> {
    if cancel_token.is_cancelled() {
        return Err(DownloadAttemptError::Cancelled);
    }

    let response_result = tokio::select! {
        _ = cancel_token.cancelled() => return Err(DownloadAttemptError::Cancelled),
        result = tokio::time::timeout(MODEL_DOWNLOAD_REQUEST_TIMEOUT, client.get(url).send()) => result,
    };
    let response = response_result
        .map_err(|_| {
            DownloadAttemptError::Retryable(format!(
                "request timed out after {}s",
                MODEL_DOWNLOAD_REQUEST_TIMEOUT.as_secs()
            ))
        })?
        .map_err(|error| DownloadAttemptError::Retryable(format!("request failed: {error}")))?;

    let status = response.status();
    if !status.is_success() {
        let message = format!("download failed with HTTP {}", status.as_u16());
        let retryable = status.is_server_error()
            || status == reqwest::StatusCode::REQUEST_TIMEOUT
            || status == reqwest::StatusCode::TOO_MANY_REQUESTS;
        return Err(if retryable {
            DownloadAttemptError::Retryable(message)
        } else {
            DownloadAttemptError::NonRetryable(message)
        });
    }

    if response.content_length() == Some(0) {
        return Err(DownloadAttemptError::NonRetryable(
            "downloaded file is empty".to_string(),
        ));
    }

    if response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/html"))
    {
        return Err(DownloadAttemptError::NonRetryable(
            "download returned an HTML response instead of a model asset".to_string(),
        ));
    }

    let mut file = tokio::fs::File::create(temp_path).await.map_err(|error| {
        DownloadAttemptError::NonRetryable(format!(
            "failed to create temporary download file: {error}"
        ))
    })?;
    let mut response = response;
    let mut prefix = Vec::with_capacity(512);
    let mut total_bytes = 0_u64;

    loop {
        let chunk_result = tokio::select! {
            _ = cancel_token.cancelled() => return Err(DownloadAttemptError::Cancelled),
            result = tokio::time::timeout(MODEL_DOWNLOAD_CHUNK_TIMEOUT, response.chunk()) => result,
        };
        let chunk = chunk_result
            .map_err(|_| {
                DownloadAttemptError::Retryable(format!(
                    "download stream timed out after {}s",
                    MODEL_DOWNLOAD_CHUNK_TIMEOUT.as_secs()
                ))
            })?
            .map_err(|error| {
                DownloadAttemptError::Retryable(format!("download stream failure: {error}"))
            })?;
        let Some(chunk) = chunk else {
            break;
        };

        if cancel_token.is_cancelled() {
            return Err(DownloadAttemptError::Cancelled);
        }
        if prefix.len() < 512 {
            let remaining = 512 - prefix.len();
            prefix.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        }
        total_bytes = total_bytes.saturating_add(chunk.len() as u64);
        file.write_all(&chunk).await.map_err(|error| {
            DownloadAttemptError::NonRetryable(format!("failed to write download: {error}"))
        })?;
    }

    if total_bytes == 0 {
        return Err(DownloadAttemptError::NonRetryable(
            "downloaded file is empty".to_string(),
        ));
    }
    if response_looks_like_html(&prefix) {
        return Err(DownloadAttemptError::NonRetryable(
            "download returned HTML instead of a model asset".to_string(),
        ));
    }

    file.flush().await.map_err(|error| {
        DownloadAttemptError::NonRetryable(format!("failed to flush download: {error}"))
    })?;
    file.sync_all().await.map_err(|error| {
        DownloadAttemptError::NonRetryable(format!("failed to sync download: {error}"))
    })?;
    Ok(())
}

async fn download_to_path(
    client: &reqwest::Client,
    url: &str,
    destination: &Path,
    cancel_token: &CancellationToken,
) -> Result<(), String> {
    let temp_path = download_temp_path(destination);
    let mut last_error = None;

    for attempt in 1..=MODEL_DOWNLOAD_MAX_ATTEMPTS {
        if cancel_token.is_cancelled() {
            let _ = cleanup_download_temp(&temp_path).await;
            return Err("cancelled".to_string());
        }
        cleanup_download_temp(&temp_path).await?;

        match download_to_temp(client, url, &temp_path, cancel_token).await {
            Ok(()) => {
                if cancel_token.is_cancelled() {
                    let _ = cleanup_download_temp(&temp_path).await;
                    return Err("cancelled".to_string());
                }
                if destination.exists() {
                    let _ = cleanup_download_temp(&temp_path).await;
                    return Err(format!(
                        "download destination appeared while installing '{}'; refusing to overwrite it",
                        destination.display()
                    ));
                }
                return match tokio::fs::rename(&temp_path, destination).await {
                    Ok(()) => Ok(()),
                    Err(error) => {
                        let cleanup = cleanup_download_temp(&temp_path).await.err();
                        Err(match cleanup {
                            Some(cleanup_error) => format!(
                                "failed to finalize download '{}': {error}; {cleanup_error}",
                                destination.display()
                            ),
                            None => format!(
                                "failed to finalize download '{}': {error}",
                                destination.display()
                            ),
                        })
                    }
                };
            }
            Err(DownloadAttemptError::Cancelled) => {
                let _ = cleanup_download_temp(&temp_path).await;
                return Err("cancelled".to_string());
            }
            Err(DownloadAttemptError::NonRetryable(error)) => {
                let cleanup = cleanup_download_temp(&temp_path).await.err();
                return Err(match cleanup {
                    Some(cleanup_error) => format!("{error}; {cleanup_error}"),
                    None => error,
                });
            }
            Err(DownloadAttemptError::Retryable(error)) => {
                let cleanup = cleanup_download_temp(&temp_path).await;
                if let Err(cleanup_error) = cleanup {
                    return Err(format!("{error}; {cleanup_error}"));
                }
                last_error = Some(error);
                if attempt < MODEL_DOWNLOAD_MAX_ATTEMPTS {
                    tokio::select! {
                        _ = cancel_token.cancelled() => return Err("cancelled".to_string()),
                        _ = tokio::time::sleep(MODEL_DOWNLOAD_RETRY_DELAY) => {}
                    }
                }
            }
        }
    }

    Err(format!(
        "download failed after {MODEL_DOWNLOAD_MAX_ATTEMPTS} attempts: {}",
        last_error.unwrap_or_else(|| "unknown error".to_string())
    ))
}

fn extract_zip_archive(archive_path: &Path, destination: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        extract_zip_archive_with_ditto(archive_path, destination)
    }

    #[cfg(not(target_os = "macos"))]
    {
        return extract_zip_archive_with_zip_crate(archive_path, destination);
    }
}

#[cfg(target_os = "macos")]
fn extract_zip_archive_with_ditto(archive_path: &Path, destination: &Path) -> Result<(), String> {
    let status = std::process::Command::new("/usr/bin/ditto")
        .arg("-x")
        .arg("-k")
        .arg(archive_path)
        .arg(destination)
        .status()
        .map_err(|e| format!("failed to launch ditto for zip extraction: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "ditto failed to extract archive '{}' into '{}' (status: {}).",
            archive_path.display(),
            destination.display(),
            status
        ))
    }
}

#[cfg(not(target_os = "macos"))]
fn extract_zip_archive_with_zip_crate(
    archive_path: &Path,
    destination: &Path,
) -> Result<(), String> {
    let file =
        std::fs::File::open(archive_path).map_err(|e| format!("failed to open archive: {e}"))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("invalid zip archive: {e}"))?;

    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|e| format!("failed to read zip entry: {e}"))?;

        let Some(safe_path) = entry.enclosed_name() else {
            continue;
        };

        let out_path = destination.join(safe_path);

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| format!("failed to create directory: {e}"))?;
            continue;
        }

        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create parent directory: {e}"))?;
        }

        let mut out_file = std::fs::File::create(&out_path)
            .map_err(|e| format!("failed to create extracted file: {e}"))?;

        std::io::copy(&mut entry, &mut out_file)
            .map_err(|e| format!("failed to extract zip entry: {e}"))?;

        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&out_path, std::fs::Permissions::from_mode(mode))
                .map_err(|e| format!("failed to preserve extracted permissions: {e}"))?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        commit_runtime_install_transaction, commit_runtime_install_transaction_with_cleanup,
        estimate_pyannote_required_free_bytes, install_pyannote_archive, install_runtime_archive,
        persist_pyannote_install_failure, plan_pyannote_background_action_inner,
        prepare_pyannote_runtime_stage, prepare_pyannote_runtime_swap,
        probe_pyannote_import_and_load, promote_staged_pyannote_runtime, pyannote_reconcile_action,
        recover_interrupted_runtime_install, remove_path_if_exists, rollback_pyannote_runtime_swap,
        rollback_runtime_install_transaction, runtime_install_journal_backup_path,
        runtime_install_journal_path, runtime_install_transaction_lock, sha256_file_hex,
        transcription_runtime_install_complete, validate_arch_descriptors,
        validate_manifest_asset_descriptor, validate_setup_manifest, verify_file_sha256,
        write_runtime_install_journal, PyannoteAssetSelection, PyannoteBackgroundActionTrigger,
        RuntimeInstallCommitError, RuntimeInstallJournal, RuntimeInstallTransaction,
    };
    use crate::release_assets::{
        PyannoteReleaseAsset, PyannoteReleaseManifest, ReleaseAssetDescriptor, RuntimeReleaseAsset,
        RuntimeReleaseManifest, SetupReleaseManifest, PYANNOTE_COMPAT_LEVEL,
        PYANNOTE_MANIFEST_ASSET, RUNTIME_MANIFEST_ASSET, SETUP_MANIFEST_ASSET,
    };
    use sbobino_domain::{AppSettings, TranscriptionEngine};
    use sbobino_infrastructure::{
        ManagedPyannoteManifest, ManagedRuntimeBinaryHealth, ManagedRuntimeHealth,
        PyannoteRuntimeHealth, ReconcileManagedPyannoteReleaseOutcome, RuntimeHealth,
        RuntimeTranscriptionFactory,
    };
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    fn build_runtime_factory() -> (tempfile::TempDir, Arc<RuntimeTranscriptionFactory>) {
        std::env::set_var("SBOBINO_ALLOW_INSECURE_LOCAL_SECRETS", "1");
        std::env::set_var("SBOBINO_RUNTIME_SOURCE_POLICY", "managed-only");
        let temp = tempdir().expect("failed to create tempdir");
        let data_dir = temp.path().join("app-data");
        let factory = Arc::new(
            RuntimeTranscriptionFactory::new(&data_dir, None)
                .expect("runtime factory should initialize"),
        );
        (temp, factory)
    }

    fn spawn_download_server(responses: Vec<&'static str>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("download server should bind");
        let address = listener
            .local_addr()
            .expect("download server address should resolve");
        let thread = thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().expect("download request should connect");
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .expect("download request timeout should set");
                let mut request = [0_u8; 4096];
                let _ = stream.read(&mut request);
                stream
                    .write_all(response.as_bytes())
                    .expect("download response should write");
                let _ = stream.shutdown(Shutdown::Both);
            }
        });
        (format!("http://{address}/model"), thread)
    }

    #[tokio::test]
    async fn transactional_model_download_retries_and_commits_only_complete_file() {
        let (url, server) = spawn_download_server(vec![
            "HTTP/1.1 200 OK\r\nContent-Length: 16\r\nConnection: close\r\n\r\npartial",
            "HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nmodel-bytes",
        ]);
        let temp = tempdir().expect("download tempdir should create");
        let destination = temp.path().join("parakeet.gguf");
        let client = reqwest::Client::new();

        super::download_to_path(&client, &url, &destination, &CancellationToken::new())
            .await
            .expect("download should recover on the second attempt");
        server.join().expect("download server should finish");

        assert_eq!(
            std::fs::read(&destination).expect("final model should exist"),
            b"model-bytes"
        );
        let leftovers = std::fs::read_dir(temp.path())
            .expect("download tempdir should read")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains(".parakeet.gguf.part-")
            })
            .collect::<Vec<_>>();
        assert!(
            leftovers.is_empty(),
            "partial download temp must be removed"
        );
    }

    #[tokio::test]
    async fn transactional_model_download_rejects_html_without_final_file() {
        let (url, server) = spawn_download_server(vec![
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 31\r\nConnection: close\r\n\r\n<html>not a model</html>",
        ]);
        let temp = tempdir().expect("download tempdir should create");
        let destination = temp.path().join("whisper.bin");
        let client = reqwest::Client::new();

        let error = super::download_to_path(&client, &url, &destination, &CancellationToken::new())
            .await
            .expect_err("HTML response must not install as a model");
        server.join().expect("download server should finish");

        assert!(
            error.contains("HTML"),
            "error should explain invalid asset: {error}"
        );
        assert!(!destination.exists());
        assert!(std::fs::read_dir(temp.path())
            .expect("download tempdir should read")
            .filter_map(Result::ok)
            .all(|entry| !entry
                .file_name()
                .to_string_lossy()
                .contains(".whisper.bin.part-")));
    }

    #[tokio::test]
    async fn failed_encoder_archive_cleanup_allows_retry_download() {
        let (url, server) = spawn_download_server(vec![
            "HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\nvalid-bytes",
        ]);
        let temp = tempdir().expect("download tempdir should create");
        let archive_path = temp.path().join("encoder.zip");
        std::fs::write(&archive_path, b"broken archive").expect("write stale archive");

        super::cleanup_downloaded_archive(&archive_path)
            .await
            .expect("failed extraction archive should be removed");
        super::download_to_path(
            &reqwest::Client::new(),
            &url,
            &archive_path,
            &CancellationToken::new(),
        )
        .await
        .expect("retry should install a fresh archive");
        server.join().expect("download server should finish");

        assert_eq!(
            std::fs::read(archive_path).expect("fresh archive should exist"),
            b"valid-bytes"
        );
    }

    #[tokio::test]
    async fn provisioning_slot_rejects_concurrent_operation_until_guard_drops() {
        let slot = Arc::new(tokio::sync::Mutex::new(None));
        let (_token, guard) = super::acquire_provisioning_slot(slot.clone())
            .await
            .expect("first provisioning operation should acquire slot");
        let error = match super::acquire_provisioning_slot(slot.clone()).await {
            Ok(_) => panic!("second provisioning operation should be rejected"),
            Err(error) => error,
        };
        assert_eq!(error.code, "provisioning_busy");

        drop(guard);
        tokio::task::yield_now().await;
        let (_token, guard) = super::acquire_provisioning_slot(slot)
            .await
            .expect("slot should be reusable after operation exits");
        drop(guard);
    }

    fn persist_settings(factory: &RuntimeTranscriptionFactory, diarization_enabled: bool) {
        let mut settings = AppSettings::default();
        settings.transcription.speaker_diarization.enabled = diarization_enabled;
        settings.sync_legacy_from_sections();
        let body = serde_json::to_string_pretty(&settings).expect("settings should serialize");
        std::fs::write(factory.data_dir().join("settings.json"), body)
            .expect("settings should persist");
    }

    fn write_executable_file(path: &std::path::Path, contents: &str) {
        std::fs::create_dir_all(
            path.parent()
                .expect("executable file should have a parent directory"),
        )
        .expect("parent directory should exist");
        std::fs::write(path, contents).expect("executable should write");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(path)
                .expect("metadata should exist")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).expect("permissions should update");
        }
    }

    fn write_fake_pyannote_stdlib(runtime_root: &std::path::Path, version_dir_name: &str) {
        #[cfg(target_os = "windows")]
        let stdlib_root = {
            let _ = version_dir_name;
            std::fs::create_dir_all(runtime_root.join("DLLs"))
                .expect("runtime DLLs dir should exist");
            runtime_root.join("Lib")
        };
        #[cfg(not(target_os = "windows"))]
        let stdlib_root = runtime_root.join("lib").join(version_dir_name);
        std::fs::create_dir_all(stdlib_root.join("encodings"))
            .expect("stdlib encodings dir should exist");
        #[cfg(not(target_os = "windows"))]
        std::fs::create_dir_all(stdlib_root.join("lib-dynload"))
            .expect("stdlib lib-dynload dir should exist");
        std::fs::create_dir_all(stdlib_root.join("collections"))
            .expect("stdlib collections dir should exist");
        #[cfg(not(target_os = "windows"))]
        std::fs::write(
            runtime_root.join("pyvenv.cfg"),
            format!("home = {}\n", runtime_root.join("bin").display()),
        )
        .expect("pyvenv should write");
        std::fs::write(
            stdlib_root.join("encodings").join("__init__.py"),
            "# test\n",
        )
        .expect("encodings init should write");
        std::fs::write(stdlib_root.join("types.py"), "# test\n").expect("types should write");
        std::fs::write(stdlib_root.join("traceback.py"), "# test\n")
            .expect("traceback should write");
        std::fs::write(
            stdlib_root.join("collections").join("__init__.py"),
            "# test\n",
        )
        .expect("collections init should write");
        std::fs::write(stdlib_root.join("collections").join("abc.py"), "# test\n")
            .expect("collections abc should write");
    }

    fn prepare_ready_pyannote_install(
        factory: &RuntimeTranscriptionFactory,
        manifest: ManagedPyannoteManifest,
        status_reason_code: &str,
    ) {
        #[cfg(target_os = "windows")]
        {
            let python_path = factory.managed_pyannote_python_dir().join("python.exe");
            std::fs::create_dir_all(
                python_path
                    .parent()
                    .expect("test Python path should have a parent"),
            )
            .expect("test Python parent should exist");
            std::fs::copy(
                std::env::current_exe().expect("current test executable should resolve"),
                &python_path,
            )
            .expect("test Python executable should copy");
        }
        #[cfg(not(target_os = "windows"))]
        write_executable_file(
            &factory
                .managed_pyannote_python_dir()
                .join("bin")
                .join("python3"),
            "#!/bin/sh\nexit 0\n",
        );
        write_fake_pyannote_stdlib(&factory.managed_pyannote_python_dir(), "python3.11");
        let model_dir = factory.managed_pyannote_model_dir();
        std::fs::create_dir_all(&model_dir).expect("model dir should exist");
        std::fs::write(model_dir.join("config.yaml"), "name: test\n").expect("config should write");
        factory
            .write_managed_pyannote_manifest(&manifest)
            .expect("manifest should write");
        factory
            .write_managed_pyannote_status(status_reason_code, "ready")
            .expect("status should write");
    }

    #[allow(dead_code)]
    fn write_local_pyannote_release_manifests(
        root: &std::path::Path,
        runtime_sha256: &str,
        model_sha256: &str,
    ) {
        let arm_runtime_sha = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            runtime_sha256
        } else {
            "pyannote-runtime-arm-sha"
        };
        let intel_runtime_sha = if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
            runtime_sha256
        } else {
            "pyannote-runtime-x86-sha"
        };
        let windows_runtime_sha = if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            runtime_sha256
        } else {
            "pyannote-runtime-windows-sha"
        };
        let runtime_manifest = RuntimeReleaseManifest {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            assets: vec![
                RuntimeReleaseAsset {
                    kind: "speech_runtime_macos_aarch64".to_string(),
                    name: "speech-runtime-macos-aarch64.zip".to_string(),
                    sha256: "runtime-sha".to_string(),
                    size_bytes: None,
                    expanded_size_bytes: None,
                },
                RuntimeReleaseAsset {
                    kind: "speech_runtime_macos_x86_64".to_string(),
                    name: "speech-runtime-macos-x86_64.zip".to_string(),
                    sha256: "runtime-x86-sha".to_string(),
                    size_bytes: None,
                    expanded_size_bytes: None,
                },
                RuntimeReleaseAsset {
                    kind: "speech_runtime_windows_x86_64".to_string(),
                    name: "speech-runtime-windows-x86_64.zip".to_string(),
                    sha256: "runtime-windows-sha".to_string(),
                    size_bytes: None,
                    expanded_size_bytes: None,
                },
            ],
        };
        let runtime_manifest_body =
            serde_json::to_vec_pretty(&runtime_manifest).expect("runtime manifest should encode");
        let runtime_manifest_sha = super::sha256_bytes_hex(&runtime_manifest_body);
        std::fs::write(root.join(RUNTIME_MANIFEST_ASSET), runtime_manifest_body)
            .expect("runtime manifest should write");

        let pyannote_manifest = PyannoteReleaseManifest {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            compat_level: PYANNOTE_COMPAT_LEVEL,
            assets: vec![
                PyannoteReleaseAsset {
                    kind: "pyannote_runtime_macos_aarch64".to_string(),
                    name: "pyannote-runtime-macos-aarch64.zip".to_string(),
                    sha256: arm_runtime_sha.to_string(),
                    size_bytes: None,
                    expanded_size_bytes: None,
                },
                PyannoteReleaseAsset {
                    kind: "pyannote_runtime_macos_x86_64".to_string(),
                    name: "pyannote-runtime-macos-x86_64.zip".to_string(),
                    sha256: intel_runtime_sha.to_string(),
                    size_bytes: None,
                    expanded_size_bytes: None,
                },
                PyannoteReleaseAsset {
                    kind: "pyannote_runtime_windows_x86_64".to_string(),
                    name: "pyannote-runtime-windows-x86_64.zip".to_string(),
                    sha256: windows_runtime_sha.to_string(),
                    size_bytes: None,
                    expanded_size_bytes: None,
                },
                PyannoteReleaseAsset {
                    kind: "pyannote_model".to_string(),
                    name: "pyannote-model-community-1.zip".to_string(),
                    sha256: model_sha256.to_string(),
                    size_bytes: None,
                    expanded_size_bytes: None,
                },
            ],
        };
        let pyannote_manifest_body =
            serde_json::to_vec_pretty(&pyannote_manifest).expect("pyannote manifest should encode");
        let pyannote_manifest_sha = super::sha256_bytes_hex(&pyannote_manifest_body);
        std::fs::write(root.join(PYANNOTE_MANIFEST_ASSET), pyannote_manifest_body)
            .expect("pyannote manifest should write");

        let setup_manifest = SetupReleaseManifest {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            release_tag: format!("v{}", env!("CARGO_PKG_VERSION")),
            pyannote_compat_level: PYANNOTE_COMPAT_LEVEL,
            runtime_manifest: descriptor(RUNTIME_MANIFEST_ASSET, &runtime_manifest_sha),
            runtime_assets: BTreeMap::from([
                (
                    "aarch64-apple-darwin".to_string(),
                    descriptor("speech-runtime-macos-aarch64.zip", "runtime-sha"),
                ),
                (
                    "x86_64-apple-darwin".to_string(),
                    descriptor("speech-runtime-macos-x86_64.zip", "runtime-x86-sha"),
                ),
                (
                    "x86_64-pc-windows-msvc".to_string(),
                    descriptor("speech-runtime-windows-x86_64.zip", "runtime-windows-sha"),
                ),
            ]),
            pyannote_manifest: descriptor(PYANNOTE_MANIFEST_ASSET, &pyannote_manifest_sha),
            pyannote_runtime_assets: BTreeMap::from([
                (
                    "aarch64-apple-darwin".to_string(),
                    descriptor("pyannote-runtime-macos-aarch64.zip", arm_runtime_sha),
                ),
                (
                    "x86_64-apple-darwin".to_string(),
                    descriptor("pyannote-runtime-macos-x86_64.zip", intel_runtime_sha),
                ),
                (
                    "x86_64-pc-windows-msvc".to_string(),
                    descriptor("pyannote-runtime-windows-x86_64.zip", windows_runtime_sha),
                ),
            ]),
            pyannote_model_asset: descriptor("pyannote-model-community-1.zip", model_sha256),
        };
        let setup_manifest_body =
            serde_json::to_vec_pretty(&setup_manifest).expect("setup manifest should encode");
        std::fs::write(root.join(SETUP_MANIFEST_ASSET), setup_manifest_body)
            .expect("setup manifest should write");
    }

    fn descriptor(name: &str, sha256: &str) -> ReleaseAssetDescriptor {
        ReleaseAssetDescriptor {
            name: name.to_string(),
            sha256: sha256.to_string(),
            size_bytes: None,
            expanded_size_bytes: None,
        }
    }

    fn managed_binary_health(available: bool) -> ManagedRuntimeBinaryHealth {
        ManagedRuntimeBinaryHealth {
            resolved_path: "/tmp/runtime/bin/tool".to_string(),
            available,
            failure_reason: if available {
                String::new()
            } else {
                "missing".to_string()
            },
            failure_message: if available {
                String::new()
            } else {
                "tool is missing".to_string()
            },
        }
    }

    fn runtime_health_with_managed_runtime(managed_runtime: ManagedRuntimeHealth) -> RuntimeHealth {
        RuntimeHealth {
            host_os: "macos".to_string(),
            host_arch: "aarch64".to_string(),
            is_apple_silicon: true,
            preferred_engine: TranscriptionEngine::WhisperCpp,
            configured_engine: TranscriptionEngine::WhisperCpp,
            runtime_source: "managed_release_asset".to_string(),
            managed_runtime_required: true,
            managed_runtime,
            ffmpeg_path: "ffmpeg".to_string(),
            ffmpeg_resolved: "/tmp/runtime/bin/ffmpeg".to_string(),
            ffmpeg_available: true,
            whisper_cli_path: "whisper-cli".to_string(),
            whisper_cli_resolved: "/tmp/runtime/bin/whisper-cli".to_string(),
            whisper_cli_available: true,
            whisper_stream_path: "whisper-stream".to_string(),
            whisper_stream_resolved: "/tmp/runtime/bin/whisper-stream".to_string(),
            whisper_stream_available: true,
            parakeet_cli_path: "parakeet-cli".to_string(),
            parakeet_cli_resolved: "/tmp/runtime/bin/parakeet-cli".to_string(),
            parakeet_cli_available: false,
            models_dir_configured: "models".to_string(),
            models_dir_resolved: "/tmp/models".to_string(),
            parakeet_models_dir_configured: "parakeet-models".to_string(),
            parakeet_models_dir_resolved: "/tmp/parakeet-models".to_string(),
            model_filename: "ggml-base.bin".to_string(),
            model_present: true,
            parakeet_model_filename: "tdt-0.6b-v3-q4_k.gguf".to_string(),
            parakeet_model_present: false,
            missing_parakeet_models: vec![],
            coreml_encoder_present: false,
            missing_models: vec![],
            missing_encoders: vec![],
            pyannote: PyannoteRuntimeHealth::default(),
            setup_complete: false,
        }
    }

    #[test]
    fn transcription_runtime_install_complete_requires_parakeet_cli() {
        let health = runtime_health_with_managed_runtime(ManagedRuntimeHealth {
            source: "managed_release_asset".to_string(),
            ready: false,
            ffmpeg: managed_binary_health(true),
            whisper_cli: managed_binary_health(true),
            whisper_stream: managed_binary_health(true),
            parakeet_cli: managed_binary_health(false),
        });

        assert!(
            !transcription_runtime_install_complete(&health),
            "runtime repair must not skip install when Parakeet CLI is missing"
        );
    }

    #[tokio::test]
    async fn plan_pyannote_background_action_skips_startup_when_diarization_disabled() {
        let (_temp, factory) = build_runtime_factory();
        persist_settings(&factory, false);

        let action = plan_pyannote_background_action_inner(
            &factory,
            PyannoteBackgroundActionTrigger::Startup,
        )
        .await
        .expect("planner should succeed");

        assert_eq!(action.status, "none");
        assert!(!action.should_start);
        assert_eq!(action.reason_code, "pyannote_auto_check_disabled");
    }

    #[tokio::test]
    async fn plan_pyannote_background_action_skips_startup_when_diarization_enabled() {
        let (_temp, factory) = build_runtime_factory();
        persist_settings(&factory, true);

        let action = plan_pyannote_background_action_inner(
            &factory,
            PyannoteBackgroundActionTrigger::Startup,
        )
        .await
        .expect("planner should succeed");

        assert_eq!(action.status, "none");
        assert!(!action.should_start);
        assert!(!action.force_reinstall);
        assert_eq!(action.reason_code, "pyannote_auto_check_disabled");
    }

    #[tokio::test]
    async fn plan_pyannote_background_action_skips_startup_compat_mismatch() {
        let (_temp, factory) = build_runtime_factory();
        persist_settings(&factory, true);
        prepare_ready_pyannote_install(
            &factory,
            ManagedPyannoteManifest {
                source: "release_asset".to_string(),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                compat_level: PYANNOTE_COMPAT_LEVEL + 1,
                runtime_asset: "pyannote-runtime-macos-aarch64.zip".to_string(),
                runtime_sha256: "runtime-sha".to_string(),
                model_asset: "pyannote-model-community-1.zip".to_string(),
                model_sha256: "model-sha".to_string(),
                runtime_arch: super::host_pyannote_arch_label().to_string(),
                installed_at: "2026-04-21T00:00:00Z".to_string(),
            },
            "ok",
        );

        let action = plan_pyannote_background_action_inner(
            &factory,
            PyannoteBackgroundActionTrigger::Startup,
        )
        .await
        .expect("planner should succeed");

        assert_eq!(action.status, "none");
        assert!(!action.should_start);
        assert!(!action.force_reinstall);
        assert_eq!(action.reason_code, "pyannote_auto_check_disabled");
    }

    #[tokio::test]
    async fn plan_pyannote_background_action_skips_startup_stale_incomplete_status() {
        let (_temp, factory) = build_runtime_factory();
        persist_settings(&factory, true);
        prepare_ready_pyannote_install(
            &factory,
            ManagedPyannoteManifest {
                source: "release_asset".to_string(),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                compat_level: PYANNOTE_COMPAT_LEVEL,
                runtime_asset: "pyannote-runtime-macos-aarch64.zip".to_string(),
                runtime_sha256: "runtime-sha".to_string(),
                model_asset: "pyannote-model-community-1.zip".to_string(),
                model_sha256: "model-sha".to_string(),
                runtime_arch: super::host_pyannote_arch_label().to_string(),
                installed_at: "2026-04-21T00:00:00Z".to_string(),
            },
            "pyannote_install_incomplete",
        );

        let action = plan_pyannote_background_action_inner(
            &factory,
            PyannoteBackgroundActionTrigger::Startup,
        )
        .await
        .expect("planner should succeed");

        assert_eq!(action.status, "none");
        assert!(!action.should_start);
        assert_eq!(action.reason_code, "pyannote_auto_check_disabled");
    }

    #[test]
    fn pyannote_reconcile_action_requests_manifest_only_migration_on_patch_update() {
        let action =
            pyannote_reconcile_action(ReconcileManagedPyannoteReleaseOutcome::ManifestUpdated)
                .expect("manifest update should produce an action");
        assert_eq!(action.status, "migrate_manifest");
        assert!(!action.should_start);
        assert!(!action.force_reinstall);
        assert_eq!(action.reason_code, "pyannote_manifest_migrated");
    }

    #[test]
    fn pyannote_reconcile_action_requests_asset_migration_on_checksum_mismatch() {
        let action =
            pyannote_reconcile_action(ReconcileManagedPyannoteReleaseOutcome::NeedsMigration {
                message:
                    "Pyannote asset checksums do not match the current release and need migration."
                        .to_string(),
            })
            .expect("checksum mismatch should produce an action");
        assert_eq!(action.status, "migrate_assets");
        assert!(action.should_start);
        assert!(action.force_reinstall);
        assert_eq!(action.reason_code, "pyannote_checksum_invalid");
    }

    #[test]
    fn verify_file_sha256_rejects_wrong_checksum() {
        let temp = tempdir().expect("failed to create tempdir");
        let file_path = temp.path().join("asset.bin");
        std::fs::write(&file_path, b"pyannote").expect("failed to write file");

        let actual = sha256_file_hex(&file_path).expect("hash should compute");
        assert!(verify_file_sha256(&file_path, &actual).is_ok());
        assert!(verify_file_sha256(&file_path, "deadbeef").is_err());
    }

    #[test]
    fn install_pyannote_archive_extracts_expected_root() {
        let temp = tempdir().expect("failed to create tempdir");
        let archive_path = temp.path().join("pyannote-runtime.zip");
        let runtime_dir = temp.path().join("runtime");
        std::fs::create_dir_all(&runtime_dir).expect("runtime dir should exist");

        let file = std::fs::File::create(&archive_path).expect("archive should create");
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::SimpleFileOptions =
            zip::write::SimpleFileOptions::default().unix_permissions(0o755);
        zip.add_directory("python/", options)
            .expect("python dir should add");
        zip.add_directory("python/bin/", options)
            .expect("bin dir should add");
        zip.start_file("python/bin/python3", options)
            .expect("python file should start");
        zip.write_all(b"#!/bin/sh\nexit 0\n")
            .expect("python file should write");
        zip.finish().expect("zip should finish");

        let destination = runtime_dir.join("python");
        install_pyannote_archive(&archive_path, &runtime_dir, "python", &destination)
            .expect("pyannote runtime should install");

        let installed = destination.join("bin").join("python3");
        assert!(installed.is_file());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn pyannote_import_load_probe_is_bounded_and_reports_failure() {
        let (_temp, factory) = build_runtime_factory();
        write_executable_file(
            &factory
                .managed_pyannote_python_dir()
                .join("bin")
                .join("python3"),
            "#!/bin/sh\necho 'synthetic pyannote import failure' >&2\nexit 1\n",
        );
        std::fs::create_dir_all(factory.managed_pyannote_model_dir())
            .expect("model directory should exist");

        let error = probe_pyannote_import_and_load(&factory, &CancellationToken::new())
            .await
            .expect_err("failed Python import should be surfaced");
        assert!(error.contains("synthetic pyannote import failure"));
    }

    #[test]
    fn install_runtime_archive_extracts_expected_layout_and_permissions() {
        let temp = tempdir().expect("failed to create tempdir");
        let archive_path = temp.path().join("speech-runtime.zip");
        let runtime_dir = temp.path().join("runtime");
        std::fs::create_dir_all(&runtime_dir).expect("runtime dir should exist");

        let file = std::fs::File::create(&archive_path).expect("archive should create");
        let mut zip = zip::ZipWriter::new(file);
        let dir_options: zip::write::SimpleFileOptions =
            zip::write::SimpleFileOptions::default().unix_permissions(0o755);
        let file_options: zip::write::SimpleFileOptions =
            zip::write::SimpleFileOptions::default().unix_permissions(0o755);

        for directory in ["runtime/", "runtime/bin/", "runtime/lib/"] {
            zip.add_directory(directory, dir_options)
                .expect("directory should add");
        }
        zip.start_file("runtime/bin/whisper-cli", file_options)
            .expect("binary should start");
        zip.write_all(b"#!/bin/sh\nexit 0\n")
            .expect("binary should write");
        zip.start_file("runtime/lib/libwhisper.dylib", file_options)
            .expect("library should start");
        zip.write_all(b"fake").expect("library should write");
        zip.finish().expect("zip should finish");

        let destination = runtime_dir.join("bin");
        let transaction = install_runtime_archive(&archive_path, &runtime_dir, &destination)
            .expect("runtime should install");

        let installed_binary = destination.join("whisper-cli");
        let installed_library = runtime_dir.join("lib").join("libwhisper.dylib");
        assert!(installed_binary.is_file());
        assert!(installed_library.is_file());

        commit_runtime_install_transaction(&transaction)
            .expect("validated runtime transaction should commit");
        assert!(!runtime_dir.join(".runtime-install-journal.json").exists());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&installed_binary)
                .expect("metadata should read")
                .permissions()
                .mode();
            assert_ne!(mode & 0o111, 0, "installed binary should remain executable");
        }
    }

    #[test]
    fn checksum_valid_but_unrunnable_runtime_rolls_back_until_validation() {
        let temp = tempdir().expect("failed to create tempdir");
        let install_root = temp.path().join("app-data");
        let runtime_dir = install_root.join("runtime");
        let destination = install_root.join("bin");
        let lib_destination = install_root.join("lib");
        std::fs::create_dir_all(&destination).expect("old binary directory should exist");
        std::fs::write(destination.join("whisper-cli"), b"old-working-runtime")
            .expect("old binary should write");
        std::fs::create_dir_all(&lib_destination).expect("old library directory should exist");
        std::fs::write(
            lib_destination.join("libwhisper.dylib"),
            b"old-working-library",
        )
        .expect("old library should write");

        let archive_path = temp.path().join("replacement.zip");
        let file = std::fs::File::create(&archive_path).expect("archive should create");
        let mut zip = zip::ZipWriter::new(file);
        let options: zip::write::SimpleFileOptions =
            zip::write::SimpleFileOptions::default().unix_permissions(0o755);
        for directory in ["runtime/", "runtime/bin/", "runtime/lib/"] {
            zip.add_directory(directory, options)
                .expect("directory should add");
        }
        zip.start_file("runtime/bin/whisper-cli", options)
            .expect("replacement binary should start");
        zip.write_all(b"this is not a runnable executable")
            .expect("replacement binary should write");
        zip.start_file("runtime/lib/libwhisper.dylib", options)
            .expect("replacement library should start");
        zip.write_all(b"replacement-library")
            .expect("replacement library should write");
        zip.finish().expect("archive should finish");

        let transaction = install_runtime_archive(&archive_path, &runtime_dir, &destination)
            .expect("checksum-valid archive should publish transactionally");
        assert!(install_root.join(".runtime-install-journal.json").is_file());
        assert!(transaction.backup_dir.join("bin/whisper-cli").is_file());
        assert_eq!(
            std::fs::read(destination.join("whisper-cli")).expect("replacement should exist"),
            b"this is not a runnable executable"
        );

        rollback_runtime_install_transaction(&transaction)
            .expect("failed validation should restore the previous runtime");
        assert_eq!(
            std::fs::read(destination.join("whisper-cli")).expect("old binary should restore"),
            b"old-working-runtime"
        );
        assert_eq!(
            std::fs::read(lib_destination.join("libwhisper.dylib"))
                .expect("old library should restore"),
            b"old-working-library"
        );
        assert!(!install_root.join(".runtime-install-journal.json").exists());
        assert!(!transaction.backup_dir.exists());
    }

    fn runtime_commit_cleanup_fixture() -> (tempfile::TempDir, RuntimeInstallTransaction) {
        let temp = tempdir().expect("commit fixture tempdir should exist");
        let install_root = temp.path().join("app-data");
        let destination = install_root.join("bin");
        let lib_destination = install_root.join("lib");
        let backup_dir = install_root.join(".runtime-backup-cleanup");
        let stage_dir = install_root.join("runtime").join(".stage-cleanup");
        std::fs::create_dir_all(&destination).expect("new binary directory should exist");
        std::fs::write(destination.join("whisper-cli"), b"new-runtime")
            .expect("new binary should write");
        std::fs::create_dir_all(&lib_destination).expect("new library directory should exist");
        std::fs::write(lib_destination.join("libwhisper.dylib"), b"new-library")
            .expect("new library should write");
        std::fs::create_dir_all(backup_dir.join("bin")).expect("backup bin should exist");
        std::fs::write(backup_dir.join("bin/whisper-cli"), b"old-runtime")
            .expect("old binary should write");
        std::fs::create_dir_all(backup_dir.join("lib")).expect("backup lib should exist");
        std::fs::write(backup_dir.join("lib/libwhisper.dylib"), b"old-library")
            .expect("old library should write");
        std::fs::create_dir_all(&stage_dir).expect("stage should exist");

        let journal = RuntimeInstallJournal {
            install_root: install_root.clone(),
            destination,
            lib_destination,
            backup_dir: backup_dir.clone(),
            stage_dir: stage_dir.clone(),
            old_bin_present: true,
            old_lib_present: true,
            new_bin_attempted: true,
            new_lib_attempted: true,
            validated: false,
        };
        write_runtime_install_journal(&journal).expect("unvalidated journal should write");

        let transaction = RuntimeInstallTransaction {
            journal,
            install_root,
            backup_dir,
            stage_dir,
        };
        (temp, transaction)
    }

    #[test]
    fn runtime_commit_marker_failure_rolls_back_without_success_state() {
        let temp = tempdir().expect("recovery tempdir should exist");
        let install_root = temp.path().join("app-data");
        let destination = install_root.join("bin");
        let lib_destination = install_root.join("lib");
        let backup_dir = install_root.join(".runtime-backup-commit-failure");
        let stage_dir = install_root.join("runtime").join(".stage-commit-failure");
        std::fs::create_dir_all(&destination).expect("new binary directory should exist");
        std::fs::write(destination.join("whisper-cli"), b"new-runtime")
            .expect("new binary should write");
        std::fs::create_dir_all(&lib_destination).expect("new library directory should exist");
        std::fs::write(lib_destination.join("libwhisper.dylib"), b"new-library")
            .expect("new library should write");
        std::fs::create_dir_all(backup_dir.join("bin")).expect("backup bin should exist");
        std::fs::write(backup_dir.join("bin/whisper-cli"), b"old-runtime")
            .expect("old binary should write");
        std::fs::create_dir_all(backup_dir.join("lib")).expect("backup lib should exist");
        std::fs::write(backup_dir.join("lib/libwhisper.dylib"), b"old-library")
            .expect("old library should write");
        std::fs::create_dir_all(&stage_dir).expect("stage should exist");

        let journal = RuntimeInstallJournal {
            install_root: install_root.clone(),
            destination: destination.clone(),
            lib_destination: lib_destination.clone(),
            backup_dir: backup_dir.clone(),
            stage_dir: stage_dir.clone(),
            old_bin_present: true,
            old_lib_present: true,
            new_bin_attempted: true,
            new_lib_attempted: true,
            validated: false,
        };
        write_runtime_install_journal(&journal).expect("unvalidated journal should write");

        // Point the commit's journal copy at a non-existent parent to inject
        // the validation-marker publication failure while retaining the real
        // transaction root used by rollback/recovery.
        let mut failed_commit_journal = journal.clone();
        failed_commit_journal.install_root = temp.path().join("missing-parent").join("runtime");
        let transaction = RuntimeInstallTransaction {
            journal: failed_commit_journal,
            install_root: install_root.clone(),
            backup_dir: backup_dir.clone(),
            stage_dir: stage_dir.clone(),
        };
        let commit_error = commit_runtime_install_transaction(&transaction)
            .expect_err("validation marker publication should fail");
        assert!(matches!(
            commit_error,
            RuntimeInstallCommitError::BeforeValidationMarker(ref message)
                if message.contains("failed to write runtime install journal")
        ));

        // This is the same durable failure policy used by the provisioning
        // command: never emit installed/completed after commit failure; first
        // restore the previous runtime from the unvalidated journal.
        rollback_runtime_install_transaction(&transaction)
            .expect("commit failure should roll back the previous runtime");
        assert_eq!(
            std::fs::read(destination.join("whisper-cli")).expect("old binary should restore"),
            b"old-runtime"
        );
        assert_eq!(
            std::fs::read(lib_destination.join("libwhisper.dylib"))
                .expect("old library should restore"),
            b"old-library"
        );
        assert!(!runtime_install_journal_path(&install_root).exists());
        assert!(!backup_dir.exists());
        assert!(!stage_dir.exists());
    }

    #[test]
    fn runtime_commit_post_marker_cleanup_failures_are_incomplete_and_recoverable() {
        for (label, fail_stage, fail_backup, fail_journal, fail_fallback) in [
            ("stage", true, false, false, false),
            ("backup", false, true, false, false),
            ("journal", false, false, true, false),
            ("journal-fallback", false, false, false, true),
        ] {
            let (_temp, transaction) = runtime_commit_cleanup_fixture();
            let stage_target = transaction.stage_dir.clone();
            let backup_target = transaction.backup_dir.clone();
            let journal_target = runtime_install_journal_path(&transaction.install_root);
            let fallback_target = runtime_install_journal_backup_path(&transaction.install_root);
            let install_root = transaction.install_root.clone();
            let destination = transaction.journal.destination.clone();
            if fail_fallback {
                std::fs::write(
                    &fallback_target,
                    serde_json::to_vec_pretty(&transaction.journal)
                        .expect("stale fallback journal should serialize"),
                )
                .expect("stale fallback journal should write");
            }
            let target = if fail_stage {
                stage_target.clone()
            } else if fail_backup {
                backup_target.clone()
            } else if fail_journal {
                journal_target.clone()
            } else if fail_fallback {
                fallback_target.clone()
            } else {
                unreachable!("fixture must inject one cleanup failure");
            };
            let target_for_cleanup = target.clone();
            let error =
                commit_runtime_install_transaction_with_cleanup(&transaction, move |path| {
                    if path == target_for_cleanup {
                        return Err(format!("injected {label} cleanup failure"));
                    }
                    remove_path_if_exists(path)
                })
                .expect_err("post-marker cleanup failure must not report success");
            assert!(matches!(
                error,
                RuntimeInstallCommitError::AfterValidationMarker(ref message)
                    if message.contains("runtime validation completed")
            ));

            let journal_body = std::fs::read(runtime_install_journal_path(&install_root))
                .expect("validated journal should remain for safe cleanup");
            let journal: RuntimeInstallJournal =
                serde_json::from_slice(&journal_body).expect("validated journal should parse");
            assert!(
                journal.validated,
                "{label} cleanup failure must retain validation marker"
            );
            assert_eq!(
                std::fs::read(destination.join("whisper-cli"))
                    .expect("validated runtime should remain active"),
                b"new-runtime"
            );
            if fail_stage {
                assert!(
                    stage_target.exists(),
                    "failed stage cleanup must retain stage"
                );
                assert!(
                    backup_target.exists(),
                    "failed stage cleanup must retain backup"
                );
            } else if fail_backup {
                assert!(
                    !stage_target.exists(),
                    "stage cleanup should precede backup failure"
                );
                assert!(
                    backup_target.exists(),
                    "failed backup cleanup must retain backup"
                );
            } else {
                assert!(
                    !stage_target.exists(),
                    "stage should be cleaned before journal failure"
                );
                assert!(
                    !backup_target.exists(),
                    "backup should be cleaned before journal failure"
                );
            }
            if fail_fallback {
                assert!(
                    journal_target.exists(),
                    "fallback cleanup failure must retain validated primary journal"
                );
                assert!(
                    fallback_target.exists(),
                    "fallback cleanup failure must retain stale fallback for recovery"
                );
            }

            // A later health/recovery pass can safely finish cleanup without
            // rolling back the already validated replacement.
            recover_interrupted_runtime_install(&install_root)
                .expect("validated cleanup should recover after the injected failure");
            assert!(!runtime_install_journal_path(&install_root).exists());
            assert!(!stage_target.exists());
            assert!(!backup_target.exists());
            assert_eq!(
                std::fs::read(destination.join("whisper-cli"))
                    .expect("validated runtime should remain after recovery"),
                b"new-runtime"
            );
        }
    }

    #[test]
    fn recovery_waits_for_active_runtime_publish_before_touching_transaction_files() {
        let temp = tempdir().expect("failed to create tempdir");
        let install_root = temp.path().join("app-data");
        let destination = install_root.join("bin");
        let lib_destination = install_root.join("lib");
        let backup_dir = install_root.join(".runtime-backup-active");
        let stage_dir = install_root.join("runtime").join(".stage-runtime-active");
        std::fs::create_dir_all(&destination).expect("replacement binary directory should exist");
        std::fs::write(destination.join("tool"), b"replacement")
            .expect("replacement binary should write");
        std::fs::create_dir_all(&lib_destination)
            .expect("replacement library directory should exist");
        std::fs::write(lib_destination.join("libtool"), b"replacement")
            .expect("replacement library should write");
        std::fs::create_dir_all(backup_dir.join("bin")).expect("backup bin should exist");
        std::fs::write(backup_dir.join("bin/tool"), b"previous")
            .expect("previous binary should write");
        std::fs::create_dir_all(backup_dir.join("lib")).expect("backup lib should exist");
        std::fs::write(backup_dir.join("lib/libtool"), b"previous")
            .expect("previous library should write");
        std::fs::create_dir_all(&stage_dir).expect("stage should exist");
        write_runtime_install_journal(&RuntimeInstallJournal {
            install_root: install_root.clone(),
            destination: destination.clone(),
            lib_destination: lib_destination.clone(),
            backup_dir: backup_dir.clone(),
            stage_dir: stage_dir.clone(),
            old_bin_present: true,
            old_lib_present: true,
            new_bin_attempted: true,
            new_lib_attempted: true,
            validated: false,
        })
        .expect("active journal should write");

        let transaction_guard = runtime_install_transaction_lock()
            .lock()
            .expect("transaction lock should be available");
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let recovery_root = install_root.clone();
        let recovery_thread = std::thread::spawn(move || {
            started_tx.send(()).expect("recovery thread should start");
            recover_interrupted_runtime_install(&recovery_root)
                .expect("recovery should complete after publish releases lock");
        });
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("recovery thread should start");
        std::thread::sleep(Duration::from_millis(50));
        assert!(
            runtime_install_journal_path(&install_root).is_file(),
            "active transaction journal must not be removed while publish holds lock"
        );
        assert!(stage_dir.exists(), "active stage must remain untouched");
        assert!(backup_dir.exists(), "active backup must remain untouched");
        drop(transaction_guard);
        recovery_thread.join().expect("recovery thread should join");
        assert_eq!(
            std::fs::read(destination.join("tool")).expect("previous binary should restore"),
            b"previous"
        );
        assert!(!runtime_install_journal_path(&install_root).exists());
    }

    #[test]
    fn recovery_finishes_cleanup_after_validation_marker_without_rollback() {
        let temp = tempdir().expect("failed to create tempdir");
        let install_root = temp.path().join("app-data");
        let destination = install_root.join("bin");
        let lib_destination = install_root.join("lib");
        let backup_dir = install_root.join(".runtime-backup-validated");
        let stage_dir = install_root
            .join("runtime")
            .join(".stage-runtime-validated");
        std::fs::create_dir_all(&destination).expect("new binary directory should exist");
        std::fs::write(destination.join("tool"), b"validated-new")
            .expect("new binary should write");
        std::fs::create_dir_all(&lib_destination).expect("new library directory should exist");
        std::fs::write(lib_destination.join("libtool"), b"validated-new")
            .expect("new library should write");
        std::fs::create_dir_all(backup_dir.join("bin")).expect("backup bin should exist");
        std::fs::write(backup_dir.join("bin/tool"), b"old").expect("old binary should write");
        std::fs::create_dir_all(backup_dir.join("lib")).expect("backup lib should exist");
        std::fs::write(backup_dir.join("lib/libtool"), b"old").expect("old library should write");
        std::fs::create_dir_all(&stage_dir).expect("stage should exist");
        write_runtime_install_journal(&RuntimeInstallJournal {
            install_root: install_root.clone(),
            destination: destination.clone(),
            lib_destination: lib_destination.clone(),
            backup_dir: backup_dir.clone(),
            stage_dir: stage_dir.clone(),
            old_bin_present: true,
            old_lib_present: true,
            new_bin_attempted: true,
            new_lib_attempted: true,
            validated: true,
        })
        .expect("validated journal should write");

        recover_interrupted_runtime_install(&install_root)
            .expect("validated transaction cleanup should succeed");
        assert_eq!(
            std::fs::read(destination.join("tool")).expect("validated binary should remain"),
            b"validated-new"
        );
        assert!(!backup_dir.exists());
        assert!(!runtime_install_journal_path(&install_root).exists());
    }

    #[test]
    fn runtime_recovery_fault_injection_preserves_library_when_backup_fails() {
        let temp = tempdir().expect("failed to create tempdir");
        let install_root = temp.path().join("app-data");
        let destination = install_root.join("bin");
        let lib_destination = install_root.join("lib");
        let backup_dir = install_root.join(".runtime-backup-fault");
        let stage_dir = install_root.join("runtime").join(".stage-runtime-fault");

        std::fs::create_dir_all(&destination).expect("old binary directory should exist");
        std::fs::write(destination.join("tool"), b"old-bin").expect("old binary should write");
        std::fs::create_dir_all(&lib_destination).expect("old library directory should exist");
        std::fs::write(lib_destination.join("libtool"), b"old-lib")
            .expect("old library should write");
        std::fs::create_dir_all(&backup_dir).expect("backup directory should exist");
        std::fs::rename(&destination, backup_dir.join("bin"))
            .expect("fault should occur after binary backup");
        std::fs::create_dir_all(&stage_dir).expect("stage directory should exist");

        write_runtime_install_journal(&RuntimeInstallJournal {
            install_root: install_root.clone(),
            destination: destination.clone(),
            lib_destination: lib_destination.clone(),
            backup_dir: backup_dir.clone(),
            stage_dir: stage_dir.clone(),
            old_bin_present: true,
            old_lib_present: true,
            new_bin_attempted: false,
            new_lib_attempted: false,
            validated: false,
        })
        .expect("fault journal should write");

        recover_interrupted_runtime_install(&install_root)
            .expect("recovery should restore the previous pair");
        assert_eq!(
            std::fs::read(destination.join("tool")).expect("binary should be restored"),
            b"old-bin"
        );
        assert_eq!(
            std::fs::read(lib_destination.join("libtool")).expect("library should be untouched"),
            b"old-lib"
        );
        assert!(!install_root.join(".runtime-install-journal.json").exists());
    }

    #[test]
    fn runtime_recovery_fault_injection_rolls_back_second_promotion() {
        let temp = tempdir().expect("failed to create tempdir");
        let install_root = temp.path().join("app-data");
        let destination = install_root.join("bin");
        let lib_destination = install_root.join("lib");
        let backup_dir = install_root.join(".runtime-backup-fault");
        let stage_dir = install_root.join("runtime").join(".stage-runtime-fault");

        std::fs::create_dir_all(&destination).expect("old binary directory should exist");
        std::fs::write(destination.join("tool"), b"old-bin").expect("old binary should write");
        std::fs::create_dir_all(&lib_destination).expect("old library directory should exist");
        std::fs::write(lib_destination.join("libtool"), b"old-lib")
            .expect("old library should write");
        std::fs::create_dir_all(&backup_dir).expect("backup directory should exist");
        std::fs::rename(&destination, backup_dir.join("bin"))
            .expect("old binary should move to backup");
        std::fs::rename(&lib_destination, backup_dir.join("lib"))
            .expect("old library should move to backup");
        std::fs::create_dir_all(&destination).expect("partial new binary should exist");
        std::fs::write(destination.join("tool"), b"new-bin")
            .expect("partial new binary should write");
        std::fs::create_dir_all(&stage_dir).expect("stage directory should exist");

        write_runtime_install_journal(&RuntimeInstallJournal {
            install_root: install_root.clone(),
            destination: destination.clone(),
            lib_destination: lib_destination.clone(),
            backup_dir: backup_dir.clone(),
            stage_dir: stage_dir.clone(),
            old_bin_present: true,
            old_lib_present: true,
            new_bin_attempted: true,
            new_lib_attempted: true,
            validated: false,
        })
        .expect("fault journal should write");

        recover_interrupted_runtime_install(&install_root)
            .expect("recovery should roll back an incomplete second promotion");
        assert_eq!(
            std::fs::read(destination.join("tool")).expect("binary should be restored"),
            b"old-bin"
        );
        assert_eq!(
            std::fs::read(lib_destination.join("libtool")).expect("library should be restored"),
            b"old-lib"
        );
        assert!(!install_root.join(".runtime-install-journal.json").exists());
    }

    #[test]
    fn failed_pyannote_repair_keeps_ready_status_and_records_diagnostic() {
        let (_temp, factory) = build_runtime_factory();
        let manifest = ManagedPyannoteManifest {
            source: "release_asset".to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            compat_level: PYANNOTE_COMPAT_LEVEL,
            runtime_asset: "pyannote-runtime-test.zip".to_string(),
            runtime_sha256: "runtime-sha".to_string(),
            model_asset: "pyannote-model-test.zip".to_string(),
            model_sha256: "model-sha".to_string(),
            runtime_arch: super::host_pyannote_arch_label().to_string(),
            installed_at: "2026-04-21T00:00:00Z".to_string(),
        };
        prepare_ready_pyannote_install(&factory, manifest, "ok");

        persist_pyannote_install_failure(
            &factory,
            true,
            "pyannote_import_load_failed",
            "synthetic repair failure",
        );

        let status = factory
            .read_managed_pyannote_status()
            .expect("ready status should remain present");
        assert_eq!(status.reason_code, "ok");
        let diagnostic_path = factory
            .managed_pyannote_runtime_dir()
            .join("last-install-failure.json");
        let diagnostic = std::fs::read_to_string(diagnostic_path)
            .expect("repair diagnostic should be recorded separately");
        assert!(diagnostic.contains("pyannote_import_load_failed"));
        assert!(diagnostic.contains("synthetic repair failure"));
    }

    #[test]
    fn pyannote_runtime_swap_rolls_back_previous_install_on_failure() {
        let temp = tempdir().expect("failed to create tempdir");
        let runtime_dir = temp.path().join("pyannote-runtime");
        std::fs::create_dir_all(runtime_dir.join("python/bin")).expect("runtime tree should exist");
        std::fs::write(runtime_dir.join("python/bin/python3"), b"old-runtime")
            .expect("old runtime should write");

        let backup = prepare_pyannote_runtime_swap(&runtime_dir, true)
            .expect("swap should stage existing runtime")
            .expect("backup should be present");
        std::fs::create_dir_all(runtime_dir.join("python/bin"))
            .expect("new runtime tree should exist");
        std::fs::write(runtime_dir.join("python/bin/python3"), b"broken-runtime")
            .expect("broken runtime should write");

        rollback_pyannote_runtime_swap(&runtime_dir, Some(backup.as_path()))
            .expect("rollback should restore previous runtime");

        let restored = std::fs::read(runtime_dir.join("python/bin/python3"))
            .expect("restored runtime should exist");
        assert_eq!(restored, b"old-runtime");
    }

    #[test]
    fn promote_staged_pyannote_runtime_swaps_only_after_staging_finishes() {
        let temp = tempdir().expect("failed to create tempdir");
        let runtime_dir = temp.path().join("pyannote-runtime");
        std::fs::create_dir_all(runtime_dir.join("python/bin")).expect("runtime tree should exist");
        std::fs::write(runtime_dir.join("python/bin/python3"), b"old-runtime")
            .expect("old runtime should write");

        let stage_dir =
            prepare_pyannote_runtime_stage(&runtime_dir).expect("stage dir should be created");
        std::fs::create_dir_all(stage_dir.join("python/bin"))
            .expect("staged runtime tree should exist");
        std::fs::write(stage_dir.join("python/bin/python3"), b"new-runtime")
            .expect("new runtime should write");

        let still_old = std::fs::read(runtime_dir.join("python/bin/python3"))
            .expect("existing runtime should remain until promotion");
        assert_eq!(still_old, b"old-runtime");

        let backup = promote_staged_pyannote_runtime(&runtime_dir, &stage_dir, true)
            .expect("promotion should succeed")
            .expect("backup should exist");

        let promoted = std::fs::read(runtime_dir.join("python/bin/python3"))
            .expect("promoted runtime should exist");
        assert_eq!(promoted, b"new-runtime");
        assert!(backup.join("python/bin/python3").is_file());
    }

    #[test]
    fn validate_setup_manifest_rejects_mismatched_release_tag() {
        let manifest = SetupReleaseManifest {
            app_version: "0.1.16".to_string(),
            release_tag: "v0.1.8".to_string(),
            pyannote_compat_level: 1,
            runtime_manifest: descriptor("runtime-manifest.json", "deadbeef"),
            runtime_assets: BTreeMap::from([
                (
                    "aarch64-apple-darwin".to_string(),
                    descriptor("speech-runtime-macos-aarch64.zip", "deadbeef"),
                ),
                (
                    "x86_64-apple-darwin".to_string(),
                    descriptor("speech-runtime-macos-x86_64.zip", "deadbeef"),
                ),
                (
                    "x86_64-pc-windows-msvc".to_string(),
                    descriptor("speech-runtime-windows-x86_64.zip", "deadbeef"),
                ),
            ]),
            pyannote_manifest: descriptor("pyannote-manifest.json", "deadbeef"),
            pyannote_runtime_assets: BTreeMap::from([
                (
                    "aarch64-apple-darwin".to_string(),
                    descriptor("pyannote-runtime-macos-aarch64.zip", "deadbeef"),
                ),
                (
                    "x86_64-apple-darwin".to_string(),
                    descriptor("pyannote-runtime-macos-x86_64.zip", "deadbeef"),
                ),
                (
                    "x86_64-pc-windows-msvc".to_string(),
                    descriptor("pyannote-runtime-windows-x86_64.zip", "deadbeef"),
                ),
            ]),
            pyannote_model_asset: descriptor("pyannote-model-community-1.zip", "deadbeef"),
        };

        let error = validate_setup_manifest("0.1.16", &manifest)
            .expect_err("release tag mismatch should fail");
        assert!(error.contains("release tag"));
    }

    #[test]
    fn validate_manifest_asset_descriptor_rejects_checksum_mismatch() {
        let descriptor = descriptor("speech-runtime-macos-aarch64.zip", "deadbeef");
        let error = validate_manifest_asset_descriptor(
            &descriptor,
            "speech-runtime-macos-aarch64.zip",
            "cafebabe",
            "runtime asset",
        )
        .expect_err("checksum mismatch should fail");
        assert!(error.contains("checksum mismatch"));
    }

    #[test]
    fn validate_arch_descriptors_requires_the_intel_runtime_entry() {
        let descriptors = BTreeMap::from([(
            "aarch64-apple-darwin".to_string(),
            descriptor("speech-runtime-macos-aarch64.zip", "deadbeef"),
        )]);

        let error = validate_arch_descriptors(
            &descriptors,
            &[
                ("aarch64-apple-darwin", "speech-runtime-macos-aarch64.zip"),
                ("x86_64-apple-darwin", "speech-runtime-macos-x86_64.zip"),
                (
                    "x86_64-pc-windows-msvc",
                    "speech-runtime-windows-x86_64.zip",
                ),
            ],
            "runtime asset descriptor",
        )
        .expect_err("dual-arch releases must include an Intel runtime descriptor");

        assert!(error.contains("x86_64-apple-darwin"));
    }

    #[test]
    fn estimate_pyannote_required_free_bytes_counts_archives_and_expanded_payloads() {
        let selection = PyannoteAssetSelection {
            runtime_asset: PyannoteReleaseAsset {
                kind: "pyannote_runtime_macos_aarch64".to_string(),
                name: "pyannote-runtime.zip".to_string(),
                sha256: "deadbeef".to_string(),
                size_bytes: Some(300),
                expanded_size_bytes: Some(1000),
            },
            model_asset: PyannoteReleaseAsset {
                kind: "pyannote_model".to_string(),
                name: "pyannote-model.zip".to_string(),
                sha256: "cafebabe".to_string(),
                size_bytes: Some(30),
                expanded_size_bytes: Some(120),
            },
            compat_level: 1,
            release_version: "0.1.16".to_string(),
        };

        assert_eq!(
            estimate_pyannote_required_free_bytes(&selection),
            300 + 1000 + 30 + 120 + super::PYANNOTE_INSTALL_HEADROOM_BYTES
        );
    }
}
