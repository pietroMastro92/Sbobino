use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use sbobino_domain::{
    ArtifactKind, ArtifactSourceOrigin, LanguageCode, ParakeetModel, SpeechModel,
    TranscriptionEngine, WhisperOptions,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunTranscriptionRequest {
    pub job_id: String,
    pub input_path: String,
    pub engine: TranscriptionEngine,
    pub language: LanguageCode,
    pub model: SpeechModel,
    pub parakeet_model: ParakeetModel,
    pub enable_ai: bool,
    pub whisper_options: WhisperOptions,
    pub title: Option<String>,
    pub parent_id: Option<String>,
    pub source_origin: ArtifactSourceOrigin,
    pub metadata: BTreeMap<String, String>,
    pub source_fingerprint_json: Option<String>,
}

impl RunTranscriptionRequest {
    pub fn speech_model_filename(&self) -> &'static str {
        match self.engine {
            TranscriptionEngine::WhisperCpp => self.model.ggml_filename(),
            TranscriptionEngine::ParakeetCpp => self.parakeet_model.gguf_filename(),
        }
    }
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
    ReplaceFinal,
    UpdatePreview,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RealtimeDelta {
    pub kind: RealtimeDeltaKind,
    pub text: String,
}
