use std::io::Read;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{Emitter, State};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::{error::CommandError, state::AppState};
use sbobino_infrastructure::{ManagedPyannoteManifest, RuntimeTranscriptionFactory};

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
const RELEASE_REPOSITORY: &str = "pietroMastro92/sbobbino";
const PYANNOTE_MANIFEST_ASSET: &str = "pyannote-manifest.json";
const PYANNOTE_RUNTIME_AARCH64_ASSET: &str = "pyannote-runtime-macos-aarch64.zip";
const PYANNOTE_RUNTIME_X86_64_ASSET: &str = "pyannote-runtime-macos-x86_64.zip";
const PYANNOTE_MODEL_ASSET: &str = "pyannote-model-community-1.zip";

#[derive(Debug, Clone, Deserialize)]
struct PyannoteReleaseManifest {
    app_version: String,
    assets: Vec<PyannoteReleaseAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct PyannoteReleaseAsset {
    kind: String,
    name: String,
    sha256: String,
}

#[derive(Debug, Clone)]
struct PyannoteAssetSelection {
    runtime_asset: PyannoteReleaseAsset,
    model_asset: PyannoteReleaseAsset,
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

#[derive(Debug, Serialize)]
pub struct ProvisioningStartResponse {
    pub started: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProvisioningModelCatalogEntry {
    pub key: String,
    pub label: String,
    pub model_file: String,
    pub installed: bool,
    pub coreml_installed: bool,
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

    if health.pyannote.ready && !force {
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

    spawn_pyannote_provisioning_download(
        app,
        state.runtime_factory.clone(),
        cancel_token,
        health.pyannote.ready,
    );

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

fn spawn_pyannote_provisioning_download(
    app: tauri::AppHandle,
    runtime_factory: std::sync::Arc<RuntimeTranscriptionFactory>,
    cancel_token: CancellationToken,
    had_ready_install: bool,
) {
    tauri::async_runtime::spawn(async move {
        let client = reqwest::Client::new();
        let total = 2usize;
        let runtime_dir = runtime_factory.managed_pyannote_runtime_dir();
        if let Err(error) = tokio::fs::create_dir_all(&runtime_dir).await {
            emit_provisioning_status(
                &app,
                "error",
                &format!("Failed to create pyannote runtime directory: {error}"),
                Some("pyannote_install_incomplete"),
            );
            if !had_ready_install {
                let _ = runtime_factory.write_managed_pyannote_status(
                    "pyannote_install_incomplete",
                    &format!("Failed to create pyannote runtime directory: {error}"),
                );
            }
            return;
        }

        let selection = match fetch_pyannote_asset_selection(&client).await {
            Ok(value) => value,
            Err(error) => {
                emit_provisioning_status(
                    &app,
                    "error",
                    &error,
                    Some("pyannote_install_incomplete"),
                );
                if !had_ready_install {
                    let _ = runtime_factory
                        .write_managed_pyannote_status("pyannote_install_incomplete", &error);
                }
                return;
            }
        };

        let downloads = vec![
            (
                selection.runtime_asset.clone(),
                "pyannote_runtime",
                "python",
                runtime_factory.managed_pyannote_python_dir(),
            ),
            (
                selection.model_asset.clone(),
                "pyannote_model",
                "model",
                runtime_factory.managed_pyannote_model_dir(),
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
                if !had_ready_install {
                    let _ = runtime_factory.write_managed_pyannote_status(
                        "pyannote_install_incomplete",
                        "Pyannote installation was cancelled before completion.",
                    );
                }
                return;
            }

            let url = release_asset_url(&selection.release_version, &asset.name);
            let archive_path = runtime_dir.join(format!(".download-{}", asset.name));
            if let Err(error) = download_to_path(&client, &url, &archive_path, &cancel_token).await
            {
                let _ = tokio::fs::remove_file(&archive_path).await;
                if error == "cancelled" {
                    emit_provisioning_status(
                        &app,
                        "cancelled",
                        "Pyannote installation cancelled.",
                        Some("cancelled"),
                    );
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
                if !had_ready_install {
                    let _ = runtime_factory.write_managed_pyannote_status(
                        "pyannote_install_incomplete",
                        &format!("Failed to download {}: {error}", asset.name),
                    );
                }
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
                    if !had_ready_install {
                        let _ = runtime_factory
                            .write_managed_pyannote_status("pyannote_checksum_invalid", &error);
                    }
                    return;
                }
            }

            let extraction = tokio::task::spawn_blocking({
                let archive_path = archive_path.clone();
                let runtime_dir = runtime_dir.clone();
                let destination = destination.clone();
                let expected_root = expected_root.to_string();
                move || {
                    install_pyannote_archive(
                        &archive_path,
                        &runtime_dir,
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
                    if !had_ready_install {
                        let _ = runtime_factory
                            .write_managed_pyannote_status("pyannote_install_incomplete", &error);
                    }
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
                    if !had_ready_install {
                        let _ = runtime_factory
                            .write_managed_pyannote_status("pyannote_install_incomplete", &message);
                    }
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
            runtime_asset: selection.runtime_asset.name.clone(),
            runtime_sha256: selection.runtime_asset.sha256.clone(),
            model_asset: selection.model_asset.name.clone(),
            model_sha256: selection.model_asset.sha256.clone(),
            runtime_arch: host_pyannote_arch_label().to_string(),
            installed_at: Utc::now().to_rfc3339(),
        };

        if let Err(error) = runtime_factory.write_managed_pyannote_manifest(&manifest) {
            emit_provisioning_status(&app, "error", &error, Some("pyannote_install_incomplete"));
            if !had_ready_install {
                let _ = runtime_factory
                    .write_managed_pyannote_status("pyannote_install_incomplete", &error);
            }
            return;
        }

        if let Err(error) = runtime_factory
            .write_managed_pyannote_status("ok", "Pyannote diarization runtime is ready.")
        {
            emit_provisioning_status(&app, "error", &error, Some("pyannote_install_incomplete"));
            if !had_ready_install {
                let _ = runtime_factory
                    .write_managed_pyannote_status("pyannote_install_incomplete", &error);
            }
            return;
        }

        emit_provisioning_status(
            &app,
            "completed",
            "Pyannote diarization runtime installed successfully.",
            None,
        );
    });
}

async fn fetch_pyannote_asset_selection(
    client: &reqwest::Client,
) -> Result<PyannoteAssetSelection, String> {
    let version = env!("CARGO_PKG_VERSION").to_string();
    let manifest_url = release_asset_url(&version, PYANNOTE_MANIFEST_ASSET);
    let response = client
        .get(&manifest_url)
        .send()
        .await
        .map_err(|e| format!("failed to fetch pyannote release manifest: {e}"))?
        .error_for_status()
        .map_err(|e| format!("failed to download pyannote release manifest: {e}"))?;
    let manifest = response
        .json::<PyannoteReleaseManifest>()
        .await
        .map_err(|e| format!("invalid pyannote release manifest: {e}"))?;

    if manifest.app_version.trim() != version {
        return Err(format!(
            "Pyannote manifest version '{}' does not match app version '{}'.",
            manifest.app_version.trim(),
            version
        ));
    }

    let runtime_kind = host_pyannote_runtime_kind();
    let runtime_asset = manifest
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
    let model_asset = manifest
        .assets
        .iter()
        .find(|asset| asset.kind == "pyannote_model")
        .cloned()
        .ok_or_else(|| "Pyannote release manifest is missing the model asset.".to_string())?;
    let expected_runtime_name = if runtime_kind == "pyannote_runtime_macos_x86_64" {
        PYANNOTE_RUNTIME_X86_64_ASSET
    } else {
        PYANNOTE_RUNTIME_AARCH64_ASSET
    };
    if runtime_asset.name != expected_runtime_name {
        return Err(format!(
            "Pyannote runtime asset name mismatch: expected '{}', got '{}'.",
            expected_runtime_name, runtime_asset.name
        ));
    }
    if model_asset.name != PYANNOTE_MODEL_ASSET {
        return Err(format!(
            "Pyannote model asset name mismatch: expected '{}', got '{}'.",
            PYANNOTE_MODEL_ASSET, model_asset.name
        ));
    }

    Ok(PyannoteAssetSelection {
        runtime_asset,
        model_asset,
        release_version: version,
    })
}

fn release_asset_url(version: &str, asset_name: &str) -> String {
    format!("https://github.com/{RELEASE_REPOSITORY}/releases/download/v{version}/{asset_name}")
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
        "macos-aarch64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "macos-x86_64"
    }
    #[cfg(not(target_os = "macos"))]
    {
        "macos-aarch64"
    }
}

fn verify_file_sha256(path: &Path, expected_sha256: &str) -> Result<(), String> {
    let expected = expected_sha256.trim().to_ascii_lowercase();
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
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 16 * 1024];

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
    use super::{install_pyannote_archive, sha256_file_hex, verify_file_sha256};
    use std::io::Write;
    use tempfile::tempdir;

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
}
