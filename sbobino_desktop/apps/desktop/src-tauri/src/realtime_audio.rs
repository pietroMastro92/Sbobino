use std::sync::{
    atomic::{AtomicU32, Ordering},
    mpsc, Arc, Mutex,
};
use std::thread;
use std::time::Duration;

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    BuildStreamError, SampleFormat, Stream, StreamConfig,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize)]
pub struct RealtimeInputLevelEvent {
    pub state: String,
    pub level: f32,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct RealtimeInputError {
    pub reason_code: String,
    pub state: String,
    pub message: String,
}

pub struct RealtimeInputPreviewHandle {
    shutdown_tx: mpsc::Sender<()>,
}

fn clamp_level(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

fn mean_abs_level(samples: impl Iterator<Item = f32>) -> f32 {
    let mut sum = 0.0_f32;
    let mut count = 0_u32;
    for sample in samples {
        sum += sample.abs();
        count += 1;
    }

    if count == 0 {
        return 0.0;
    }

    let normalized = (sum / count as f32) * 3.2;
    clamp_level(normalized)
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

fn classify_input_error(detail: &str) -> RealtimeInputError {
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

fn emit_level_event(app: &AppHandle, state: &str, level: f32, message: impl Into<String>) {
    let _ = app.emit(
        "realtime://input_level",
        RealtimeInputLevelEvent {
            state: state.to_string(),
            level: clamp_level(level),
            message: message.into(),
        },
    );
}

fn build_stream_from_config(
    device: &cpal::Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    level_bits: Arc<AtomicU32>,
    last_error: Arc<Mutex<Option<String>>>,
) -> Result<Stream, BuildStreamError> {
    match sample_format {
        SampleFormat::F32 => {
            let level_slot = level_bits.clone();
            let error_slot = last_error.clone();
            device.build_input_stream(
                config,
                move |data: &[f32], _| {
                    let level = mean_abs_level(data.iter().copied());
                    level_slot.store(level.to_bits(), Ordering::Relaxed);
                },
                move |error| {
                    if let Ok(mut slot) = error_slot.lock() {
                        *slot = Some(error.to_string());
                    }
                },
                None,
            )
        }
        SampleFormat::I16 => {
            let level_slot = level_bits.clone();
            let error_slot = last_error.clone();
            device.build_input_stream(
                config,
                move |data: &[i16], _| {
                    let level =
                        mean_abs_level(data.iter().map(|sample| *sample as f32 / i16::MAX as f32));
                    level_slot.store(level.to_bits(), Ordering::Relaxed);
                },
                move |error| {
                    if let Ok(mut slot) = error_slot.lock() {
                        *slot = Some(error.to_string());
                    }
                },
                None,
            )
        }
        SampleFormat::U16 => {
            let level_slot = level_bits.clone();
            let error_slot = last_error.clone();
            device.build_input_stream(
                config,
                move |data: &[u16], _| {
                    let level = mean_abs_level(
                        data.iter()
                            .map(|sample| (*sample as f32 / u16::MAX as f32) * 2.0 - 1.0),
                    );
                    level_slot.store(level.to_bits(), Ordering::Relaxed);
                },
                move |error| {
                    if let Ok(mut slot) = error_slot.lock() {
                        *slot = Some(error.to_string());
                    }
                },
                None,
            )
        }
        _ => Err(BuildStreamError::StreamConfigNotSupported),
    }
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

pub fn start_input_preview(
    app: &AppHandle,
) -> Result<RealtimeInputPreviewHandle, RealtimeInputError> {
    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
    let (startup_tx, startup_rx) = mpsc::channel::<Result<(), RealtimeInputError>>();
    let app_handle = app.clone();

    thread::spawn(move || {
        let host = cpal::default_host();
        let device = match host.default_input_device() {
            Some(value) => value,
            None => {
                let _ = startup_tx.send(Err(map_input_error("microphone_missing", "")));
                return;
            }
        };
        let device_name = device
            .name()
            .unwrap_or_else(|_| "Default microphone".to_string());
        let supported_config = match device.default_input_config() {
            Ok(value) => value,
            Err(error) => {
                let _ = startup_tx.send(Err(classify_input_error(&error.to_string())));
                return;
            }
        };
        let level_bits = Arc::new(AtomicU32::new(0.0_f32.to_bits()));
        let last_error = Arc::new(Mutex::new(None::<String>));
        let stream = match build_stream_from_config(
            &device,
            &supported_config.config(),
            supported_config.sample_format(),
            level_bits.clone(),
            last_error.clone(),
        ) {
            Ok(value) => value,
            Err(error) => {
                let _ = startup_tx.send(Err(classify_input_error(&error.to_string())));
                return;
            }
        };
        if let Err(error) = stream.play() {
            let _ = startup_tx.send(Err(classify_input_error(&error.to_string())));
            return;
        }
        let _stream = stream;
        let _ = startup_tx.send(Ok(()));
        emit_level_event(&app_handle, "running", 0.0, format!("Using {device_name}"));

        loop {
            match shutdown_rx.recv_timeout(Duration::from_millis(45)) {
                Ok(_) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    emit_level_event(&app_handle, "idle", 0.0, "Microphone preview stopped.");
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let message = if let Ok(mut slot) = last_error.lock() {
                        slot.take()
                    } else {
                        None
                    };
                    if let Some(detail) = message {
                        let error = classify_input_error(&detail);
                        emit_level_event(&app_handle, &error.state, 0.0, error.message);
                        continue;
                    }

                    let level = f32::from_bits(level_bits.load(Ordering::Relaxed));
                    emit_level_event(
                        &app_handle,
                        "running",
                        level,
                        format!("Using {device_name}"),
                    );
                }
            }
        }
    });

    match startup_rx.recv_timeout(Duration::from_secs(60)) {
        Ok(Ok(())) => Ok(RealtimeInputPreviewHandle { shutdown_tx }),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(map_input_error(
            "microphone_unavailable",
            "Microphone preview startup timed out while waiting for the input device.",
        )),
    }
}

impl RealtimeInputPreviewHandle {
    pub fn stop(self, app: &AppHandle, final_state: &str, message: &str) {
        let _ = self.shutdown_tx.send(());
        emit_level_event(app, final_state, 0.0, message.to_string());
    }
}
