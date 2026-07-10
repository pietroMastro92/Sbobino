use serde::Deserialize;
#[cfg(target_os = "macos")]
use tauri::TitleBarStyle;
use tauri::{Emitter, Manager};

use crate::error::CommandError;

#[derive(Debug, Deserialize)]
pub struct OpenSettingsWindowPayload {
    pub pane: Option<String>,
}

#[tauri::command]
pub async fn open_settings_window(
    app: tauri::AppHandle,
    payload: Option<OpenSettingsWindowPayload>,
) -> Result<bool, CommandError> {
    let pane = payload
        .and_then(|value| value.pane)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if let Some(window) = app.get_webview_window("settings") {
        if let Some(target_pane) = pane.as_ref() {
            let _ = window.emit("settings://navigate", target_pane.clone());
        }
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        if let Some(main_win) = app.get_webview_window("main") {
            let _ = main_win.emit("settings_opened", ());
        }
        return Ok(false);
    }

    let mut settings_url = "index.html?window=settings".to_string();
    if let Some(target_pane) = pane.as_ref() {
        settings_url.push_str("&pane=");
        settings_url.push_str(target_pane);
    }

    let main_window = app.get_webview_window("main");

    let builder = tauri::WebviewWindowBuilder::new(
        &app,
        "settings",
        tauri::WebviewUrl::App(settings_url.into()),
    )
    .title("Settings")
    .inner_size(1024.0, 760.0)
    .min_inner_size(900.0, 620.0)
    .resizable(true)
    .transparent(false);

    #[cfg(target_os = "macos")]
    let mut builder = builder
        .title_bar_style(TitleBarStyle::Overlay)
        .hidden_title(true);
    #[cfg(not(target_os = "macos"))]
    let mut builder = builder;

    // Make settings an owned child of the main window.
    // On macOS this keeps settings above main and prevents main from being focused.
    if let Some(ref parent) = main_window {
        builder = builder.parent(parent).map_err(|error| {
            CommandError::new("window", format!("failed to set parent window: {error}"))
        })?;
    }

    let settings_window = builder.build().map_err(|error| {
        CommandError::new("window", format!("failed to open settings window: {error}"))
    })?;

    if let Some(main_win) = main_window {
        let _ = main_win.emit("settings_opened", ());
        let app_handle = app.clone();
        settings_window.on_window_event(move |event| match event {
            tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed => {
                if let Some(main) = app_handle.get_webview_window("main") {
                    let _ = main.emit("settings_closed", ());
                }
            }
            _ => {}
        });
    }

    Ok(true)
}
