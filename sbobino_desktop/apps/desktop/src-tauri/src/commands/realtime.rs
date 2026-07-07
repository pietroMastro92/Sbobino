use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use tokio::time::{sleep, Duration};
use uuid::Uuid;

use sbobino_application::{ApplicationError, RealtimeDelta};
use sbobino_domain::{
    ArtifactKind, ArtifactSourceOrigin, JobProgress, JobStage, LanguageCode, ParakeetModel,
    SpeechModel, TranscriptArtifact, TranscriptionEngine,
};

use crate::commands::transcription::{JobFailedEvent, JobProgressEvent};
use crate::realtime_audio::start_input_preview;
use crate::{error::CommandError, state::AppState};

const REALTIME_INPUT_PATH: &str = "realtime://microphone";
const REALTIME_SOURCE_LABEL: &str = "Live microphone";

fn resolve_realtime_engine(
    state: &AppState,
) -> Result<sbobino_infrastructure::adapters::whisper_stream::WhisperStreamEngine, CommandError> {
    match state.runtime_factory.build_whisper_stream_engine() {
        Ok(engine) => Ok(engine),
        Err(error) => {
            if state.runtime_factory.managed_runtime_required() {
                return Err(CommandError::from(ApplicationError::SpeechToText(error)));
            }

            let settings = state
                .runtime_factory
                .load_settings()
                .map_err(|load_error| CommandError::new("settings", load_error))?;
            let whisper_stream_path = state.runtime_factory.resolve_binary_path(
                &settings.transcription.whisperkit_cli_path,
                "whisper-stream",
            );
            let models_dir = state
                .runtime_factory
                .resolve_models_dir(&settings.transcription.models_dir);
            Ok(
                sbobino_infrastructure::adapters::whisper_stream::WhisperStreamEngine::new(
                    whisper_stream_path,
                    models_dir,
                ),
            )
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct StartRealtimePayload {
    #[serde(default)]
    pub engine: Option<TranscriptionEngine>,
    pub model: Option<SpeechModel>,
    #[serde(default)]
    pub parakeet_model: Option<ParakeetModel>,
    pub language: Option<LanguageCode>,
    pub resume_artifact_id: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StartRealtimeResponse {
    pub started: bool,
    pub job_id: String,
}

#[derive(Debug, Deserialize)]
pub struct StopRealtimePayload {
    pub save: Option<bool>,
    pub title: Option<String>,
    pub elapsed_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct StopRealtimeResponse {
    pub saved: bool,
    pub queued: bool,
    pub job_id: Option<String>,
    pub artifact: Option<TranscriptArtifact>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RealtimeStatusEvent {
    pub state: String,
    pub message: String,
}

#[derive(Debug, Clone)]
struct RealtimeArtifactInput {
    job_id: String,
    session_title: String,
    language_code: String,
    model_filename: String,
    elapsed_seconds: Option<u64>,
    transcript: String,
    saved_audio_path: Option<PathBuf>,
}

fn clean_optional_title(title: Option<String>) -> Option<String> {
    title
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn fallback_live_title() -> String {
    format!("live_{}", Utc::now().format("%d%m%Y_%H%M%S"))
}

fn realtime_job_progress(
    job_id: &str,
    stage: JobStage,
    message: impl Into<String>,
    percentage: u8,
    current_seconds: Option<f32>,
    total_seconds: Option<f32>,
) -> JobProgress {
    JobProgress {
        job_id: job_id.to_string(),
        stage,
        message: message.into(),
        percentage,
        current_seconds,
        total_seconds,
    }
}

#[allow(clippy::too_many_arguments)]
fn realtime_progress_event(
    progress: JobProgress,
    title: Option<String>,
    model: SpeechModel,
    language: LanguageCode,
) -> JobProgressEvent {
    JobProgressEvent {
        progress,
        input_path: REALTIME_INPUT_PATH.to_string(),
        title,
        source_origin: ArtifactSourceOrigin::Realtime,
        source_label: Some(REALTIME_SOURCE_LABEL.to_string()),
        source_folder: None,
        model,
        language,
        preset: None,
        workspace_id: None,
    }
}

fn emit_realtime_progress(
    app: &tauri::AppHandle,
    progress: JobProgress,
    title: Option<String>,
    model: SpeechModel,
    language: LanguageCode,
) {
    let _ = app.emit(
        "transcription://progress",
        realtime_progress_event(progress, title, model, language),
    );
}

fn build_realtime_artifact(
    input: RealtimeArtifactInput,
) -> Result<TranscriptArtifact, ApplicationError> {
    let mut metadata = BTreeMap::new();
    metadata.insert("kind".to_string(), "realtime".to_string());
    metadata.insert("language".to_string(), input.language_code.clone());
    metadata.insert("model".to_string(), input.model_filename.clone());
    if let Some(elapsed_seconds) = input.elapsed_seconds {
        metadata.insert("duration_seconds".to_string(), elapsed_seconds.to_string());
    }
    metadata.insert(
        "audio_saved".to_string(),
        if input.saved_audio_path.is_some() {
            "true".to_string()
        } else {
            "false".to_string()
        },
    );

    let source_label = input
        .saved_audio_path
        .as_ref()
        .and_then(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| format!("{}.wav", input.session_title));

    let mut artifact = TranscriptArtifact::new(
        input.job_id,
        input.session_title,
        ArtifactKind::Realtime,
        source_label,
        ArtifactSourceOrigin::Realtime,
        input.transcript,
        String::new(),
        String::new(),
        String::new(),
        metadata,
    )
    .map_err(|e| ApplicationError::Validation(e.to_string()))?;
    artifact.audio_available = input.saved_audio_path.is_some();
    artifact.audio_duration_seconds = input.elapsed_seconds.map(|value| value as f32);
    artifact.processing_engine = Some("whisper_stream".to_string());
    artifact.processing_model = Some(input.model_filename);
    artifact.processing_language = Some(input.language_code);
    if let Some(path) = input.saved_audio_path.as_ref() {
        artifact.set_source_external_path(path.to_string_lossy().to_string());
    }

    Ok(artifact)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_realtime_artifact_preserves_live_job_metadata() {
        let artifact = build_realtime_artifact(RealtimeArtifactInput {
            job_id: "live-job-1".to_string(),
            session_title: "Daily standup".to_string(),
            language_code: "it".to_string(),
            model_filename: "ggml-large-v3-turbo.bin".to_string(),
            elapsed_seconds: Some(3725),
            transcript: "Ciao a tutti.".to_string(),
            saved_audio_path: Some(PathBuf::from("/tmp/sbobino-live/session.wav")),
        })
        .expect("realtime artifact should be valid");

        assert_eq!(artifact.job_id, "live-job-1");
        assert_eq!(artifact.title, "Daily standup");
        assert_eq!(artifact.kind, ArtifactKind::Realtime);
        assert_eq!(artifact.source_origin, ArtifactSourceOrigin::Realtime);
        assert_eq!(artifact.source_label, "session.wav");
        assert_eq!(artifact.raw_transcript, "Ciao a tutti.");
        assert_eq!(
            artifact.processing_engine.as_deref(),
            Some("whisper_stream")
        );
        assert_eq!(
            artifact.processing_model.as_deref(),
            Some("ggml-large-v3-turbo.bin")
        );
        assert_eq!(artifact.processing_language.as_deref(), Some("it"));
        assert_eq!(artifact.audio_duration_seconds, Some(3725.0));
        assert_eq!(
            artifact
                .metadata
                .get("duration_seconds")
                .map(String::as_str),
            Some("3725")
        );
        assert_eq!(
            artifact.metadata.get("audio_saved").map(String::as_str),
            Some("true")
        );
    }

    #[test]
    fn build_realtime_artifact_rejects_empty_live_transcript() {
        let error = build_realtime_artifact(RealtimeArtifactInput {
            job_id: "live-job-2".to_string(),
            session_title: "Empty live".to_string(),
            language_code: "en".to_string(),
            model_filename: "ggml-base.bin".to_string(),
            elapsed_seconds: None,
            transcript: "  ".to_string(),
            saved_audio_path: None,
        })
        .expect_err("empty live transcript should not produce an artifact");

        assert!(error.to_string().contains("transcript"));
    }
}

async fn stop_realtime_preview(
    app: &tauri::AppHandle,
    state: &AppState,
    final_state: &str,
    message: &str,
) {
    if let Some(preview) = state.realtime.preview.lock().await.take() {
        preview.stop(app, final_state, message);
    }
}

async fn start_realtime_preview(
    app: &tauri::AppHandle,
    state: &AppState,
) -> Result<(), CommandError> {
    stop_realtime_preview(app, state, "idle", "Microphone preview reset.").await;
    let preview = start_input_preview(app)
        .map_err(|error| CommandError::from(ApplicationError::SpeechToText(error.message)))?;
    *state.realtime.preview.lock().await = Some(preview);
    Ok(())
}

#[tauri::command]
pub async fn start_realtime(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: Option<StartRealtimePayload>,
) -> Result<StartRealtimeResponse, CommandError> {
    let payload = payload.unwrap_or(StartRealtimePayload {
        engine: None,
        model: None,
        parakeet_model: None,
        language: None,
        resume_artifact_id: None,
        title: None,
    });

    let settings = state
        .runtime_factory
        .load_settings()
        .map_err(|e| CommandError::new("settings", e))?;

    let default_model = settings.transcription.model;
    let default_language = settings.transcription.language;
    let engine_kind = payload
        .engine
        .unwrap_or_else(|| settings.transcription.engine.clone());
    let model = payload.model.unwrap_or(default_model);
    let language = payload.language.unwrap_or(default_language);
    let job_id = Uuid::new_v4().to_string();
    let requested_title = clean_optional_title(payload.title.clone());
    let mut session_title = requested_title.clone().unwrap_or_else(fallback_live_title);

    if engine_kind == TranscriptionEngine::ParakeetCpp {
        let parakeet_model = payload
            .parakeet_model
            .unwrap_or(settings.transcription.parakeet_model);
        return Err(CommandError::from(ApplicationError::SpeechToText(format!(
            "Parakeet.cpp live transcription is not enabled yet for '{}'. File transcription works with Parakeet; live requires the parakeet.cpp C API streaming path and must not fall back to Whisper.",
            parakeet_model.gguf_filename()
        ))));
    }

    let engine = resolve_realtime_engine(&state)?;
    {
        let mut current_engine = state.realtime.engine.lock().await;
        *current_engine = engine.clone();
    }

    start_realtime_preview(&app, &state).await?;

    if let Some(id) = &payload.resume_artifact_id {
        let artifact = state
            .artifact_service
            .get(id)
            .await
            .map_err(CommandError::from)?
            .ok_or_else(|| CommandError::new("not_found", "realtime session not found"))?;

        engine.seed_buffer(&artifact.raw_transcript).await;
        session_title = requested_title.unwrap_or_else(|| artifact.title.clone());
        *state.realtime.session_name.lock().await = Some(session_title.clone());
    } else {
        engine.reset().await;
        *state.realtime.session_name.lock().await = Some(session_title.clone());
    }

    *state.realtime.active_job_id.lock().await = Some(job_id.clone());
    *state.realtime.model_filename.lock().await = Some(model.ggml_filename().to_string());
    *state.realtime.language_code.lock().await = language.as_whisper_code().to_string();
    *state.realtime.model.lock().await = Some(model.clone());
    *state.realtime.language.lock().await = Some(language.clone());

    let app_handle = app.clone();
    let emit_delta = Arc::new(move |delta: RealtimeDelta| {
        let _ = app_handle.emit("realtime://delta", delta);
    });

    if let Err(error) = engine
        .start(
            model.ggml_filename(),
            language.as_whisper_code(),
            emit_delta,
        )
        .await
    {
        *state.realtime.active_job_id.lock().await = None;
        stop_realtime_preview(&app, &state, "idle", "Microphone preview stopped.").await;
        return Err(CommandError::from(error));
    }

    sleep(Duration::from_millis(350)).await;
    if !engine.is_running().await {
        *state.realtime.active_job_id.lock().await = None;
        stop_realtime_preview(&app, &state, "idle", "Microphone preview stopped.").await;
        let diagnostics = engine.snapshot_diagnostics().await;
        let detail = if diagnostics.is_empty() {
            "Realtime transcription stopped immediately. Verify microphone access and that at least one audio input device is available.".to_string()
        } else {
            diagnostics.join(" ")
        };
        return Err(CommandError::from(ApplicationError::SpeechToText(detail)));
    }

    let _ = app.emit(
        "realtime://status",
        RealtimeStatusEvent {
            state: "running".to_string(),
            message: "Live listening".to_string(),
        },
    );

    emit_realtime_progress(
        &app,
        realtime_job_progress(
            &job_id,
            JobStage::Transcribing,
            "Live listening",
            0,
            Some(0.0),
            None,
        ),
        Some(session_title),
        model,
        language,
    );

    Ok(StartRealtimeResponse {
        started: true,
        job_id,
    })
}

#[tauri::command]
pub async fn pause_realtime(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let engine = state.realtime.engine.lock().await.clone();
    engine.pause().await.map_err(CommandError::from)?;
    stop_realtime_preview(&app, &state, "paused", "Microphone preview paused.").await;

    let _ = app.emit(
        "realtime://status",
        RealtimeStatusEvent {
            state: "paused".to_string(),
            message: "Live paused".to_string(),
        },
    );

    Ok(())
}

#[tauri::command]
pub async fn resume_realtime(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    let engine = state.realtime.engine.lock().await.clone();
    start_realtime_preview(&app, &state).await?;
    engine.resume().await.map_err(CommandError::from)?;

    let _ = app.emit(
        "realtime://status",
        RealtimeStatusEvent {
            state: "running".to_string(),
            message: "Live resumed".to_string(),
        },
    );

    Ok(())
}

#[tauri::command]
pub async fn stop_realtime(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: Option<StopRealtimePayload>,
) -> Result<StopRealtimeResponse, CommandError> {
    let payload = payload.unwrap_or(StopRealtimePayload {
        save: Some(true),
        title: None,
        elapsed_seconds: None,
    });
    let save = payload.save.unwrap_or(true);

    let settings = state
        .runtime_factory
        .load_settings()
        .map_err(|e| CommandError::new("settings", e))?;
    let engine = state.realtime.engine.lock().await.clone();
    let job_id = state
        .realtime
        .active_job_id
        .lock()
        .await
        .take()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let existing_session_title = state.realtime.session_name.lock().await.clone();
    let session_title = clean_optional_title(payload.title.clone())
        .or(existing_session_title)
        .unwrap_or_else(fallback_live_title);
    let language = state
        .realtime
        .language
        .lock()
        .await
        .clone()
        .unwrap_or(settings.transcription.language);
    let model = state
        .realtime
        .model
        .lock()
        .await
        .clone()
        .unwrap_or(settings.transcription.model);
    let language_code = state.realtime.language_code.lock().await.clone();
    let model_filename = state
        .realtime
        .model_filename
        .lock()
        .await
        .clone()
        .unwrap_or_else(|| model.ggml_filename().to_string());
    *state.realtime.session_name.lock().await = None;
    *state.realtime.model.lock().await = None;
    *state.realtime.language.lock().await = None;

    stop_realtime_preview(&app, &state, "idle", "Microphone preview stopped.").await;

    let _ = app.emit(
        "realtime://status",
        RealtimeStatusEvent {
            state: "stopped".to_string(),
            message: "Live stopped".to_string(),
        },
    );

    emit_realtime_progress(
        &app,
        realtime_job_progress(
            &job_id,
            if save {
                JobStage::Persisting
            } else {
                JobStage::Cancelled
            },
            if save {
                "Saving live transcription"
            } else {
                "Live transcription discarded"
            },
            if save { 90 } else { 100 },
            payload.elapsed_seconds.map(|value| value as f32),
            None,
        ),
        Some(session_title.clone()),
        model.clone(),
        language.clone(),
    );

    let app_handle = app.clone();
    let artifact_service = state.artifact_service.clone();
    let elapsed_seconds = payload.elapsed_seconds;
    let job_id_for_task = job_id.clone();
    tauri::async_runtime::spawn(async move {
        let stop_result = match engine.stop().await {
            Ok(result) => result,
            Err(error) => {
                let _ = app_handle.emit(
                    "transcription://failed",
                    JobFailedEvent {
                        job_id: job_id_for_task.clone(),
                        message: error.to_string(),
                    },
                );
                return;
            }
        };

        if !save {
            emit_realtime_progress(
                &app_handle,
                realtime_job_progress(
                    &job_id_for_task,
                    JobStage::Cancelled,
                    "Live transcription discarded",
                    100,
                    elapsed_seconds.map(|value| value as f32),
                    None,
                ),
                Some(session_title),
                model,
                language,
            );
            return;
        }

        if stop_result.transcript.trim().is_empty() {
            emit_realtime_progress(
                &app_handle,
                realtime_job_progress(
                    &job_id_for_task,
                    JobStage::Cancelled,
                    "Live transcription stopped with no captured speech",
                    100,
                    elapsed_seconds.map(|value| value as f32),
                    None,
                ),
                Some(session_title),
                model,
                language,
            );
            return;
        }

        let artifact = match build_realtime_artifact(RealtimeArtifactInput {
            job_id: job_id_for_task.clone(),
            session_title: session_title.clone(),
            language_code,
            model_filename,
            elapsed_seconds,
            transcript: stop_result.transcript,
            saved_audio_path: stop_result.saved_audio_path,
        }) {
            Ok(artifact) => artifact,
            Err(error) => {
                let _ = app_handle.emit(
                    "transcription://failed",
                    JobFailedEvent {
                        job_id: job_id_for_task.clone(),
                        message: error.to_string(),
                    },
                );
                return;
            }
        };

        match artifact_service.save(&artifact).await {
            Ok(()) => {
                emit_realtime_progress(
                    &app_handle,
                    realtime_job_progress(
                        &artifact.job_id,
                        JobStage::Completed,
                        "Live transcription saved",
                        100,
                        None,
                        None,
                    ),
                    Some(artifact.title.clone()),
                    model,
                    language,
                );
                let _ = app_handle.emit("transcription://completed", artifact.clone());
                let _ = app_handle.emit("realtime://saved", artifact);
            }
            Err(error) => {
                let _ = app_handle.emit(
                    "transcription://failed",
                    JobFailedEvent {
                        job_id: job_id_for_task,
                        message: error.to_string(),
                    },
                );
            }
        }
    });

    Ok(StopRealtimeResponse {
        saved: false,
        queued: true,
        job_id: Some(job_id),
        artifact: None,
    })
}

#[tauri::command]
pub async fn list_realtime_sessions(
    state: State<'_, AppState>,
) -> Result<Vec<TranscriptArtifact>, CommandError> {
    state
        .artifact_service
        .list(sbobino_application::ArtifactQuery {
            kind: Some(ArtifactKind::Realtime),
            query: None,
            limit: Some(100),
            offset: Some(0),
        })
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn load_realtime_session(
    state: State<'_, AppState>,
    payload: crate::commands::artifacts::GetArtifactPayload,
) -> Result<Option<TranscriptArtifact>, CommandError> {
    let artifact = state
        .artifact_service
        .get(&payload.id)
        .await
        .map_err(CommandError::from)?;

    if let Some(item) = &artifact {
        let engine = state.realtime.engine.lock().await.clone();
        engine.seed_buffer(&item.raw_transcript).await;
        *state.realtime.session_name.lock().await = Some(item.title.clone());
    }

    Ok(artifact)
}
