mod ai_support;
mod commands;
mod error;
mod realtime_audio;
mod release_assets;
mod state;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use tauri::Manager;
#[cfg(target_os = "macos")]
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    Emitter,
};
use tokio::sync::{Mutex, Semaphore};
use tracing_subscriber::{fmt, EnvFilter};

use crate::commands::artifacts::{
    analyze_artifact_emotions, chat_artifact, delete_artifacts, empty_deleted_artifacts,
    export_artifact, generate_artifact_pack, get_artifact, hard_delete_artifacts, list_artifacts,
    list_deleted_artifacts, list_recent_artifacts, optimize_artifact, read_artifact_audio,
    read_audio_file, rename_artifact, restore_artifacts, summarize_artifact, update_artifact,
    update_artifact_timeline, write_trimmed_audio,
};
use crate::commands::automatic_import::{
    clear_automatic_import_quarantine_item, retry_automatic_import_quarantine_item,
    scan_automatic_import,
};
use crate::commands::backup::{export_app_backup, import_app_backup};
use crate::commands::provisioning::{
    plan_pyannote_background_action, provisioning_cancel, provisioning_download_model,
    provisioning_install_pyannote, provisioning_install_runtime, provisioning_models,
    provisioning_start, provisioning_status, read_setup_report, reconcile_post_update_runtime,
    write_setup_report,
};
use crate::commands::realtime::{
    list_realtime_sessions, load_realtime_session, pause_realtime, resume_realtime, start_realtime,
    stop_realtime,
};
use crate::commands::runtime::{
    ensure_transcription_runtime, get_realtime_start_readiness, get_transcription_runtime_health,
    get_transcription_start_preflight,
};
use crate::commands::settings::{
    delete_prompt, get_ai_capability_status, get_ai_providers, get_settings, get_settings_snapshot,
    list_gemini_models, list_prompts, reset_prompts, save_prompt, test_prompt, update_ai_providers,
    update_settings, update_settings_partial,
};
use crate::commands::transcription::{cancel_transcription, start_transcription};
use crate::commands::updates::check_updates;
use crate::commands::window::open_settings_window;
use crate::state::{ProvisioningRuntime, RealtimeRuntime};
use sbobino_infrastructure::adapters::whisper_stream::WhisperStreamEngine;

#[cfg(target_os = "macos")]
const MENU_CHECK_UPDATES_ID: &str = "app_menu_check_updates";
#[cfg(target_os = "macos")]
const MENU_CHECK_UPDATES_EVENT: &str = "app://menu-check-updates";

#[cfg(target_os = "macos")]
fn setup_macos_app_menu(app: &tauri::AppHandle) -> tauri::Result<()> {
    let menu = Menu::default(app)?;
    let check_updates_item = MenuItem::with_id(
        app,
        MENU_CHECK_UPDATES_ID,
        "Verifica disponibilita aggiornamenti...",
        true,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;

    if let Some(app_submenu) = menu
        .items()?
        .into_iter()
        .find_map(|item| item.as_submenu().cloned())
    {
        let insert_position = app_submenu.items()?.len().min(2);
        app_submenu.insert(&check_updates_item, insert_position)?;
        app_submenu.insert(&separator, insert_position + 1)?;
    } else {
        menu.append(&check_updates_item)?;
    }

    app.set_menu(menu)?;

    app.on_menu_event(|app_handle, event| {
        if event.id() != MENU_CHECK_UPDATES_ID {
            return;
        }
        if let Some(main_window) = app_handle.get_webview_window("main") {
            let _ = main_window.show();
            let _ = main_window.unminimize();
            let _ = main_window.set_focus();
            let _ = main_window.emit(MENU_CHECK_UPDATES_EVENT, ());
        } else {
            let _ = app_handle.emit(MENU_CHECK_UPDATES_EVENT, ());
        }
    });

    Ok(())
}

pub fn run() {
    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
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
            #[cfg(target_os = "macos")]
            setup_macos_app_menu(&app.handle())
                .map_err(|error| std::io::Error::other(format!("menu setup failure: {error}")))?;

            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                use window_vibrancy::{
                    apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState,
                };
                let _ = apply_vibrancy(
                    &window,
                    NSVisualEffectMaterial::UnderWindowBackground,
                    Some(NSVisualEffectState::Active),
                    None,
                );
            }

            let data_dir = app
                .path()
                .app_data_dir()
                .unwrap_or_else(|_| PathBuf::from("."));
            let resources_dir = app.path().resource_dir().ok();

            let bundle = sbobino_infrastructure::bootstrap(&data_dir, resources_dir)
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
                transcription_gate: Arc::new(Semaphore::new(1)),
                realtime: RealtimeRuntime {
                    engine: Arc::new(Mutex::new(realtime_engine)),
                    preview: Arc::new(Mutex::new(None)),
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
            scan_automatic_import,
            retry_automatic_import_quarantine_item,
            clear_automatic_import_quarantine_item,
            get_ai_providers,
            get_ai_capability_status,
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
            update_artifact_timeline,
            rename_artifact,
            delete_artifacts,
            restore_artifacts,
            hard_delete_artifacts,
            empty_deleted_artifacts,
            export_artifact,
            export_app_backup,
            chat_artifact,
            summarize_artifact,
            generate_artifact_pack,
            analyze_artifact_emotions,
            optimize_artifact,
            read_artifact_audio,
            read_audio_file,
            write_trimmed_audio,
            import_app_backup,
            start_realtime,
            pause_realtime,
            resume_realtime,
            stop_realtime,
            list_realtime_sessions,
            load_realtime_session,
            provisioning_status,
            provisioning_models,
            plan_pyannote_background_action,
            reconcile_post_update_runtime,
            provisioning_start,
            provisioning_download_model,
            provisioning_install_pyannote,
            provisioning_install_runtime,
            provisioning_cancel,
            read_setup_report,
            write_setup_report,
            ensure_transcription_runtime,
            get_realtime_start_readiness,
            get_transcription_runtime_health,
            get_transcription_start_preflight,
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
