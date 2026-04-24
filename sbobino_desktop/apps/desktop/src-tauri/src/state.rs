use std::{collections::HashMap, sync::Arc};

use sbobino_application::{ArtifactService, SettingsService};
use sbobino_infrastructure::{
    adapters::whisper_stream::WhisperStreamEngine, RuntimeTranscriptionFactory,
};
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::realtime_audio::RealtimeInputPreviewHandle;

pub struct TranscriptionTask {
    pub cancel_token: CancellationToken,
}

#[derive(Clone)]
pub struct RealtimeRuntime {
    pub engine: Arc<Mutex<WhisperStreamEngine>>,
    pub preview: Arc<Mutex<Option<RealtimeInputPreviewHandle>>>,
    pub session_name: Arc<Mutex<Option<String>>>,
    pub model_filename: Arc<Mutex<Option<String>>>,
    pub language_code: Arc<Mutex<String>>,
}

#[derive(Clone)]
pub struct ProvisioningRuntime {
    pub cancel_token: Arc<Mutex<Option<CancellationToken>>>,
}

#[derive(Clone)]
pub struct AppState {
    pub artifact_service: Arc<ArtifactService>,
    pub settings_service: Arc<SettingsService>,
    pub runtime_factory: Arc<RuntimeTranscriptionFactory>,
    pub transcription_tasks: Arc<Mutex<HashMap<String, TranscriptionTask>>>,
    // Serializes heavy transcription work. Multiple start_transcription
    // invocations return immediately with a job id, but only one job at a
    // time acquires this permit and runs whisper-cli + pyannote. Prevents
    // the webview from being killed when 3+ concurrent jobs exhaust RAM
    // and contend on the Metal device.
    pub transcription_gate: Arc<Semaphore>,
    pub realtime: RealtimeRuntime,
    pub provisioning: ProvisioningRuntime,
}
