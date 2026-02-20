mod commands;
mod error;
mod state;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;
use tokio::sync::Mutex;
use tracing_subscriber::{fmt, EnvFilter};

use crate::commands::artifacts::{
    chat_artifact, delete_artifacts, empty_deleted_artifacts, export_artifact, get_artifact,
    hard_delete_artifacts, list_artifacts, list_deleted_artifacts, list_recent_artifacts,
    read_audio_file, rename_artifact, restore_artifacts, summarize_artifact, update_artifact,
};
use crate::commands::provisioning::{
    provisioning_cancel, provisioning_download_model, provisioning_models, provisioning_start,
    provisioning_status,
};
use crate::commands::realtime::{
    list_realtime_sessions, load_realtime_session, pause_realtime, resume_realtime, start_realtime,
    stop_realtime,
};
use crate::commands::runtime::get_transcription_runtime_health;
use crate::commands::settings::{
    delete_prompt, get_ai_providers, get_settings, get_settings_snapshot, list_gemini_models,
    list_prompts, reset_prompts, save_prompt, test_prompt, update_ai_providers, update_settings,
    update_settings_partial,
};
use crate::commands::transcription::{cancel_transcription, start_transcription};
use crate::commands::updates::check_updates;
use crate::commands::window::open_settings_window;
use crate::state::{ProvisioningRuntime, RealtimeRuntime};
use sbobino_infrastructure::adapters::whisper_stream::WhisperStreamEngine;

pub fn run() {
    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if window.label() == "main" {
                    let app = window.app_handle();
                    let secondary_labels = app
                        .webview_windows()
                        .keys()
                        .filter(|label| label.as_str() != "main")
                        .cloned()
                        .collect::<Vec<_>>();

                    for label in secondary_labels {
                        if let Some(secondary) = app.get_webview_window(label.as_str()) {
                            let _ = secondary.close();
                        }
                    }
                }
            }
        })
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| PathBuf::from("."));

            let bundle = sbobino_infrastructure::bootstrap(&data_dir)
                .map_err(|e| std::io::Error::other(format!("bootstrap failure: {e}")))?;

            let realtime_engine = bundle
                .runtime_factory
                .build_whisper_stream_engine()
                .unwrap_or_else(|_| {
                    let whisper_stream_path = bundle
                        .runtime_factory
                        .resolve_binary_path("whisper-stream", "whisper-stream");
                    let models_dir = bundle.runtime_factory.resolve_models_dir("models");
                    WhisperStreamEngine::new(whisper_stream_path, models_dir)
                });

            app.manage(state::AppState {
                artifact_service: bundle.artifact_service,
                settings_service: bundle.settings_service,
                runtime_factory: bundle.runtime_factory,
                transcription_tasks: Arc::new(Mutex::new(HashMap::new())),
                realtime: RealtimeRuntime {
                    engine: realtime_engine,
                    session_name: Arc::new(Mutex::new(None)),
                    model_filename: Arc::new(Mutex::new(None)),
                    language_code: Arc::new(Mutex::new("auto".to_string())),
                },
                provisioning: ProvisioningRuntime {
                    cancel_token: Arc::new(Mutex::new(None)),
                },
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            update_settings,
            get_settings_snapshot,
            update_settings_partial,
            get_ai_providers,
            update_ai_providers,
            list_gemini_models,
            list_prompts,
            save_prompt,
            delete_prompt,
            reset_prompts,
            test_prompt,
            start_transcription,
            cancel_transcription,
            list_artifacts,
            list_deleted_artifacts,
            list_recent_artifacts,
            get_artifact,
            update_artifact,
            rename_artifact,
            delete_artifacts,
            restore_artifacts,
            hard_delete_artifacts,
            empty_deleted_artifacts,
            export_artifact,
            chat_artifact,
            summarize_artifact,
            read_audio_file,
            start_realtime,
            pause_realtime,
            resume_realtime,
            stop_realtime,
            list_realtime_sessions,
            load_realtime_session,
            provisioning_status,
            provisioning_models,
            provisioning_start,
            provisioning_download_model,
            provisioning_cancel,
            get_transcription_runtime_health,
            check_updates,
            open_settings_window,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run sbobino desktop app");
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).with_target(false).try_init();
}
