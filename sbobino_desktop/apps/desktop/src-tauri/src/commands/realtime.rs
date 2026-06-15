use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use tokio::time::{sleep, Duration};
use uuid::Uuid;

use sbobino_application::{ApplicationError, RealtimeDelta};
use sbobino_domain::{
    ArtifactKind, ArtifactSourceOrigin, LanguageCode, ParakeetModel, SpeechModel, TimedSegment,
    TranscriptArtifact, TranscriptionEngine, TranscriptionOutput,
};

use crate::parakeet_realtime::{ParakeetRealtimeEngine, ParakeetRealtimeStopResult};
use crate::realtime_audio::start_input_preview;
use crate::{error::CommandError, state::AppState};

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

fn resolve_parakeet_live_engine(state: &AppState) -> Result<ParakeetRealtimeEngine, CommandError> {
    let settings = state
        .runtime_factory
        .load_settings()
        .map_err(|load_error| CommandError::new("settings", load_error))?;
    let parakeet_cli_path = state
        .runtime_factory
        .resolve_binary_path(&settings.transcription.parakeet_cli_path, "parakeet-cli");
    let bin_dir = std::path::PathBuf::from(&parakeet_cli_path)
        .parent()
        .map(std::path::PathBuf::from)
        .ok_or_else(|| {
            CommandError::from(ApplicationError::SpeechToText(format!(
                "Unable to resolve Parakeet runtime directory from {parakeet_cli_path}"
            )))
        })?;
    let lib_path = if bin_dir.file_name().and_then(|name| name.to_str()) == Some("bin") {
        bin_dir
            .parent()
            .unwrap_or(&bin_dir)
            .join("lib")
            .join("libparakeet.dylib")
    } else {
        bin_dir.join("libparakeet.dylib")
    };
    let models_dir = state
        .runtime_factory
        .resolve_models_dir(&settings.transcription.parakeet_models_dir);
    Ok(ParakeetRealtimeEngine::new(lib_path, models_dir.into()))
}

fn select_parakeet_live_model(
    models_dir: &std::path::Path,
    requested: ParakeetModel,
) -> Result<ParakeetModel, CommandError> {
    let realtime_candidates = [
        ParakeetModel::RealtimeEou120mV1F16,
        ParakeetModel::RealtimeEou120mV1Q8,
    ];
    if matches!(
        requested,
        ParakeetModel::RealtimeEou120mV1F16 | ParakeetModel::RealtimeEou120mV1Q8
    ) {
        if models_dir.join(requested.gguf_filename()).exists() {
            return Ok(requested);
        }
    }

    for candidate in realtime_candidates {
        if models_dir.join(candidate.gguf_filename()).exists() {
            return Ok(candidate);
        }
    }

    Err(CommandError::from(ApplicationError::SpeechToText(format!(
        "Parakeet live requires {} or {} in {}. Install the realtime EOU model from Settings > Local Models.",
        ParakeetModel::RealtimeEou120mV1F16.gguf_filename(),
        ParakeetModel::RealtimeEou120mV1Q8.gguf_filename(),
        models_dir.display()
    ))))
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

struct RealtimeStopResult {
    transcript: String,
    segments: Vec<TimedSegment>,
    saved_audio_path: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RealtimeStatusEvent {
    pub state: String,
    pub message: String,
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

    if let Some(id) = &payload.resume_artifact_id {
        let artifact = state
            .artifact_service
            .get(id)
            .await
            .map_err(CommandError::from)?
            .ok_or_else(|| CommandError::new("not_found", "realtime session not found"))?;

        *state.realtime.session_name.lock().await = Some(artifact.title.clone());
    } else {
        *state.realtime.session_name.lock().await = None;
    }

    *state.realtime.language_code.lock().await = language.as_whisper_code().to_string();
    *state.realtime.active_engine.lock().await = engine_kind.clone();

    let app_handle = app.clone();
    let emit_delta = Arc::new(move |delta: RealtimeDelta| {
        let _ = app_handle.emit("realtime://delta", delta);
    });

    match engine_kind {
        TranscriptionEngine::WhisperCpp => {
            let engine = resolve_realtime_engine(&state)?;
            {
                let mut current_engine = state.realtime.engine.lock().await;
                *current_engine = engine.clone();
            }
            if let Some(id) = &payload.resume_artifact_id {
                if let Some(artifact) = state
                    .artifact_service
                    .get(id)
                    .await
                    .map_err(CommandError::from)?
                {
                    engine.seed_buffer(&artifact.raw_transcript).await;
                }
            } else {
                engine.reset().await;
            }
            *state.realtime.model_filename.lock().await = Some(model.ggml_filename().to_string());

            start_realtime_preview(&app, &state).await?;

            if let Err(error) = engine
                .start(
                    model.ggml_filename(),
                    language.as_whisper_code(),
                    emit_delta,
                )
                .await
            {
                stop_realtime_preview(&app, &state, "idle", "Microphone preview stopped.").await;
                return Err(CommandError::from(error));
            }
        }
        TranscriptionEngine::ParakeetCpp => {
            stop_realtime_preview(&app, &state, "idle", "Microphone preview stopped.").await;
            let parakeet_engine = resolve_parakeet_live_engine(&state)?;
            let models_dir = state
                .runtime_factory
                .resolve_models_dir(&settings.transcription.parakeet_models_dir);
            let requested_model = payload
                .parakeet_model
                .unwrap_or(settings.transcription.parakeet_model);
            let live_model =
                select_parakeet_live_model(std::path::Path::new(&models_dir), requested_model)?;
            if let Some(id) = &payload.resume_artifact_id {
                if let Some(artifact) = state
                    .artifact_service
                    .get(id)
                    .await
                    .map_err(CommandError::from)?
                {
                    parakeet_engine.seed_buffer(&artifact.raw_transcript).await;
                }
            } else {
                parakeet_engine.reset().await;
            }
            *state.realtime.model_filename.lock().await =
                Some(live_model.gguf_filename().to_string());
            {
                let mut current_engine = state.realtime.parakeet_engine.lock().await;
                *current_engine = Some(parakeet_engine.clone());
            }

            if let Err(error) = parakeet_engine
                .start(live_model.gguf_filename(), emit_delta)
                .await
            {
                return Err(CommandError::from(error));
            }
        }
    }

    sleep(Duration::from_millis(350)).await;
    let running = match engine_kind {
        TranscriptionEngine::WhisperCpp => {
            let engine = state.realtime.engine.lock().await.clone();
            engine.is_running().await
        }
        TranscriptionEngine::ParakeetCpp => {
            let engine = state.realtime.parakeet_engine.lock().await.clone();
            match engine {
                Some(engine) => engine.is_running().await,
                None => false,
            }
        }
    };
    if !running {
        if engine_kind == TranscriptionEngine::WhisperCpp {
            stop_realtime_preview(&app, &state, "idle", "Microphone preview stopped.").await;
        }
        let diagnostics = match engine_kind {
            TranscriptionEngine::WhisperCpp => {
                let engine = state.realtime.engine.lock().await.clone();
                engine.snapshot_diagnostics().await
            }
            TranscriptionEngine::ParakeetCpp => {
                let engine = state.realtime.parakeet_engine.lock().await.clone();
                match engine {
                    Some(engine) => engine.snapshot_diagnostics().await,
                    None => Vec::new(),
                }
            }
        };
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
    match state.realtime.active_engine.lock().await.clone() {
        TranscriptionEngine::WhisperCpp => {
            let engine = state.realtime.engine.lock().await.clone();
            engine.pause().await.map_err(CommandError::from)?;
            stop_realtime_preview(&app, &state, "paused", "Microphone preview paused.").await;
        }
        TranscriptionEngine::ParakeetCpp => {
            if let Some(engine) = state.realtime.parakeet_engine.lock().await.clone() {
                engine.pause().await.map_err(CommandError::from)?;
            }
        }
    }

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
    match state.realtime.active_engine.lock().await.clone() {
        TranscriptionEngine::WhisperCpp => {
            let engine = state.realtime.engine.lock().await.clone();
            start_realtime_preview(&app, &state).await?;
            engine.resume().await.map_err(CommandError::from)?;
        }
        TranscriptionEngine::ParakeetCpp => {
            if let Some(engine) = state.realtime.parakeet_engine.lock().await.clone() {
                engine.resume().await.map_err(CommandError::from)?;
            }
        }
    }

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

    let active_engine = state.realtime.active_engine.lock().await.clone();
    let stop_result = match active_engine {
        TranscriptionEngine::WhisperCpp => {
            let engine = state.realtime.engine.lock().await.clone();
            let result = engine.stop().await.map_err(CommandError::from)?;
            stop_realtime_preview(&app, &state, "idle", "Microphone preview stopped.").await;
            RealtimeStopResult {
                transcript: result.transcript,
                segments: Vec::new(),
                saved_audio_path: result.saved_audio_path,
            }
        }
        TranscriptionEngine::ParakeetCpp => {
            let engine = state
                .realtime
                .parakeet_engine
                .lock()
                .await
                .clone()
                .ok_or_else(|| CommandError::new("realtime", "Parakeet live is not running"))?;
            let result: ParakeetRealtimeStopResult =
                engine.stop().await.map_err(CommandError::from)?;
            RealtimeStopResult {
                transcript: result.transcript,
                segments: result.segments,
                saved_audio_path: result.saved_audio_path,
            }
        }
    };

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
        .or_else(|| {
            payload
                .title
                .clone()
                .filter(|title| !title.trim().is_empty())
        })
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
    if !stop_result.segments.is_empty() {
        metadata.insert(
            "timeline_v2".to_string(),
            TranscriptionOutput {
                text: stop_result.transcript.clone(),
                segments: stop_result.segments.clone(),
            }
            .timeline_v2_metadata_json(),
        );
    }

    let source_label = stop_result
        .saved_audio_path
        .as_ref()
        .and_then(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| format!("{session_title}.wav"));

    let mut artifact = TranscriptArtifact::new(
        Uuid::new_v4().to_string(),
        session_title.clone(),
        ArtifactKind::Realtime,
        source_label,
        ArtifactSourceOrigin::Realtime,
        stop_result.transcript,
        optimized,
        summary,
        faqs,
        metadata,
    )
    .map_err(|e| CommandError::new("validation", e.to_string()))?;
    artifact.audio_available = stop_result.saved_audio_path.is_some();
    artifact.audio_duration_seconds = payload.elapsed_seconds.map(|value| value as f32);
    artifact.processing_engine = Some(match active_engine {
        TranscriptionEngine::WhisperCpp => "whisper_stream".to_string(),
        TranscriptionEngine::ParakeetCpp => "parakeet_cpp".to_string(),
    });
    artifact.processing_model = Some(model_filename.clone());
    artifact.processing_language = Some(state.realtime.language_code.lock().await.clone());
    if let Some(path) = stop_result.saved_audio_path.as_ref() {
        artifact.set_source_external_path(path.to_string_lossy().to_string());
    }

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
        let engine = state.realtime.engine.lock().await.clone();
        engine.seed_buffer(&item.raw_transcript).await;
        *state.realtime.session_name.lock().await = Some(item.title.clone());
    }

    Ok(artifact)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parakeet_live_uses_installed_realtime_model_when_tdt_is_selected() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            temp.path()
                .join(ParakeetModel::RealtimeEou120mV1Q8.gguf_filename()),
            b"model",
        )
        .expect("write realtime model");

        let selected =
            select_parakeet_live_model(temp.path(), ParakeetModel::Tdt06bV3Q4).expect("selected");

        assert_eq!(selected, ParakeetModel::RealtimeEou120mV1Q8);
    }

    #[test]
    fn parakeet_live_fails_closed_without_realtime_model() {
        let temp = tempfile::tempdir().expect("temp dir");
        std::fs::write(
            temp.path().join(ParakeetModel::Tdt06bV3Q4.gguf_filename()),
            b"model",
        )
        .expect("write tdt model");

        let error = select_parakeet_live_model(temp.path(), ParakeetModel::Tdt06bV3Q4)
            .expect_err("tdt-only live should fail");

        assert!(
            error.message.contains("Parakeet live requires"),
            "{:?}",
            error
        );
    }
}
