use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use sbobino_application::{ApplicationError, RunTranscriptionRequest};
use sbobino_domain::{
    ArtifactSourceOrigin, JobProgress, JobStage, LanguageCode, ParakeetModel, SpeechModel,
    TranscriptionEngine, WhisperOptions,
};

use crate::{
    commands::automatic_import::{
        record_automatic_import_failure, record_automatic_import_success,
        IMPORT_FOLDER_METADATA_KEY, IMPORT_PRESET_METADATA_KEY, IMPORT_SOURCE_LABEL_METADATA_KEY,
        IMPORT_WORKSPACE_METADATA_KEY,
    },
    error::CommandError,
    state::{AppState, TranscriptionTask},
};

const DELTA_REPLACE_PREFIX: &str = "\u{001F}REPLACE:";

#[derive(Debug, Clone, Deserialize)]
pub struct StartTranscriptionPayload {
    pub input_path: String,
    pub engine: TranscriptionEngine,
    pub language: LanguageCode,
    pub model: SpeechModel,
    #[serde(default)]
    pub parakeet_model: ParakeetModel,
    pub enable_ai: bool,
    #[serde(default)]
    pub whisper_options: WhisperOptions,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub source_origin: Option<ArtifactSourceOrigin>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub source_fingerprint_json: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StartTranscriptionResponse {
    pub job_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobProgressEvent {
    #[serde(flatten)]
    pub progress: JobProgress,
    pub input_path: String,
    pub title: Option<String>,
    pub source_origin: ArtifactSourceOrigin,
    pub source_label: Option<String>,
    pub source_folder: Option<String>,
    pub model: SpeechModel,
    pub language: LanguageCode,
    pub preset: Option<String>,
    pub workspace_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobFailedEvent {
    pub job_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TranscriptionDeltaEvent {
    pub job_id: String,
    pub text: String,
    pub sequence: u64,
    pub mode: String,
}

#[derive(Debug, Deserialize)]
pub struct CancelTranscriptionPayload {
    pub job_id: String,
}

#[tauri::command]
pub async fn start_transcription(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: StartTranscriptionPayload,
) -> Result<StartTranscriptionResponse, CommandError> {
    spawn_transcription_job(app, state.inner().clone(), payload).await
}

pub(crate) async fn spawn_transcription_job(
    app: tauri::AppHandle,
    state: AppState,
    payload: StartTranscriptionPayload,
) -> Result<StartTranscriptionResponse, CommandError> {
    let job_id = Uuid::new_v4().to_string();

    let request = RunTranscriptionRequest {
        job_id: job_id.clone(),
        input_path: payload.input_path,
        engine: payload.engine,
        language: payload.language,
        model: payload.model,
        parakeet_model: payload.parakeet_model,
        enable_ai: payload.enable_ai,
        whisper_options: payload.whisper_options,
        title: payload.title,
        parent_id: payload.parent_id,
        source_origin: payload
            .source_origin
            .unwrap_or(ArtifactSourceOrigin::Imported),
        metadata: payload.metadata,
        source_fingerprint_json: payload.source_fingerprint_json,
    };

    let runtime_factory = state.runtime_factory.clone();
    let app_handle = app.clone();
    let delta_app_handle = app.clone();
    let task_job_id = job_id.clone();
    let delta_job_id = job_id.clone();
    let cleanup_job_id = job_id.clone();
    let delta_sequence = Arc::new(AtomicU64::new(0));
    let cancellation_token = CancellationToken::new();
    let task_cancellation_token = cancellation_token.clone();
    let tasks = state.transcription_tasks.clone();
    let transcription_gate = state.transcription_gate.clone();
    let automatic_import_metadata = request.metadata.clone();
    let automatic_import_state = state.clone();
    let progress_input_path = request.input_path.clone();
    let progress_title = request.title.clone();
    let progress_source_origin = request.source_origin.clone();
    let progress_source_label = request
        .metadata
        .get(IMPORT_SOURCE_LABEL_METADATA_KEY)
        .cloned();
    let progress_source_folder = request.metadata.get(IMPORT_FOLDER_METADATA_KEY).cloned();
    let progress_model = request.model.clone();
    let progress_language = request.language.clone();
    let progress_preset = request.metadata.get(IMPORT_PRESET_METADATA_KEY).cloned();
    let progress_workspace_id = request.metadata.get(IMPORT_WORKSPACE_METADATA_KEY).cloned();

    tauri::async_runtime::spawn(async move {
        let progress_input_path = progress_input_path.clone();
        let progress_title = progress_title.clone();
        let progress_source_origin = progress_source_origin.clone();
        let progress_source_label = progress_source_label.clone();
        let progress_source_folder = progress_source_folder.clone();
        let progress_model = progress_model.clone();
        let progress_language = progress_language.clone();
        let progress_preset = progress_preset.clone();
        let progress_workspace_id = progress_workspace_id.clone();
        let emit_progress = Arc::new(move |progress: JobProgress| {
            let _ = app_handle.emit(
                "transcription://progress",
                JobProgressEvent {
                    progress,
                    input_path: progress_input_path.clone(),
                    title: progress_title.clone(),
                    source_origin: progress_source_origin.clone(),
                    source_label: progress_source_label.clone(),
                    source_folder: progress_source_folder.clone(),
                    model: progress_model.clone(),
                    language: progress_language.clone(),
                    preset: progress_preset.clone(),
                    workspace_id: progress_workspace_id.clone(),
                },
            );
        });
        let delta_sequence = delta_sequence.clone();
        let emit_delta = Arc::new(move |text: String| {
            let (mode, normalized_text) =
                if let Some(snapshot) = text.strip_prefix(DELTA_REPLACE_PREFIX) {
                    ("replace".to_string(), snapshot.to_string())
                } else {
                    ("append".to_string(), text)
                };
            let sequence = delta_sequence.fetch_add(1, Ordering::Relaxed);
            let _ = delta_app_handle.emit(
                "transcription://delta",
                TranscriptionDeltaEvent {
                    job_id: delta_job_id.clone(),
                    text: normalized_text,
                    sequence,
                    mode,
                },
            );
        });

        // Serialize heavy work. If another job is already running, emit a
        // Queued progress event so the UI can show "Waiting" and the user
        // can still cancel. Dropping `_permit` at end of this block releases
        // the gate for the next queued job.
        let _permit = match transcription_gate.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                emit_progress(JobProgress {
                    job_id: task_job_id.clone(),
                    stage: JobStage::Queued,
                    message: "Waiting for previous transcription to finish".to_string(),
                    percentage: 0,
                    current_seconds: None,
                    total_seconds: None,
                });
                tokio::select! {
                    biased;
                    _ = task_cancellation_token.cancelled() => {
                        let mut registry = tasks.lock().await;
                        registry.remove(&cleanup_job_id);
                        return;
                    }
                    permit = transcription_gate.clone().acquire_owned() => match permit {
                        Ok(permit) => permit,
                        Err(_) => {
                            let _ = app.emit(
                                "transcription://failed",
                                JobFailedEvent {
                                    job_id: task_job_id.clone(),
                                    message: "Transcription gate closed unexpectedly".to_string(),
                                },
                            );
                            let mut registry = tasks.lock().await;
                            registry.remove(&cleanup_job_id);
                            return;
                        }
                    },
                }
            }
        };

        if task_cancellation_token.is_cancelled() {
            let mut registry = tasks.lock().await;
            registry.remove(&cleanup_job_id);
            return;
        }

        let transcription_service = match runtime_factory.build_service() {
            Ok(service) => service,
            Err(error) => {
                let _ = app.emit(
                    "transcription://failed",
                    JobFailedEvent {
                        job_id: task_job_id.clone(),
                        message: format!("Transcription runtime unavailable: {error}"),
                    },
                );
                let mut registry = tasks.lock().await;
                registry.remove(&cleanup_job_id);
                return;
            }
        };

        match transcription_service
            .run_file_transcription(request, emit_progress, emit_delta, task_cancellation_token)
            .await
        {
            Ok(artifact) => {
                let _ = record_automatic_import_success(
                    &automatic_import_state,
                    &automatic_import_metadata,
                )
                .await;
                let _ = app.emit("transcription://completed", artifact);
            }
            Err(ApplicationError::Cancelled) => {}
            Err(error) => {
                let _ = record_automatic_import_failure(
                    &automatic_import_state,
                    &automatic_import_metadata,
                    &error.to_string(),
                )
                .await;
                let _ = app.emit(
                    "transcription://failed",
                    JobFailedEvent {
                        job_id: task_job_id,
                        message: error.to_string(),
                    },
                );
            }
        }

        let mut registry = tasks.lock().await;
        registry.remove(&cleanup_job_id);
    });

    let mut registry = state.transcription_tasks.lock().await;
    registry.insert(
        job_id.clone(),
        TranscriptionTask {
            cancel_token: cancellation_token,
        },
    );

    Ok(StartTranscriptionResponse { job_id })
}

#[tauri::command]
pub async fn cancel_transcription(
    state: State<'_, AppState>,
    payload: CancelTranscriptionPayload,
) -> Result<(), CommandError> {
    let task = {
        let mut registry = state.transcription_tasks.lock().await;
        registry.remove(&payload.job_id)
    };

    if let Some(task) = task {
        task.cancel_token.cancel();
    }

    Ok(())
}
