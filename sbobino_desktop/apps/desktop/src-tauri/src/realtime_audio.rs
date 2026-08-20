use cpal::traits::{DeviceTrait, HostTrait};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize)]
pub struct RealtimeTelemetry {
    pub captured_seconds: f32,
    pub processed_seconds: f32,
    pub backlog_seconds: f32,
    pub inference_ms: Option<f32>,
    pub first_preview_ms: Option<f32>,
    pub finalization_ms: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RealtimeInputLevelEvent {
    pub state: String,
    pub level: f32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry: Option<RealtimeTelemetry>,
}

#[derive(Debug, Clone)]
pub struct RealtimeInputError {
    pub reason_code: String,
    pub state: String,
    pub message: String,
}

fn clamp_level(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

fn map_input_error(reason_code: &str, detail: impl Into<String>) -> RealtimeInputError {
    let detail = detail.into();
    match reason_code {
        "microphone_blocked" => RealtimeInputError {
            reason_code: reason_code.to_string(),
            state: "blocked".to_string(),
            message: "Microphone access is blocked. Allow Sbobino in System Settings > Privacy & Security > Microphone.".to_string(),
        },
        "microphone_missing" => RealtimeInputError {
            reason_code: reason_code.to_string(),
            state: "unavailable".to_string(),
            message: "No audio input device is available.".to_string(),
        },
        "microphone_busy" => RealtimeInputError {
            reason_code: reason_code.to_string(),
            state: "unavailable".to_string(),
            message: format!(
                "The microphone is unavailable or in use by another app. {}",
                detail.trim()
            ),
        },
        _ => RealtimeInputError {
            reason_code: reason_code.to_string(),
            state: "unavailable".to_string(),
            message: format!("Microphone preview failed. {}", detail.trim()),
        },
    }
}

pub(crate) fn classify_input_error(detail: &str) -> RealtimeInputError {
    let lower = detail.to_ascii_lowercase();
    if lower.contains("not permitted")
        || lower.contains("permission")
        || lower.contains("denied")
        || lower.contains("unauthorized")
    {
        map_input_error("microphone_blocked", detail)
    } else if lower.contains("busy")
        || lower.contains("in use")
        || lower.contains("device not available")
        || lower.contains("cannot start")
        || lower.contains("couldn't")
    {
        map_input_error("microphone_busy", detail)
    } else {
        map_input_error("microphone_unavailable", detail)
    }
}

pub(crate) fn emit_level_event(
    app: &AppHandle,
    state: &str,
    level: f32,
    message: impl Into<String>,
) {
    let _ = app.emit(
        "realtime://input_level",
        RealtimeInputLevelEvent {
            state: state.to_string(),
            level: clamp_level(level),
            message: message.into(),
            telemetry: None,
        },
    );
}

pub fn probe_input_device_name() -> Result<String, RealtimeInputError> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| map_input_error("microphone_missing", ""))?;

    Ok(device
        .name()
        .unwrap_or_else(|_| "Default microphone".to_string()))
}
