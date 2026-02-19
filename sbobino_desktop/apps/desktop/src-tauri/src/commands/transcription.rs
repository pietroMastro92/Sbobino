use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use sbobino_application::{ApplicationError, RunTranscriptionRequest};
use sbobino_domain::{JobProgress, LanguageCode, SpeechModel, WhisperOptions};

use crate::{
    error::CommandError,
    state::{AppState, TranscriptionTask},
};

#[derive(Debug, Deserialize)]
pub struct StartTranscriptionPayload {
    pub input_path: String,
    pub language: LanguageCode,
    pub model: SpeechModel,
    pub enable_ai: bool,
    #[serde(default)]
    pub whisper_options: WhisperOptions,
}

#[derive(Debug, Serialize)]
pub struct StartTranscriptionResponse {
    pub job_id: String,
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
    let job_id = Uuid::new_v4().to_string();

    let request = RunTranscriptionRequest {
        job_id: job_id.clone(),
        input_path: payload.input_path,
        language: payload.language,
        model: payload.model,
        enable_ai: payload.enable_ai,
        whisper_options: payload.whisper_options,
    };

    let transcription_service = state
        .runtime_factory
        .build_service()
        .map_err(|error| CommandError::new("runtime_factory", error))?;
    let app_handle = app.clone();
    let delta_app_handle = app.clone();
    let task_job_id = job_id.clone();
    let delta_job_id = job_id.clone();
    let cleanup_job_id = job_id.clone();
    let delta_sequence = Arc::new(AtomicU64::new(0));
    let cancellation_token = CancellationToken::new();
    let task_cancellation_token = cancellation_token.clone();
    let tasks = state.transcription_tasks.clone();

    tauri::async_runtime::spawn(async move {
        let emit_progress = Arc::new(move |progress: JobProgress| {
            let _ = app_handle.emit("transcription://progress", progress);
        });
        let delta_sequence = delta_sequence.clone();
        let emit_delta = Arc::new(move |text: String| {
            let sequence = delta_sequence.fetch_add(1, Ordering::Relaxed);
            let _ = delta_app_handle.emit(
                "transcription://delta",
                TranscriptionDeltaEvent {
                    job_id: delta_job_id.clone(),
                    text,
                    sequence,
                },
            );
        });

        match transcription_service
            .run_file_transcription(request, emit_progress, emit_delta, task_cancellation_token)
            .await
        {
            Ok(artifact) => {
                let _ = app.emit("transcription://completed", artifact);
            }
            Err(ApplicationError::Cancelled) => {}
            Err(error) => {
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
