use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use tokio::time::{sleep, Duration};
use uuid::Uuid;

use sbobino_application::{ApplicationError, RealtimeDelta};
use sbobino_domain::{ArtifactKind, LanguageCode, SpeechModel, TranscriptArtifact};

use crate::{error::CommandError, state::AppState};

#[derive(Debug, Deserialize)]
pub struct StartRealtimePayload {
    pub model: Option<SpeechModel>,
    pub language: Option<LanguageCode>,
    pub resume_artifact_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StartRealtimeResponse {
    pub started: bool,
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
    pub artifact: Option<TranscriptArtifact>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RealtimeStatusEvent {
    pub state: String,
    pub message: String,
}

#[tauri::command]
pub async fn start_realtime(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: Option<StartRealtimePayload>,
) -> Result<StartRealtimeResponse, CommandError> {
    let payload = payload.unwrap_or(StartRealtimePayload {
        model: None,
        language: None,
        resume_artifact_id: None,
    });

    let settings = state
        .runtime_factory
        .load_settings()
        .map_err(|e| CommandError::new("settings", e))?;

    let default_model = settings.transcription.model;
    let default_language = settings.transcription.language;
    let model = payload.model.unwrap_or(default_model);
    let language = payload.language.unwrap_or(default_language);

    if let Some(id) = &payload.resume_artifact_id {
        let artifact = state
            .artifact_service
            .get(id)
            .await
            .map_err(CommandError::from)?
            .ok_or_else(|| CommandError::new("not_found", "realtime session not found"))?;

        state
            .realtime
            .engine
            .seed_buffer(&artifact.raw_transcript)
            .await;
        *state.realtime.session_name.lock().await = Some(artifact.title.clone());
    } else {
        state.realtime.engine.reset().await;
        *state.realtime.session_name.lock().await = None;
    }

    *state.realtime.model_filename.lock().await = Some(model.ggml_filename().to_string());
    *state.realtime.language_code.lock().await = language.as_whisper_code().to_string();

    let app_handle = app.clone();
    let emit_delta = Arc::new(move |delta: RealtimeDelta| {
        let _ = app_handle.emit("realtime://delta", delta);
    });

    state
        .realtime
        .engine
        .start(
            model.ggml_filename(),
            language.as_whisper_code(),
            emit_delta,
        )
        .await
        .map_err(CommandError::from)?;

    sleep(Duration::from_millis(350)).await;
    if !state.realtime.engine.is_running().await {
        let diagnostics = state.realtime.engine.snapshot_diagnostics().await;
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

    Ok(StartRealtimeResponse { started: true })
}

#[tauri::command]
pub async fn pause_realtime(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), CommandError> {
    state
        .realtime
        .engine
        .pause()
        .await
        .map_err(CommandError::from)?;

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
    state
        .realtime
        .engine
        .resume()
        .await
        .map_err(CommandError::from)?;

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

    let stop_result = state
        .realtime
        .engine
        .stop()
        .await
        .map_err(CommandError::from)?;

    let _ = app.emit(
        "realtime://status",
        RealtimeStatusEvent {
            state: "stopped".to_string(),
            message: "Live stopped".to_string(),
        },
    );

    if !save || stop_result.transcript.trim().is_empty() {
        return Ok(StopRealtimeResponse {
            saved: false,
            artifact: None,
        });
    }

    let settings = state
        .runtime_factory
        .load_settings()
        .map_err(|e| CommandError::new("settings", e))?;

    let session_title = state
        .realtime
        .session_name
        .lock()
        .await
        .clone()
        .or_else(|| payload.title.clone().filter(|title| !title.trim().is_empty()))
        .unwrap_or_else(|| format!("live_{}", Utc::now().format("%d%m%Y_%H%M%S")));

    let language_code = state.realtime.language_code.lock().await.clone();
    let model_filename = state
        .realtime
        .model_filename
        .lock()
        .await
        .clone()
        .unwrap_or_else(|| settings.transcription.model.ggml_filename().to_string());

    let optimized = String::new();
    let summary = String::new();
    let faqs = String::new();

    let mut metadata = BTreeMap::new();
    metadata.insert("kind".to_string(), "realtime".to_string());
    metadata.insert("language".to_string(), language_code.clone());
    metadata.insert("model".to_string(), model_filename.clone());
    if let Some(elapsed_seconds) = payload.elapsed_seconds {
        metadata.insert("duration_seconds".to_string(), elapsed_seconds.to_string());
    }
    metadata.insert(
        "audio_saved".to_string(),
        if stop_result.saved_audio_path.is_some() {
            "true".to_string()
        } else {
            "false".to_string()
        },
    );

    let input_path = stop_result
        .saved_audio_path
        .as_ref()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| format!("{session_title}.wav"));

    let artifact = TranscriptArtifact::new(
        Uuid::new_v4().to_string(),
        session_title.clone(),
        ArtifactKind::Realtime,
        input_path,
        stop_result.transcript,
        optimized,
        summary,
        faqs,
        metadata,
    )
    .map_err(|e| CommandError::new("validation", e.to_string()))?;

    state
        .artifact_service
        .save(&artifact)
        .await
        .map_err(CommandError::from)?;

    let _ = app.emit("realtime://saved", artifact.clone());

    Ok(StopRealtimeResponse {
        saved: true,
        artifact: Some(artifact),
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
        state
            .realtime
            .engine
            .seed_buffer(&item.raw_transcript)
            .await;
        *state.realtime.session_name.lock().await = Some(item.title.clone());
    }

    Ok(artifact)
}
