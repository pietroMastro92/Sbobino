use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::State;
use tracing::warn;

use sbobino_domain::{LanguageCode, ParakeetModel, SpeechModel, TranscriptionEngine};
use sbobino_infrastructure::{ManagedRuntimeHealth, PyannoteRuntimeHealth};

use crate::commands::realtime::{
    parakeet_live_target_lang, resolve_parakeet_live_library_path, select_parakeet_live_model,
};
use crate::parakeet_realtime::ParakeetRealtimeEngine;
use crate::realtime_audio::probe_input_device_name;
use crate::{error::CommandError, state::AppState};

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeHealthResponse {
    pub app_version: String,
    pub host_os: String,
    pub host_arch: String,
    pub is_apple_silicon: bool,
    pub preferred_engine: String,
    pub configured_engine: String,
    pub runtime_source: String,
    pub managed_runtime_required: bool,
    pub managed_runtime: ManagedRuntimeHealth,
    pub ffmpeg_path: String,
    pub ffmpeg_resolved: String,
    pub ffmpeg_available: bool,
    pub whisper_cli_path: String,
    pub whisper_cli_resolved: String,
    pub whisper_cli_available: bool,
    pub whisper_stream_path: String,
    pub whisper_stream_resolved: String,
    pub whisper_stream_available: bool,
    pub parakeet_cli_path: String,
    pub parakeet_cli_resolved: String,
    pub parakeet_cli_available: bool,
    pub models_dir_configured: String,
    pub models_dir_resolved: String,
    pub parakeet_models_dir_configured: String,
    pub parakeet_models_dir_resolved: String,
    pub model_filename: String,
    pub model_present: bool,
    pub parakeet_model_filename: String,
    pub parakeet_model_present: bool,
    pub missing_parakeet_models: Vec<String>,
    pub coreml_encoder_present: bool,
    pub missing_models: Vec<String>,
    pub missing_encoders: Vec<String>,
    pub pyannote: PyannoteRuntimeHealth,
    pub setup_complete: bool,
}

#[derive(Debug, Deserialize)]
pub struct StartPreflightPayload {
    #[serde(default)]
    pub engine: Option<TranscriptionEngine>,
    pub model: SpeechModel,
    #[serde(default)]
    pub parakeet_model: Option<ParakeetModel>,
    #[serde(default)]
    pub language: Option<LanguageCode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StartPreflightResponse {
    pub allowed: bool,
    pub reason_code: String,
    pub message: String,
    pub engine: String,
    pub model_filename: String,
    pub model_path: String,
    pub whisper_cli_resolved: String,
    pub whisper_stream_resolved: String,
    pub parakeet_cli_resolved: String,
    pub pyannote: PyannoteRuntimeHealth,
}

#[derive(Debug, Clone, Serialize)]
pub struct RealtimeStartReadinessResponse {
    pub allowed: bool,
    pub reason_code: String,
    pub message: String,
    pub engine: String,
    pub model_filename: String,
    pub model_path: String,
    pub ffmpeg_resolved: String,
    pub whisper_stream_resolved: String,
    pub parakeet_cli_resolved: String,
    pub input_device_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnsureRuntimeResponse {
    pub ready: bool,
    pub engine: String,
    pub did_setup: bool,
    pub message: String,
    pub ffmpeg_resolved: String,
    pub whisper_cli_resolved: String,
    pub whisper_stream_resolved: String,
    pub parakeet_cli_resolved: String,
}

fn engine_to_wire(engine: &TranscriptionEngine) -> &'static str {
    match engine {
        TranscriptionEngine::WhisperCpp => "whisper_cpp",
        TranscriptionEngine::ParakeetCpp => "parakeet_cpp",
    }
}

fn runtime_toolchain_ready(health: &sbobino_infrastructure::RuntimeHealth) -> bool {
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

fn first_managed_runtime_failure(
    managed_runtime: &ManagedRuntimeHealth,
    engine: TranscriptionEngine,
) -> Option<(&'static str, &str, &str)> {
    if !managed_runtime.ffmpeg.available {
        return Some((
            "FFmpeg",
            managed_runtime.ffmpeg.resolved_path.as_str(),
            managed_runtime.ffmpeg.failure_message.as_str(),
        ));
    }
    match engine {
        TranscriptionEngine::WhisperCpp => {
            if !managed_runtime.whisper_cli.available {
                return Some((
                    "Whisper CLI",
                    managed_runtime.whisper_cli.resolved_path.as_str(),
                    managed_runtime.whisper_cli.failure_message.as_str(),
                ));
            }
            if !managed_runtime.whisper_stream.available {
                return Some((
                    "Whisper Stream",
                    managed_runtime.whisper_stream.resolved_path.as_str(),
                    managed_runtime.whisper_stream.failure_message.as_str(),
                ));
            }
        }
        TranscriptionEngine::ParakeetCpp => {
            if !managed_runtime.parakeet_cli.available {
                return Some((
                    "Parakeet CLI",
                    managed_runtime.parakeet_cli.resolved_path.as_str(),
                    managed_runtime.parakeet_cli.failure_message.as_str(),
                ));
            }
        }
    }
    None
}

fn runtime_health_response(health: sbobino_infrastructure::RuntimeHealth) -> RuntimeHealthResponse {
    RuntimeHealthResponse {
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        host_os: health.host_os,
        host_arch: health.host_arch,
        is_apple_silicon: health.is_apple_silicon,
        preferred_engine: engine_to_wire(&health.preferred_engine).to_string(),
        configured_engine: engine_to_wire(&health.configured_engine).to_string(),
        runtime_source: health.runtime_source,
        managed_runtime_required: health.managed_runtime_required,
        managed_runtime: health.managed_runtime,
        ffmpeg_path: health.ffmpeg_path,
        ffmpeg_resolved: health.ffmpeg_resolved,
        ffmpeg_available: health.ffmpeg_available,
        whisper_cli_path: health.whisper_cli_path,
        whisper_cli_resolved: health.whisper_cli_resolved,
        whisper_cli_available: health.whisper_cli_available,
        whisper_stream_path: health.whisper_stream_path,
        whisper_stream_resolved: health.whisper_stream_resolved,
        whisper_stream_available: health.whisper_stream_available,
        parakeet_cli_path: health.parakeet_cli_path,
        parakeet_cli_resolved: health.parakeet_cli_resolved,
        parakeet_cli_available: health.parakeet_cli_available,
        models_dir_configured: health.models_dir_configured,
        models_dir_resolved: health.models_dir_resolved,
        parakeet_models_dir_configured: health.parakeet_models_dir_configured,
        parakeet_models_dir_resolved: health.parakeet_models_dir_resolved,
        model_filename: health.model_filename,
        model_present: health.model_present,
        parakeet_model_filename: health.parakeet_model_filename,
        parakeet_model_present: health.parakeet_model_present,
        missing_parakeet_models: health.missing_parakeet_models,
        coreml_encoder_present: health.coreml_encoder_present,
        missing_models: health.missing_models,
        missing_encoders: health.missing_encoders,
        pyannote: health.pyannote,
        setup_complete: health.setup_complete,
    }
}

fn is_legacy_whisperkit_path(path: &str) -> bool {
    path.to_ascii_lowercase().contains("whisperkit-cli")
}

fn runtime_toolchain_message(
    health: &sbobino_infrastructure::RuntimeHealth,
    setup_note: Option<&str>,
) -> String {
    if health.managed_runtime_required {
        if let Some((label, path, detail)) =
            first_managed_runtime_failure(&health.managed_runtime, health.configured_engine.clone())
        {
            let mut message = if detail.trim().is_empty() {
                format!("{label} is not runnable at '{path}'.")
            } else {
                format!("{label} verification failed at '{path}': {}", detail.trim())
            };
            if let Some(note) = setup_note {
                message.push(' ');
                message.push_str(note);
            }
            message.push_str(" Repair the local runtime from Settings > Local Models.");
            return message;
        }
    }

    let mut missing = Vec::new();
    if !health.ffmpeg_available {
        missing.push(format!(
            "FFmpeg is not runnable at '{}'.",
            health.ffmpeg_resolved
        ));
    }
    match health.configured_engine {
        TranscriptionEngine::WhisperCpp => {
            if !health.whisper_cli_available {
                missing.push(format!(
                    "Whisper CLI is not runnable at '{}'.",
                    health.whisper_cli_resolved
                ));
            }
            if !health.whisper_stream_available {
                missing.push(format!(
                    "Whisper Stream is not runnable at '{}'.",
                    health.whisper_stream_resolved
                ));
            }
        }
        TranscriptionEngine::ParakeetCpp => {
            if !health.parakeet_cli_available {
                missing.push(format!(
                    "Parakeet CLI is not runnable at '{}'.",
                    health.parakeet_cli_resolved
                ));
            }
        }
    }

    let mut message = if missing.is_empty() {
        "Transcription runtime unavailable.".to_string()
    } else {
        missing.join(" ")
    };
    if let Some(note) = setup_note {
        message.push(' ');
        message.push_str(note);
    }
    message.push_str(" Repair the local runtime from Settings > Local Models.");
    message
}

async fn normalize_runtime_settings_for_whisper_cpp(state: &AppState) -> (bool, Option<String>) {
    let mut did_setup = false;
    let mut setup_note = None::<String>;

    match state.settings_service.snapshot().await {
        Ok(mut settings) => {
            let mut changed = false;

            if settings.transcription.engine != TranscriptionEngine::WhisperCpp {
                settings.transcription.engine = TranscriptionEngine::WhisperCpp;
                settings.transcription_engine = TranscriptionEngine::WhisperCpp;
                changed = true;
            }

            let transcription_path = settings.transcription.whisper_cli_path.trim();
            if transcription_path.is_empty() || is_legacy_whisperkit_path(transcription_path) {
                settings.transcription.whisper_cli_path = "whisper-cli".to_string();
                changed = true;
            }

            let legacy_path = settings.whisper_cli_path.trim();
            if legacy_path.is_empty() || is_legacy_whisperkit_path(legacy_path) {
                settings.whisper_cli_path = "whisper-cli".to_string();
                changed = true;
            }

            let transcription_stream_path = settings.transcription.whisperkit_cli_path.trim();
            if transcription_stream_path.is_empty()
                || is_legacy_whisperkit_path(transcription_stream_path)
            {
                settings.transcription.whisperkit_cli_path = "whisper-stream".to_string();
                changed = true;
            }

            let legacy_stream_path = settings.whisperkit_cli_path.trim();
            if legacy_stream_path.is_empty() || is_legacy_whisperkit_path(legacy_stream_path) {
                settings.whisperkit_cli_path = "whisper-stream".to_string();
                changed = true;
            }

            if changed {
                settings.sync_sections_from_legacy();
                settings.sync_legacy_from_sections();
                match state.settings_service.update(settings).await {
                    Ok(_) => {
                        did_setup = true;
                    }
                    Err(error) => {
                        let message =
                            format!("Failed to persist whisper.cpp runtime settings: {error}");
                        warn!("{message}");
                        setup_note = Some(message);
                    }
                }
            }
        }
        Err(error) => {
            let message = format!("Failed to load settings for whisper.cpp runtime setup: {error}");
            warn!("{message}");
            setup_note = Some(message);
        }
    }

    (did_setup, setup_note)
}

#[tauri::command]
pub async fn ensure_transcription_runtime(
    state: State<'_, AppState>,
) -> Result<EnsureRuntimeResponse, CommandError> {
    let health = state
        .runtime_factory
        .runtime_health_preflight()
        .map_err(|e| CommandError::new("runtime_health", e))?;

    if runtime_toolchain_ready(&health) {
        return Ok(EnsureRuntimeResponse {
            ready: true,
            engine: engine_to_wire(&health.configured_engine).to_string(),
            did_setup: false,
            message: match health.configured_engine {
                TranscriptionEngine::WhisperCpp => "Whisper.cpp runtime available.".to_string(),
                TranscriptionEngine::ParakeetCpp => "Parakeet.cpp runtime available.".to_string(),
            },
            ffmpeg_resolved: health.ffmpeg_resolved,
            whisper_cli_resolved: health.whisper_cli_resolved,
            whisper_stream_resolved: health.whisper_stream_resolved,
            parakeet_cli_resolved: health.parakeet_cli_resolved,
        });
    }

    let (did_setup, setup_note) = if health.configured_engine == TranscriptionEngine::WhisperCpp {
        normalize_runtime_settings_for_whisper_cpp(&state).await
    } else {
        (false, None)
    };

    let refreshed = state
        .runtime_factory
        .runtime_health()
        .map_err(|e| CommandError::new("runtime_health", e))?;

    let ready = runtime_toolchain_ready(&refreshed);
    let message = if ready {
        if did_setup {
            "Whisper.cpp runtime is ready.".to_string()
        } else if refreshed.configured_engine == TranscriptionEngine::ParakeetCpp {
            "Parakeet.cpp runtime available.".to_string()
        } else {
            "Whisper.cpp runtime available.".to_string()
        }
    } else {
        runtime_toolchain_message(&refreshed, setup_note.as_deref())
    };

    Ok(EnsureRuntimeResponse {
        ready,
        engine: engine_to_wire(&refreshed.configured_engine).to_string(),
        did_setup,
        message,
        ffmpeg_resolved: refreshed.ffmpeg_resolved,
        whisper_cli_resolved: refreshed.whisper_cli_resolved,
        whisper_stream_resolved: refreshed.whisper_stream_resolved,
        parakeet_cli_resolved: refreshed.parakeet_cli_resolved,
    })
}

#[tauri::command]
pub async fn get_realtime_start_readiness(
    state: State<'_, AppState>,
    payload: Option<StartPreflightPayload>,
) -> Result<RealtimeStartReadinessResponse, CommandError> {
    eprintln!("[realtime-readiness] command received payload={payload:?}");

    let mut settings = state
        .runtime_factory
        .load_settings()
        .map_err(|e| CommandError::new("settings", e))?;
    let selected_engine = payload
        .as_ref()
        .and_then(|value| value.engine.clone())
        .unwrap_or_else(|| settings.transcription.engine.clone());
    eprintln!("[realtime-readiness] selected_engine={selected_engine:?}");

    if selected_engine == TranscriptionEngine::WhisperCpp {
        let _ = normalize_runtime_settings_for_whisper_cpp(&state).await;
        settings = state
            .runtime_factory
            .load_settings()
            .map_err(|e| CommandError::new("settings", e))?;
    }

    if selected_engine == TranscriptionEngine::ParakeetCpp {
        let selected_language = payload
            .as_ref()
            .and_then(|value| value.language.clone())
            .unwrap_or_else(|| settings.transcription.language.clone());
        let requested_parakeet_model = payload
            .as_ref()
            .and_then(|value| value.parakeet_model.clone())
            .unwrap_or_else(|| settings.transcription.parakeet_model.clone());
        let health = state
            .runtime_factory
            .runtime_health_preflight()
            .map_err(|e| CommandError::new("runtime_health", e))?;
        let models_dir = PathBuf::from(&health.parakeet_models_dir_resolved);
        let selected_parakeet_model = select_parakeet_live_model(
            &models_dir,
            requested_parakeet_model,
            selected_language.clone(),
        )
        .map_err(|e| CommandError::new("runtime_health", e.message))?;
        let model_filename = selected_parakeet_model.gguf_filename().to_string();
        let model_path_buf = models_dir.join(&model_filename);
        let model_path = model_path_buf.to_string_lossy().to_string();
        let parakeet_lib =
            resolve_parakeet_live_library_path(PathBuf::from(&health.parakeet_cli_resolved));

        eprintln!(
            "[realtime-readiness] parakeet health cli_available={} cli={} models_dir={}",
            health.parakeet_cli_available,
            health.parakeet_cli_resolved,
            health.parakeet_models_dir_resolved
        );
        eprintln!(
            "[realtime-readiness] parakeet resolved lib={} model={}",
            parakeet_lib.display(),
            model_path
        );

        if !health.parakeet_cli_available {
            eprintln!("[realtime-readiness] blocked: parakeet_cli_missing");
            return Ok(RealtimeStartReadinessResponse {
                allowed: false,
                reason_code: "parakeet_cli_missing".to_string(),
                message: format!(
                    "Parakeet.cpp runtime is not available at {}.",
                    health.parakeet_cli_resolved
                ),
                engine: "parakeet_cpp".to_string(),
                model_filename,
                model_path,
                ffmpeg_resolved: health.ffmpeg_resolved,
                whisper_stream_resolved: health.whisper_stream_resolved,
                parakeet_cli_resolved: health.parakeet_cli_resolved,
                input_device_name: None,
            });
        }

        if !parakeet_lib.exists() {
            eprintln!(
                "[realtime-readiness] blocked: parakeet_live_library_missing path={}",
                parakeet_lib.display()
            );
            return Ok(RealtimeStartReadinessResponse {
                allowed: false,
                reason_code: "parakeet_live_library_missing".to_string(),
                message: format!(
                    "Parakeet.cpp live library is missing at {}. Reinstall the local runtime.",
                    parakeet_lib.display()
                ),
                engine: "parakeet_cpp".to_string(),
                model_filename,
                model_path,
                ffmpeg_resolved: health.ffmpeg_resolved,
                whisper_stream_resolved: health.whisper_stream_resolved,
                parakeet_cli_resolved: health.parakeet_cli_resolved,
                input_device_name: None,
            });
        }

        if let Err(error) =
            ParakeetRealtimeEngine::new(parakeet_lib.clone(), models_dir.clone()).validate_library()
        {
            eprintln!(
                "[realtime-readiness] blocked: parakeet_live_library_unloadable path={} error={}",
                parakeet_lib.display(),
                error
            );
            return Ok(RealtimeStartReadinessResponse {
                allowed: false,
                reason_code: "parakeet_live_library_unloadable".to_string(),
                message: format!(
                    "Parakeet.cpp live library is not loadable at {}. {}",
                    parakeet_lib.display(),
                    error
                ),
                engine: "parakeet_cpp".to_string(),
                model_filename,
                model_path,
                ffmpeg_resolved: health.ffmpeg_resolved,
                whisper_stream_resolved: health.whisper_stream_resolved,
                parakeet_cli_resolved: health.parakeet_cli_resolved,
                input_device_name: None,
            });
        }

        if !model_path_buf.exists() {
            eprintln!(
                "[realtime-readiness] blocked: parakeet_realtime_model_missing path={}",
                model_path_buf.display()
            );
            return Ok(RealtimeStartReadinessResponse {
                allowed: false,
                reason_code: "parakeet_realtime_model_missing".to_string(),
                message: format!(
                    "Parakeet.cpp live requires the selected streaming model. Download '{}' in Local Models.",
                    model_filename
                ),
                engine: "parakeet_cpp".to_string(),
                model_filename,
                model_path,
                ffmpeg_resolved: health.ffmpeg_resolved,
                whisper_stream_resolved: health.whisper_stream_resolved,
                parakeet_cli_resolved: health.parakeet_cli_resolved,
                input_device_name: None,
            });
        }

        let input_device_name = match crate::realtime_audio::probe_input_device_name() {
            Ok(name) => Some(name),
            Err(error) => {
                eprintln!(
                    "[realtime-readiness] blocked: input_device reason={} message={}",
                    error.reason_code, error.message
                );
                return Ok(RealtimeStartReadinessResponse {
                    allowed: false,
                    reason_code: error.reason_code,
                    message: error.message,
                    engine: "parakeet_cpp".to_string(),
                    model_filename,
                    model_path,
                    ffmpeg_resolved: health.ffmpeg_resolved,
                    whisper_stream_resolved: health.whisper_stream_resolved,
                    parakeet_cli_resolved: health.parakeet_cli_resolved,
                    input_device_name: None,
                });
            }
        };

        eprintln!(
            "[realtime-readiness] allowed: parakeet model={} input_device={input_device_name:?}",
            model_filename
        );
        return Ok(RealtimeStartReadinessResponse {
            allowed: true,
            reason_code: "ok".to_string(),
            message: format!(
                "Parakeet.cpp live is ready with '{}' (lang {}).",
                model_filename,
                parakeet_live_target_lang(selected_language)
            ),
            engine: "parakeet_cpp".to_string(),
            model_filename,
            model_path,
            ffmpeg_resolved: health.ffmpeg_resolved,
            whisper_stream_resolved: health.whisper_stream_resolved,
            parakeet_cli_resolved: health.parakeet_cli_resolved,
            input_device_name,
        });
    }
    let selected_model = payload
        .as_ref()
        .map(|value| value.model.clone())
        .unwrap_or_else(|| settings.transcription.model.clone());
    let live_health = state
        .runtime_factory
        .live_start_health(selected_model.clone())
        .map_err(|e| CommandError::new("runtime_health", e))?;

    let model_filename = selected_model.ggml_filename().to_string();
    let model_path = PathBuf::from(&live_health.models_dir_resolved)
        .join(&model_filename)
        .to_string_lossy()
        .to_string();

    if !live_health.ffmpeg_available {
        return Ok(RealtimeStartReadinessResponse {
            allowed: false,
            reason_code: "ffmpeg_missing".to_string(),
            message: format!(
                "FFmpeg is not runnable at '{}'. Repair the local runtime from Settings > Local Models.",
                live_health.ffmpeg_resolved
            ),
            engine: "whisper_cpp".to_string(),
            model_filename,
            model_path,
            ffmpeg_resolved: live_health.ffmpeg_resolved,
            whisper_stream_resolved: live_health.whisper_stream_resolved,
            parakeet_cli_resolved: live_health.parakeet_cli_resolved,
            input_device_name: None,
        });
    }

    if !live_health.whisper_stream_available {
        return Ok(RealtimeStartReadinessResponse {
            allowed: false,
            reason_code: "whisper_stream_missing".to_string(),
            message: format!(
                "Whisper Stream is not runnable at '{}'. Repair the local runtime from Settings > Local Models.",
                live_health.whisper_stream_resolved
            ),
            engine: "whisper_cpp".to_string(),
            model_filename,
            model_path,
            ffmpeg_resolved: live_health.ffmpeg_resolved,
            whisper_stream_resolved: live_health.whisper_stream_resolved,
            parakeet_cli_resolved: live_health.parakeet_cli_resolved,
            input_device_name: None,
        });
    }

    if !live_health.model_present {
        return Ok(RealtimeStartReadinessResponse {
            allowed: false,
            reason_code: "model_missing".to_string(),
            message: format!(
                "Model file '{}' was not found in '{}'. Download models from Settings > Local Models.",
                model_filename, live_health.models_dir_resolved
            ),
            engine: "whisper_cpp".to_string(),
            model_filename,
            model_path,
            ffmpeg_resolved: live_health.ffmpeg_resolved,
            whisper_stream_resolved: live_health.whisper_stream_resolved,
            parakeet_cli_resolved: live_health.parakeet_cli_resolved,
            input_device_name: None,
        });
    }

    match probe_input_device_name() {
        Ok(device_name) => Ok(RealtimeStartReadinessResponse {
            allowed: true,
            reason_code: "ok".to_string(),
            message: "Realtime start readiness passed.".to_string(),
            engine: "whisper_cpp".to_string(),
            model_filename,
            model_path,
            ffmpeg_resolved: live_health.ffmpeg_resolved,
            whisper_stream_resolved: live_health.whisper_stream_resolved,
            parakeet_cli_resolved: live_health.parakeet_cli_resolved,
            input_device_name: Some(device_name),
        }),
        Err(error) => Ok(RealtimeStartReadinessResponse {
            allowed: false,
            reason_code: error.reason_code,
            message: error.message,
            engine: "whisper_cpp".to_string(),
            model_filename,
            model_path,
            ffmpeg_resolved: live_health.ffmpeg_resolved,
            whisper_stream_resolved: live_health.whisper_stream_resolved,
            parakeet_cli_resolved: live_health.parakeet_cli_resolved,
            input_device_name: None,
        }),
    }
}

#[tauri::command]
pub async fn get_transcription_start_preflight(
    state: State<'_, AppState>,
    payload: Option<StartPreflightPayload>,
) -> Result<StartPreflightResponse, CommandError> {
    let health = state
        .runtime_factory
        .runtime_health()
        .map_err(|e| CommandError::new("runtime_health", e))?;

    let requested_engine = payload
        .as_ref()
        .and_then(|value| value.engine.clone())
        .unwrap_or_else(|| health.configured_engine.clone());
    let model_filename = match requested_engine {
        TranscriptionEngine::WhisperCpp => payload
            .as_ref()
            .map(|value| value.model.ggml_filename().to_string())
            .unwrap_or_else(|| health.model_filename.clone()),
        TranscriptionEngine::ParakeetCpp => payload
            .as_ref()
            .and_then(|value| value.parakeet_model.clone())
            .map(|model| model.gguf_filename().to_string())
            .unwrap_or_else(|| health.parakeet_model_filename.clone()),
    };
    let model_dir = match requested_engine {
        TranscriptionEngine::WhisperCpp => &health.models_dir_resolved,
        TranscriptionEngine::ParakeetCpp => &health.parakeet_models_dir_resolved,
    };
    let model_path = PathBuf::from(model_dir)
        .join(&model_filename)
        .to_string_lossy()
        .to_string();
    let engine_wire = engine_to_wire(&requested_engine).to_string();

    if !health.ffmpeg_available {
        let message = if health.managed_runtime_required {
            runtime_toolchain_message(&health, None)
        } else {
            format!(
                "FFmpeg is not runnable at '{}'. Configure FFmpeg path in Settings > Advanced.",
                health.ffmpeg_resolved
            )
        };
        return Ok(StartPreflightResponse {
            allowed: false,
            reason_code: "ffmpeg_missing".to_string(),
            message,
            engine: engine_wire,
            model_filename,
            model_path,
            whisper_cli_resolved: health.whisper_cli_resolved,
            whisper_stream_resolved: health.whisper_stream_resolved,
            parakeet_cli_resolved: health.parakeet_cli_resolved,
            pyannote: health.pyannote,
        });
    }

    if requested_engine == TranscriptionEngine::WhisperCpp && !health.whisper_cli_available {
        let message = if health.managed_runtime_required {
            runtime_toolchain_message(&health, None)
        } else {
            format!(
                "Whisper CLI is not runnable at '{}'. Configure Whisper CLI path in Settings > Local Models.",
                health.whisper_cli_resolved
            )
        };
        return Ok(StartPreflightResponse {
            allowed: false,
            reason_code: "whispercpp_missing".to_string(),
            message,
            engine: engine_wire,
            model_filename,
            model_path,
            whisper_cli_resolved: health.whisper_cli_resolved,
            whisper_stream_resolved: health.whisper_stream_resolved,
            parakeet_cli_resolved: health.parakeet_cli_resolved,
            pyannote: health.pyannote,
        });
    }

    if requested_engine == TranscriptionEngine::ParakeetCpp && !health.parakeet_cli_available {
        let message = format!(
            "Parakeet CLI is not runnable at '{}'. Repair the local runtime from Settings > Local Models.",
            health.parakeet_cli_resolved
        );
        return Ok(StartPreflightResponse {
            allowed: false,
            reason_code: "parakeetcpp_missing".to_string(),
            message,
            engine: engine_wire,
            model_filename,
            model_path,
            whisper_cli_resolved: health.whisper_cli_resolved,
            whisper_stream_resolved: health.whisper_stream_resolved,
            parakeet_cli_resolved: health.parakeet_cli_resolved,
            pyannote: health.pyannote,
        });
    }

    if !PathBuf::from(&model_path).exists() {
        return Ok(StartPreflightResponse {
            allowed: false,
            reason_code: "model_missing".to_string(),
            message: format!(
                "Model file '{}' was not found in '{}'. Download models from Settings > Local Models.",
                model_filename, model_dir
            ),
            engine: engine_wire,
            model_filename,
            model_path,
            whisper_cli_resolved: health.whisper_cli_resolved,
            whisper_stream_resolved: health.whisper_stream_resolved,
            parakeet_cli_resolved: health.parakeet_cli_resolved,
            pyannote: health.pyannote,
        });
    }

    Ok(StartPreflightResponse {
        allowed: true,
        reason_code: "ok".to_string(),
        message: match requested_engine {
            TranscriptionEngine::WhisperCpp => "Whisper.cpp preflight passed.".to_string(),
            TranscriptionEngine::ParakeetCpp => "Parakeet.cpp preflight passed.".to_string(),
        },
        engine: engine_wire,
        model_filename,
        model_path,
        whisper_cli_resolved: health.whisper_cli_resolved,
        whisper_stream_resolved: health.whisper_stream_resolved,
        parakeet_cli_resolved: health.parakeet_cli_resolved,
        pyannote: health.pyannote,
    })
}

#[tauri::command]
pub async fn get_transcription_runtime_health(
    state: State<'_, AppState>,
) -> Result<RuntimeHealthResponse, CommandError> {
    let health = state
        .runtime_factory
        .runtime_health()
        .map_err(|e| CommandError::new("runtime_health", e))?;

    Ok(runtime_health_response(health))
}

#[tauri::command]
pub async fn get_transcription_runtime_status(
    state: State<'_, AppState>,
) -> Result<RuntimeHealthResponse, CommandError> {
    let health = state
        .runtime_factory
        .runtime_health_preflight()
        .map_err(|e| CommandError::new("runtime_status", e))?;

    Ok(runtime_health_response(health))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sbobino_infrastructure::{ManagedRuntimeBinaryHealth, RuntimeHealth};

    fn available_binary(name: &str) -> ManagedRuntimeBinaryHealth {
        ManagedRuntimeBinaryHealth {
            resolved_path: format!("/tmp/{name}"),
            available: true,
            failure_reason: String::new(),
            failure_message: String::new(),
        }
    }

    fn missing_binary(name: &str) -> ManagedRuntimeBinaryHealth {
        ManagedRuntimeBinaryHealth {
            resolved_path: format!("/tmp/{name}"),
            available: false,
            failure_reason: "missing_file".to_string(),
            failure_message: "Managed runtime binary is missing.".to_string(),
        }
    }

    fn runtime_health_for_managed_runtime(
        configured_engine: TranscriptionEngine,
        parakeet_available: bool,
    ) -> RuntimeHealth {
        let parakeet_cli = if parakeet_available {
            available_binary("parakeet-cli")
        } else {
            missing_binary("parakeet-cli")
        };
        RuntimeHealth {
            host_os: "macos".to_string(),
            host_arch: "aarch64".to_string(),
            is_apple_silicon: true,
            preferred_engine: TranscriptionEngine::WhisperCpp,
            configured_engine,
            runtime_source: "managed_release_asset".to_string(),
            managed_runtime_required: true,
            managed_runtime: ManagedRuntimeHealth {
                source: "managed_release_asset".to_string(),
                ready: parakeet_available,
                ffmpeg: available_binary("ffmpeg"),
                whisper_cli: available_binary("whisper-cli"),
                whisper_stream: available_binary("whisper-stream"),
                parakeet_cli,
            },
            ffmpeg_path: "ffmpeg".to_string(),
            ffmpeg_resolved: "/tmp/ffmpeg".to_string(),
            ffmpeg_available: true,
            whisper_cli_path: "whisper-cli".to_string(),
            whisper_cli_resolved: "/tmp/whisper-cli".to_string(),
            whisper_cli_available: true,
            whisper_stream_path: "whisper-stream".to_string(),
            whisper_stream_resolved: "/tmp/whisper-stream".to_string(),
            whisper_stream_available: true,
            parakeet_cli_path: "parakeet-cli".to_string(),
            parakeet_cli_resolved: "/tmp/parakeet-cli".to_string(),
            parakeet_cli_available: parakeet_available,
            models_dir_configured: "/tmp/models".to_string(),
            models_dir_resolved: "/tmp/models".to_string(),
            parakeet_models_dir_configured: "/tmp/parakeet-models".to_string(),
            parakeet_models_dir_resolved: "/tmp/parakeet-models".to_string(),
            model_filename: "ggml-base.bin".to_string(),
            model_present: true,
            parakeet_model_filename: "tdt-0.6b-v3-q4_k.gguf".to_string(),
            parakeet_model_present: true,
            missing_parakeet_models: Vec::new(),
            coreml_encoder_present: true,
            missing_models: Vec::new(),
            missing_encoders: Vec::new(),
            pyannote: PyannoteRuntimeHealth {
                enabled: false,
                ready: true,
                runtime_installed: false,
                model_installed: false,
                runtime_dir: "/tmp/runtime/pyannote".to_string(),
                arch: "aarch64-apple-darwin".to_string(),
                device: "cpu".to_string(),
                source: "disabled".to_string(),
                reason_code: "disabled".to_string(),
                message: "disabled".to_string(),
            },
            setup_complete: parakeet_available,
        }
    }

    #[test]
    fn managed_runtime_requires_parakeet_even_when_whisper_is_selected() {
        let health = runtime_health_for_managed_runtime(TranscriptionEngine::WhisperCpp, false);

        assert!(!runtime_toolchain_ready(&health));
    }

    #[test]
    fn managed_runtime_is_ready_when_all_release_binaries_are_available() {
        let health = runtime_health_for_managed_runtime(TranscriptionEngine::WhisperCpp, true);

        assert!(runtime_toolchain_ready(&health));
    }
}
