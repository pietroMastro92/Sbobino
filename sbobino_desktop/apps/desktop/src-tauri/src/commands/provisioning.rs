use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{Emitter, State};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::{
    error::CommandError,
    release_assets::{
        release_asset_url, release_tag, PyannoteReleaseAsset, PyannoteReleaseManifest,
        ReleaseAssetDescriptor, RuntimeReleaseAsset, RuntimeReleaseManifest, SetupReleaseManifest,
        PYANNOTE_COMPAT_LEVEL, PYANNOTE_MANIFEST_ASSET, PYANNOTE_MODEL_ASSET,
        PYANNOTE_RUNTIME_AARCH64_ASSET, PYANNOTE_RUNTIME_X86_64_ASSET, RUNTIME_AARCH64_ASSET,
        RUNTIME_MANIFEST_ASSET, SETUP_MANIFEST_ASSET,
    },
    state::AppState,
};
use sbobino_infrastructure::{
    ManagedPyannoteManifest, ManagedRuntimeHealth, ReconcileManagedPyannoteReleaseOutcome,
    RuntimeTranscriptionFactory, PYANNOTE_MANIFEST_FILENAME,
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
const LOCAL_RELEASE_ASSETS_DIR_ENV: &str = "SBOBINO_LOCAL_RELEASE_ASSETS_DIR";

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
    if !had_ready_install {
        if let Err(error) = cleanup_pyannote_workdir(runtime_factory) {
            tracing::warn!("failed to clean up incomplete pyannote install: {error}");
        }
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
}

fn format_managed_runtime_install_error(health: &ManagedRuntimeHealth) -> String {
    let failing_tool = if !health.ffmpeg.available {
        Some(("FFmpeg", &health.ffmpeg))
    } else if !health.whisper_cli.available {
        Some(("Whisper CLI", &health.whisper_cli))
    } else if !health.whisper_stream.available {
        Some(("Whisper Stream", &health.whisper_stream))
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
        .runtime_health()
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

    Ok(MODEL_CATALOG
        .iter()
        .map(|(key, label, model_file, encoder_dir, _encoder_archive)| {
            ProvisioningModelCatalogEntry {
                key: (*key).to_string(),
                label: (*label).to_string(),
                model_file: (*model_file).to_string(),
                installed: models_dir.join(model_file).exists(),
                coreml_installed: models_dir.join(encoder_dir).is_dir(),
            }
        })
        .collect())
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

async fn plan_pyannote_background_action_inner(
    runtime_factory: &std::sync::Arc<RuntimeTranscriptionFactory>,
    trigger: PyannoteBackgroundActionTrigger,
) -> Result<PyannoteBackgroundActionResponse, CommandError> {
    let manifest_before = runtime_factory.read_managed_pyannote_manifest();
    let status_before = runtime_factory.read_managed_pyannote_status();

    if matches!(trigger, PyannoteBackgroundActionTrigger::PostUpdate)
        && should_attempt_post_update_pyannote_reconcile(manifest_before.as_ref())
    {
        let client = reqwest::Client::new();
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
            match outcome {
                ReconcileManagedPyannoteReleaseOutcome::NoAction => {}
                ReconcileManagedPyannoteReleaseOutcome::ManifestUpdated => {
                    return Ok(pyannote_background_action_response(
                        "migrate_manifest",
                        false,
                        false,
                        "pyannote_manifest_migrated",
                        "Pyannote metadata was updated for this app version.",
                    ));
                }
                ReconcileManagedPyannoteReleaseOutcome::NeedsMigration { message } => {
                    let reason_code = infer_pyannote_reconcile_reason_code(&message);
                    return Ok(pyannote_background_action_response(
                        "migrate_assets",
                        true,
                        true,
                        reason_code,
                        message,
                    ));
                }
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

    let cancel_token = CancellationToken::new();
    *state.provisioning.cancel_token.lock().await = Some(cancel_token.clone());

    spawn_provisioning_download(
        app,
        models_dir,
        missing_models,
        missing_encoders,
        cancel_token,
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

    let cancel_token = CancellationToken::new();
    *state.provisioning.cancel_token.lock().await = Some(cancel_token.clone());

    spawn_provisioning_download(
        app,
        models_dir,
        missing_models,
        missing_encoders,
        cancel_token,
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

    let cancel_token = CancellationToken::new();
    *state.provisioning.cancel_token.lock().await = Some(cancel_token.clone());

    if state.runtime_factory.has_bundled_pyannote_override_assets() {
        spawn_pyannote_bundled_install(
            app,
            state.runtime_factory.clone(),
            cancel_token,
            repair_required,
        );
    } else {
        spawn_pyannote_provisioning_download(
            app,
            state.runtime_factory.clone(),
            cancel_token,
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
    let runtime_ready =
        health.ffmpeg_available && health.whisper_cli_available && health.whisper_stream_available;

    if runtime_ready && !force {
        emit_provisioning_status(
            &app,
            "completed",
            "Local transcription runtime is already installed.",
            None,
        );
        return Ok(ProvisioningStartResponse { started: false });
    }

    let cancel_token = CancellationToken::new();
    *state.provisioning.cancel_token.lock().await = Some(cancel_token.clone());

    spawn_runtime_provisioning_download(app, state.runtime_factory.clone(), cancel_token);

    Ok(ProvisioningStartResponse { started: true })
}

#[tauri::command]
pub async fn provisioning_cancel(state: State<'_, AppState>) -> Result<(), CommandError> {
    let token = {
        let mut guard = state.provisioning.cancel_token.lock().await;
        guard.take()
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
    models_dir: PathBuf,
    missing_models: Vec<String>,
    missing_encoders: Vec<(String, String)>,
    cancel_token: CancellationToken,
) {
    let total = missing_models.len() + missing_encoders.len();

    tauri::async_runtime::spawn(async move {
        let client = reqwest::Client::new();
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

            let url = format!("{MODEL_BASE_URL}{model}");
            let destination = models_dir.join(&model);
            match download_to_path(&client, &url, &destination, &cancel_token).await {
                Ok(()) => emit_progress(model, "whisper_model", "downloaded".to_string()),
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
                move || extract_zip_archive(&archive_path, &models_dir)
            })
            .await;

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

            let _ = tokio::fs::remove_file(&archive_path).await;
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

fn spawn_pyannote_bundled_install(
    app: tauri::AppHandle,
    runtime_factory: std::sync::Arc<RuntimeTranscriptionFactory>,
    cancel_token: CancellationToken,
    _repair_required: bool,
) {
    tauri::async_runtime::spawn(async move {
        if cancel_token.is_cancelled() {
            emit_provisioning_status(
                &app,
                "cancelled",
                "Pyannote installation cancelled.",
                Some("cancelled"),
            );
            return;
        }

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
            emit_provisioning_status(&app, "error", &error, Some("pyannote_repair_required"));
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

        match runtime_factory.runtime_health() {
            Ok(health) if health.pyannote.ready => emit_provisioning_status(
                &app,
                "completed",
                "Pyannote diarization runtime installed successfully.",
                None,
            ),
            Ok(health) => emit_provisioning_status(
                &app,
                "error",
                &health.pyannote.message,
                Some(health.pyannote.reason_code.as_str()),
            ),
            Err(error) => {
                emit_provisioning_status(&app, "error", &error, Some("pyannote_repair_required"))
            }
        }
    });
}

fn spawn_pyannote_provisioning_download(
    app: tauri::AppHandle,
    runtime_factory: std::sync::Arc<RuntimeTranscriptionFactory>,
    cancel_token: CancellationToken,
    had_ready_install: bool,
    reset_existing_install: bool,
) {
    tauri::async_runtime::spawn(async move {
        let client = reqwest::Client::new();
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

        match runtime_factory.runtime_health() {
            Ok(health) if health.pyannote.ready => {
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
            Ok(health) => {
                let reason_code = if health.pyannote.reason_code.trim().is_empty() {
                    "pyannote_install_incomplete"
                } else {
                    health.pyannote.reason_code.as_str()
                };
                let message = if health.pyannote.message.trim().is_empty() {
                    "Pyannote diarization runtime could not be validated after installation."
                } else {
                    health.pyannote.message.as_str()
                };
                emit_provisioning_status(&app, "error", message, Some(reason_code));
                if let Err(restore_error) =
                    rollback_pyannote_runtime_swap(&runtime_dir, backup_runtime_dir.as_deref())
                {
                    tracing::warn!("failed to rollback pyannote runtime after health validation error: {restore_error}");
                }
                persist_pyannote_install_failure(
                    runtime_factory.as_ref(),
                    had_ready_install,
                    reason_code,
                    message,
                );
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
) {
    tauri::async_runtime::spawn(async move {
        let client = reqwest::Client::new();
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

        match extraction {
            Ok(Ok(())) => {}
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
        }

        let managed_runtime = runtime_factory.managed_runtime_health();
        if !managed_runtime.ready {
            emit_provisioning_status(
                &app,
                "error",
                &format_managed_runtime_install_error(&managed_runtime),
                Some("runtime_install_incomplete"),
            );
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
    validate_manifest_asset_descriptor(
        &bundle.setup_manifest.pyannote_runtime_asset,
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
    let runtime_asset = bundle
        .runtime_manifest
        .assets
        .iter()
        .find(|asset| asset.kind == "speech_runtime_macos_aarch64")
        .cloned()
        .ok_or_else(|| {
            "Runtime release manifest is missing the speech runtime asset.".to_string()
        })?;
    validate_manifest_asset_descriptor(
        &bundle.setup_manifest.runtime_asset,
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
        tokio::fs::copy(&source, destination).await.map_err(|e| {
            format!(
                "failed to stage local release asset '{}': {e}",
                source.display()
            )
        })?;
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
    validate_release_descriptor_name(
        &manifest.runtime_asset,
        RUNTIME_AARCH64_ASSET,
        "runtime asset descriptor",
    )?;
    validate_release_descriptor_name(
        &manifest.pyannote_manifest,
        PYANNOTE_MANIFEST_ASSET,
        "pyannote manifest descriptor",
    )?;
    validate_release_descriptor_name(
        &manifest.pyannote_runtime_asset,
        if host_pyannote_runtime_kind() == "pyannote_runtime_macos_x86_64" {
            PYANNOTE_RUNTIME_X86_64_ASSET
        } else {
            PYANNOTE_RUNTIME_AARCH64_ASSET
        },
        "pyannote runtime asset descriptor",
    )?;
    validate_release_descriptor_name(
        &manifest.pyannote_model_asset,
        PYANNOTE_MODEL_ASSET,
        "pyannote model asset descriptor",
    )?;

    Ok(())
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
    #[cfg(not(target_os = "macos"))]
    {
        "pyannote_runtime_macos_aarch64"
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
    #[cfg(not(target_os = "macos"))]
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

fn install_runtime_archive(
    archive_path: &Path,
    runtime_dir: &Path,
    destination: &Path,
) -> Result<(), String> {
    let stage_dir = runtime_dir.join(".stage-runtime");
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

    let install_root = destination.parent().ok_or_else(|| {
        format!(
            "failed to determine runtime install root from '{}'.",
            destination.display()
        )
    })?;
    let lib_destination = install_root.join("lib");

    remove_path_if_exists(destination)?;
    remove_path_if_exists(&lib_destination)?;
    std::fs::create_dir_all(install_root)
        .map_err(|e| format!("failed to create runtime install root: {e}"))?;

    std::fs::rename(&staged_bin, destination)
        .map_err(|e| format!("failed to move staged runtime binaries into place: {e}"))?;
    std::fs::rename(&staged_lib, &lib_destination)
        .map_err(|e| format!("failed to move staged runtime libraries into place: {e}"))?;

    remove_path_if_exists(&stage_dir)?;
    Ok(())
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

async fn download_to_path(
    client: &reqwest::Client,
    url: &str,
    destination: &Path,
    cancel_token: &CancellationToken,
) -> Result<(), String> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?
        .error_for_status()
        .map_err(|e| format!("download failed: {e}"))?;

    let mut file = tokio::fs::File::create(destination)
        .await
        .map_err(|e| format!("failed to create destination file: {e}"))?;

    let mut response = response;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("download stream failure: {e}"))?
    {
        if cancel_token.is_cancelled() {
            let _ = tokio::fs::remove_file(destination).await;
            return Err("cancelled".to_string());
        }

        file.write_all(&chunk)
            .await
            .map_err(|e| format!("failed to write chunk: {e}"))?;
    }

    file.flush()
        .await
        .map_err(|e| format!("failed to flush destination file: {e}"))?;

    Ok(())
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
        estimate_pyannote_required_free_bytes, install_pyannote_archive, install_runtime_archive,
        plan_pyannote_background_action_inner, prepare_pyannote_runtime_stage,
        prepare_pyannote_runtime_swap, promote_staged_pyannote_runtime,
        rollback_pyannote_runtime_swap, sha256_file_hex, validate_manifest_asset_descriptor,
        validate_setup_manifest, verify_file_sha256, PyannoteAssetSelection,
        PyannoteBackgroundActionTrigger,
    };
    use crate::release_assets::{
        PyannoteReleaseAsset, PyannoteReleaseManifest, ReleaseAssetDescriptor, RuntimeReleaseAsset,
        RuntimeReleaseManifest, SetupReleaseManifest, PYANNOTE_COMPAT_LEVEL,
        PYANNOTE_MANIFEST_ASSET, RUNTIME_MANIFEST_ASSET, SETUP_MANIFEST_ASSET,
    };
    use sbobino_domain::AppSettings;
    use sbobino_infrastructure::{ManagedPyannoteManifest, RuntimeTranscriptionFactory};
    use std::io::Write;
    use std::sync::{Arc, Mutex, OnceLock};
    use tempfile::tempdir;

    fn release_assets_env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

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
        let stdlib_root = runtime_root.join("lib").join(version_dir_name);
        std::fs::create_dir_all(stdlib_root.join("encodings"))
            .expect("stdlib encodings dir should exist");
        std::fs::create_dir_all(stdlib_root.join("lib-dynload"))
            .expect("stdlib lib-dynload dir should exist");
        std::fs::create_dir_all(stdlib_root.join("collections"))
            .expect("stdlib collections dir should exist");
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

    fn write_local_pyannote_release_manifests(
        root: &std::path::Path,
        runtime_sha256: &str,
        model_sha256: &str,
    ) {
        let runtime_manifest = RuntimeReleaseManifest {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            assets: vec![RuntimeReleaseAsset {
                kind: "speech_runtime_macos_aarch64".to_string(),
                name: "speech-runtime-macos-aarch64.zip".to_string(),
                sha256: "runtime-sha".to_string(),
                size_bytes: None,
                expanded_size_bytes: None,
            }],
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
                    kind: super::host_pyannote_runtime_kind().to_string(),
                    name: "pyannote-runtime-macos-aarch64.zip".to_string(),
                    sha256: runtime_sha256.to_string(),
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
            runtime_asset: descriptor("speech-runtime-macos-aarch64.zip", "runtime-sha"),
            pyannote_manifest: descriptor(PYANNOTE_MANIFEST_ASSET, &pyannote_manifest_sha),
            pyannote_runtime_asset: descriptor(
                "pyannote-runtime-macos-aarch64.zip",
                runtime_sha256,
            ),
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

    #[tokio::test]
    async fn plan_pyannote_background_action_skips_missing_install_when_diarization_disabled() {
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
        assert_eq!(action.reason_code, "pyannote_disabled");
    }

    #[tokio::test]
    async fn plan_pyannote_background_action_installs_missing_runtime_when_diarization_enabled() {
        let (_temp, factory) = build_runtime_factory();
        persist_settings(&factory, true);

        let action = plan_pyannote_background_action_inner(
            &factory,
            PyannoteBackgroundActionTrigger::Startup,
        )
        .await
        .expect("planner should succeed");

        assert_eq!(action.status, "install_missing");
        assert!(action.should_start);
        assert!(!action.force_reinstall);
        assert_eq!(action.reason_code, "pyannote_runtime_missing");
    }

    #[tokio::test]
    async fn plan_pyannote_background_action_reports_real_compat_mismatch_as_asset_migration() {
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

        assert_eq!(action.status, "migrate_assets");
        assert!(action.should_start);
        assert!(action.force_reinstall);
        assert_eq!(action.reason_code, "pyannote_version_mismatch");
    }

    #[tokio::test]
    async fn plan_pyannote_background_action_self_heals_stale_incomplete_status() {
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
        assert_eq!(action.reason_code, "ok");
    }

    #[tokio::test]
    async fn plan_pyannote_background_action_requests_manifest_only_migration_on_patch_update() {
        let (_guard, _temp, factory) = {
            let guard = release_assets_env_lock()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let (temp, factory) = build_runtime_factory();
            (guard, temp, factory)
        };
        persist_settings(&factory, true);
        prepare_ready_pyannote_install(
            &factory,
            ManagedPyannoteManifest {
                source: "release_asset".to_string(),
                app_version: "0.1.0".to_string(),
                compat_level: PYANNOTE_COMPAT_LEVEL,
                runtime_asset: "pyannote-runtime-macos-aarch64.zip".to_string(),
                runtime_sha256: "runtime-sha".to_string(),
                model_asset: "pyannote-model-community-1.zip".to_string(),
                model_sha256: "model-sha".to_string(),
                runtime_arch: super::host_pyannote_arch_label().to_string(),
                installed_at: "2026-04-21T00:00:00Z".to_string(),
            },
            "ok",
        );

        let release_assets_dir = factory.data_dir().join("release-assets");
        std::fs::create_dir_all(&release_assets_dir).expect("release assets dir should exist");
        write_local_pyannote_release_manifests(&release_assets_dir, "runtime-sha", "model-sha");
        std::env::set_var(super::LOCAL_RELEASE_ASSETS_DIR_ENV, &release_assets_dir);

        let action = plan_pyannote_background_action_inner(
            &factory,
            PyannoteBackgroundActionTrigger::PostUpdate,
        )
        .await
        .expect("planner should succeed");

        std::env::remove_var(super::LOCAL_RELEASE_ASSETS_DIR_ENV);

        assert_eq!(action.status, "migrate_manifest");
        assert!(!action.should_start);
        assert!(!action.force_reinstall);
        assert_eq!(action.reason_code, "pyannote_manifest_migrated");
    }

    #[tokio::test]
    async fn plan_pyannote_background_action_requests_asset_migration_on_checksum_mismatch() {
        let (_guard, _temp, factory) = {
            let guard = release_assets_env_lock()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let (temp, factory) = build_runtime_factory();
            (guard, temp, factory)
        };
        persist_settings(&factory, true);
        prepare_ready_pyannote_install(
            &factory,
            ManagedPyannoteManifest {
                source: "release_asset".to_string(),
                app_version: "0.1.0".to_string(),
                compat_level: PYANNOTE_COMPAT_LEVEL,
                runtime_asset: "pyannote-runtime-macos-aarch64.zip".to_string(),
                runtime_sha256: "installed-runtime-sha".to_string(),
                model_asset: "pyannote-model-community-1.zip".to_string(),
                model_sha256: "installed-model-sha".to_string(),
                runtime_arch: super::host_pyannote_arch_label().to_string(),
                installed_at: "2026-04-21T00:00:00Z".to_string(),
            },
            "ok",
        );

        let release_assets_dir = factory.data_dir().join("release-assets");
        std::fs::create_dir_all(&release_assets_dir).expect("release assets dir should exist");
        write_local_pyannote_release_manifests(
            &release_assets_dir,
            "expected-runtime-sha",
            "expected-model-sha",
        );
        std::env::set_var(super::LOCAL_RELEASE_ASSETS_DIR_ENV, &release_assets_dir);

        let action = plan_pyannote_background_action_inner(
            &factory,
            PyannoteBackgroundActionTrigger::PostUpdate,
        )
        .await
        .expect("planner should succeed");

        std::env::remove_var(super::LOCAL_RELEASE_ASSETS_DIR_ENV);

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
        install_runtime_archive(&archive_path, &runtime_dir, &destination)
            .expect("runtime should install");

        let installed_binary = destination.join("whisper-cli");
        let installed_library = runtime_dir.join("lib").join("libwhisper.dylib");
        assert!(installed_binary.is_file());
        assert!(installed_library.is_file());

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
            runtime_asset: descriptor("speech-runtime-macos-aarch64.zip", "deadbeef"),
            pyannote_manifest: descriptor("pyannote-manifest.json", "deadbeef"),
            pyannote_runtime_asset: descriptor("pyannote-runtime-macos-aarch64.zip", "deadbeef"),
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
