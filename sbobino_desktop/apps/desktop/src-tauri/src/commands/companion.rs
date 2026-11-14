use crate::{error::CommandError, state::AppState};
use sbobino_infrastructure::companion::{
    persisted_pairing_state, revoke_persisted_pairing, AiSettingsStatus, CompanionServer, Pairing,
    PersistedPairingState,
};
use std::sync::Arc;
use tauri::{Emitter, State};

/// Wire shape shared by the status and the explicit revocation commands.
///
/// Inactive states never carry the pairing payload: the certificate, bearer
/// token and private material stay on the desktop side until a listener is
/// actually running, and a saved pairing is reported as saved, not as valid.
#[derive(serde::Serialize)]
pub struct CompanionPairingState {
    pub state: &'static str,
    pub pairing: Option<Pairing>,
}

fn pairing_state(
    active: Option<Pairing>,
    persisted: PersistedPairingState,
) -> CompanionPairingState {
    if let Some(pairing) = active {
        return CompanionPairingState {
            state: "active",
            pairing: Some(pairing),
        };
    }
    let state = match persisted {
        PersistedPairingState::Absent => "inactive_unpaired",
        PersistedPairingState::Present => "inactive_paired",
        PersistedPairingState::Unknown => "unknown",
    };
    CompanionPairingState {
        state,
        pairing: None,
    }
}

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

/// Suspends the listener but keeps the saved pairing: a later explicit start
/// reuses the same certificate, key, token and port. Remote jobs are in memory,
/// so the drain interrupts them; their result is not recoverable.
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

/// Metadata-only companion state for the UI.
///
/// The same `state.companion` lock serializes this read against start, stop and
/// revoke. The persisted side is a read-only file-presence check on the profile
/// directory: no store, lease, record or Keychain entry is created just to
/// answer, and nothing is decrypted or started.
#[tauri::command]
pub async fn companion_pairing_state(
    state: State<'_, AppState>,
) -> Result<CompanionPairingState, CommandError> {
    let slot = state.companion.lock().await;
    let active = slot.as_ref().map(|server| server.pairing.clone());
    let persisted = persisted_pairing_state(state.runtime_factory.data_dir());
    Ok(pairing_state(active, persisted))
}

/// Explicit trust revocation: stop the listener (if any), drop the slot, then
/// remove only the saved pairing record.
///
/// Everything runs under the single `state.companion` lock, so a revoke cannot
/// overlap a start, stop or status read. Jobs, audio, artifacts, AI state and
/// the master key are untouched. A failure from the persistence hook is
/// reported and never turned into a claimed success; the UI must reload the
/// real state afterwards.
#[tauri::command]
pub async fn revoke_companion_pairing(
    state: State<'_, AppState>,
) -> Result<CompanionPairingState, CommandError> {
    let mut slot = state.companion.lock().await;
    if let Some(server) = slot.as_ref() {
        server.shutdown().await;
    }
    let _ = slot.take();
    revoke_persisted_pairing(state.runtime_factory.data_dir())
        .map_err(|message| CommandError::new("companion", message))?;
    let persisted = persisted_pairing_state(state.runtime_factory.data_dir());
    Ok(pairing_state(None, persisted))
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
