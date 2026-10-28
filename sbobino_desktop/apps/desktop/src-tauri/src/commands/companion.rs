use crate::{error::CommandError, state::AppState};
use sbobino_infrastructure::companion::{AiSettingsStatus, CompanionServer, Pairing};
use std::sync::Arc;
use tauri::{Emitter, State};

#[tauri::command]
pub async fn start_companion(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    lan: bool,
) -> Result<Pairing, CommandError> {
    let mut slot = state.companion.lock().await;
    if let Some(server) = slot.as_ref() {
        return Ok(server.pairing.clone());
    }
    let server = CompanionServer::start(
        state.runtime_factory.clone(),
        state.transcription_gate.clone(),
        lan,
        env!("CARGO_PKG_VERSION").into(),
        Arc::new(move |artifact| {
            let _ = app.emit("transcription://completed", artifact);
        }),
    )
    .await
    .map_err(|e| CommandError::new("companion", e))?;
    let pairing = server.pairing.clone();
    *slot = Some(server);
    Ok(pairing)
}

#[tauri::command]
pub async fn stop_companion(state: State<'_, AppState>) -> Result<(), CommandError> {
    let mut slot = state.companion.lock().await;
    if let Some(server) = slot.as_ref() {
        server.shutdown().await;
    }
    slot.take();
    Ok(())
}

#[tauri::command]
pub async fn companion_status(state: State<'_, AppState>) -> Result<Option<Pairing>, CommandError> {
    Ok(state
        .companion
        .lock()
        .await
        .as_ref()
        .map(|s| s.pairing.clone()))
}

#[tauri::command]
pub async fn companion_ai_status(
    state: State<'_, AppState>,
) -> Result<AiSettingsStatus, CommandError> {
    let service = state
        .companion
        .lock()
        .await
        .as_ref()
        .map(CompanionServer::ai_service)
        .ok_or_else(|| CommandError::new("companion", "Attiva prima il collegamento."))?;
    Ok(service.status())
}

#[tauri::command]
pub async fn select_companion_ai_provider(
    state: State<'_, AppState>,
    selection: Option<String>,
) -> Result<AiSettingsStatus, CommandError> {
    let service = state
        .companion
        .lock()
        .await
        .as_ref()
        .map(CompanionServer::ai_service)
        .ok_or_else(|| CommandError::new("companion", "Attiva prima il collegamento."))?;
    service
        .select(selection)
        .await
        .map_err(|message| CommandError::new("companion", message))
}
