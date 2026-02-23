use serde::Deserialize;
use tauri::{Emitter, Manager, TitleBarStyle};

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
        return Ok(false);
    }

    let mut settings_url = "index.html?window=settings".to_string();
    if let Some(target_pane) = pane.as_ref() {
        settings_url.push_str("&pane=");
        settings_url.push_str(target_pane);
    }

    let mut builder = tauri::WebviewWindowBuilder::new(
        &app,
        "settings",
        tauri::WebviewUrl::App(settings_url.into()),
    )
    .title("Settings")
    .inner_size(1024.0, 760.0)
    .min_inner_size(900.0, 620.0)
    .resizable(true)
    .transparent(true)
    .title_bar_style(TitleBarStyle::Overlay)
    .hidden_title(true);

    if let Some(main_window) = app.get_webview_window("main") {
        builder = builder.parent(&main_window).unwrap();
    }

    let settings_window = builder.build()
    .map_err(|error| {
        CommandError::new("window", format!("failed to open settings window: {error}"))
    })?;

    // Apply the same macOS vibrancy effect as the main window
    #[cfg(target_os = "macos")]
    {
        use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial, NSVisualEffectState};
        let _ = apply_vibrancy(
            &settings_window,
            NSVisualEffectMaterial::Sidebar,
            Some(NSVisualEffectState::Active),
            None,
        );
    }

    Ok(true)
}
