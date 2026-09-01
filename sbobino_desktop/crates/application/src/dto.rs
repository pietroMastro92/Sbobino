use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use sbobino_domain::{
    ArtifactKind, ArtifactSourceOrigin, LanguageCode, ParakeetModel, SpeechModel,
    TranscriptionEngine, TranscriptionLanguagePolicy, WhisperOptions,
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
    pub fn language_policy(&self) -> TranscriptionLanguagePolicy {
        TranscriptionLanguagePolicy {
            preferred_language: self.language.clone(),
            // Selecting Auto opts into adaptive detection; a concrete
            // language is an explicit engine constraint.
            adaptive_detection: self.language.is_auto(),
        }
    }
    pub fn speech_model_filename(&self) -> &'static str {
        match self.engine {
            TranscriptionEngine::WhisperCpp => self.model.ggml_filename(),
            TranscriptionEngine::ParakeetCpp => self.parakeet_model.gguf_filename(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(language: LanguageCode) -> RunTranscriptionRequest {
        RunTranscriptionRequest {
            job_id: "language-policy-test".to_string(),
            input_path: "input.wav".to_string(),
            engine: TranscriptionEngine::WhisperCpp,
            language,
            model: SpeechModel::Base,
            parakeet_model: ParakeetModel::default(),
            enable_ai: false,
            whisper_options: WhisperOptions::default(),
            title: None,
            parent_id: None,
            source_origin: ArtifactSourceOrigin::Imported,
            metadata: BTreeMap::new(),
            source_fingerprint_json: None,
        }
    }

    #[test]
    fn concrete_language_disables_adaptive_detection() {
        let policy = request(LanguageCode::It).language_policy();
        assert_eq!(policy.preferred_language, LanguageCode::It);
        assert!(!policy.adaptive_detection);
        assert_eq!(policy.runtime_language_code(), "it");
    }

    #[test]
    fn auto_language_keeps_adaptive_detection_enabled() {
        let policy = request(LanguageCode::Auto).language_policy();
        assert!(policy.adaptive_detection);
        assert_eq!(policy.runtime_language_code(), "auto");
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_seconds: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_seconds: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language_confidence: Option<f32>,
}
