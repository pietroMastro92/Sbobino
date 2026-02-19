use serde::{Deserialize, Serialize};

use sbobino_domain::{ArtifactKind, LanguageCode, SpeechModel, WhisperOptions};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTranscriptionRequest {
    pub job_id: String,
    pub input_path: String,
    pub language: LanguageCode,
    pub model: SpeechModel,
    pub enable_ai: bool,
    pub whisper_options: WhisperOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SummaryFaq {
    pub summary: String,
    pub faqs: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArtifactQuery {
    pub kind: Option<ArtifactKind>,
    pub query: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RealtimeDeltaKind {
    AppendFinal,
    UpdatePreview,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealtimeDelta {
    pub kind: RealtimeDeltaKind,
    pub text: String,
}
