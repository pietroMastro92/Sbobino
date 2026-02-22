use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum LanguageCode {
    #[default]
    Auto,
    En,
    It,
    Fr,
    De,
    Es,
    Pt,
    Zh,
    Ja,
}

impl LanguageCode {
    pub fn as_whisper_code(&self) -> &str {
        match self {
            Self::Auto => "auto",
            Self::En => "en",
            Self::It => "it",
            Self::Fr => "fr",
            Self::De => "de",
            Self::Es => "es",
            Self::Pt => "pt",
            Self::Zh => "zh",
            Self::Ja => "ja",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpeechModel {
    Tiny,
    #[default]
    Base,
    Small,
    Medium,
    LargeTurbo,
}

impl SpeechModel {
    pub fn ggml_filename(&self) -> &'static str {
        match self {
            Self::Tiny => "ggml-tiny.bin",
            Self::Base => "ggml-base.bin",
            Self::Small => "ggml-small.bin",
            Self::Medium => "ggml-medium.bin",
            Self::LargeTurbo => "ggml-large-v3-turbo-q8_0.bin",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptionEngine {
    WhisperKit,
    #[default]
    WhisperCpp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AiProvider {
    #[default]
    None,
    FoundationApple,
    Gemini,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RemoteServiceKind {
    #[default]
    Google,
    OpenAi,
    Anthropic,
    Azure,
    LmStudio,
    Ollama,
    OpenRouter,
    Xai,
    HuggingFace,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RemoteServiceConfig {
    pub id: String,
    pub kind: RemoteServiceKind,
    pub label: String,
    pub enabled: bool,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromptCategory {
    Cleanup,
    Summary,
    Insights,
    Qa,
    Rewrite,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptTemplate {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub category: PromptCategory,
    pub body: String,
    pub builtin: bool,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralSettings {
    pub auto_update_enabled: bool,
    pub auto_update_repo: String,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            auto_update_enabled: true,
            auto_update_repo: "pietroMastro92/sbobbino".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WhisperOptions {
    // Shared behavior
    pub translate_to_english: bool,
    // whisper.cpp-focused controls
    pub no_context: bool,
    pub split_on_word: bool,
    // Shared thresholds / decoding controls
    pub temperature: f32,
    pub temperature_increment_on_fallback: f32,
    pub temperature_fallback_count: u8,
    pub entropy_threshold: f32,
    pub logprob_threshold: f32,
    pub first_token_logprob_threshold: f32,
    pub no_speech_threshold: f32,
    pub word_threshold: f32,
    pub best_of: u8,
    pub beam_size: u8,
    pub threads: u8,
    pub processors: u8,
    // WhisperKit-focused controls
    pub use_prefill_prompt: bool,
    pub use_prefill_cache: bool,
    pub without_timestamps: bool,
    pub word_timestamps: bool,
    pub prompt: Option<String>,
    pub concurrent_worker_count: u8,
    pub chunking_strategy: String,
    pub audio_encoder_compute_units: String,
    pub text_decoder_compute_units: String,
}

impl Default for WhisperOptions {
    fn default() -> Self {
        // Use half of logical CPUs (clamped 4–16) for optimal throughput on Intel & Apple Silicon
        let logical_cpus = num_cpus::get() as u8;
        let optimal_threads = (logical_cpus / 2).clamp(4, 16);

        Self {
            translate_to_english: false,
            no_context: false,
            split_on_word: false,
            temperature: 0.0,
            temperature_increment_on_fallback: 0.2,
            temperature_fallback_count: 5,
            entropy_threshold: 2.5,
            logprob_threshold: -1.0,
            first_token_logprob_threshold: -1.5,
            no_speech_threshold: 0.6,
            word_threshold: 0.01,
            best_of: 5,
            beam_size: 5,
            threads: optimal_threads,
            processors: 1,
            use_prefill_prompt: true,
            use_prefill_cache: true,
            without_timestamps: false,
            word_timestamps: false,
            prompt: None,
            concurrent_worker_count: 4,
            chunking_strategy: "vad".to_string(),
            audio_encoder_compute_units: "cpu_and_neural_engine".to_string(),
            text_decoder_compute_units: "cpu_and_neural_engine".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TranscriptionSettings {
    pub engine: TranscriptionEngine,
    pub model: SpeechModel,
    pub language: LanguageCode,
    pub whisper_cli_path: String,
    pub whisperkit_cli_path: String,
    pub ffmpeg_path: String,
    pub models_dir: String,
    pub enable_ai_post_processing: bool,
    pub whisper_options: WhisperOptions,
}

impl Default for TranscriptionSettings {
    fn default() -> Self {
        Self {
            engine: TranscriptionEngine::default(),
            model: SpeechModel::Base,
            language: LanguageCode::Auto,
            whisper_cli_path: "whisper-cli".to_string(),
            whisperkit_cli_path: "whisperkit-cli".to_string(),
            ffmpeg_path: "ffmpeg".to_string(),
            models_dir: "models".to_string(),
            enable_ai_post_processing: false,
            whisper_options: WhisperOptions::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FoundationProviderSettings {
    pub enabled: bool,
}

impl Default for FoundationProviderSettings {
    fn default() -> Self {
        Self { enabled: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GeminiProviderSettings {
    pub api_key: Option<String>,
    pub model: String,
}

impl Default for GeminiProviderSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            model: "gemini-2.5-flash-lite".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AiProviderSettings {
    pub foundation_apple: FoundationProviderSettings,
    pub gemini: GeminiProviderSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    pub active_provider: AiProvider,
    pub active_remote_service_id: Option<String>,
    pub providers: AiProviderSettings,
    pub remote_services: Vec<RemoteServiceConfig>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            active_provider: AiProvider::None,
            active_remote_service_id: None,
            providers: AiProviderSettings::default(),
            remote_services: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PromptBindings {
    pub optimize_prompt_id: String,
    pub summary_prompt_id: String,
    pub faq_prompt_id: String,
}

impl Default for PromptBindings {
    fn default() -> Self {
        Self {
            optimize_prompt_id: "builtin_improve_grammar".to_string(),
            summary_prompt_id: "builtin_bullet_points".to_string(),
            faq_prompt_id: "builtin_generate_faq".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PromptSettings {
    pub templates: Vec<PromptTemplate>,
    pub bindings: PromptBindings,
}

impl Default for PromptSettings {
    fn default() -> Self {
        Self {
            templates: default_prompt_templates(),
            bindings: PromptBindings::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromptTask {
    Optimize,
    Summary,
    Faq,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    // Legacy-compatible flat fields used by current app flows.
    pub transcription_engine: TranscriptionEngine,
    pub model: SpeechModel,
    pub language: LanguageCode,
    pub ai_post_processing: bool,
    pub gemini_model: String,
    pub gemini_api_key: Option<String>,
    pub whisper_cli_path: String,
    pub whisperkit_cli_path: String,
    pub ffmpeg_path: String,
    pub models_dir: String,
    pub auto_update_enabled: bool,
    pub auto_update_repo: String,

    // New structured settings for Whisper-style settings workspace.
    pub general: GeneralSettings,
    pub transcription: TranscriptionSettings,
    pub ai: AiSettings,
    pub prompts: PromptSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        let general = GeneralSettings::default();
        let transcription = TranscriptionSettings::default();
        let ai = AiSettings::default();
        let prompts = PromptSettings::default();

        Self {
            transcription_engine: transcription.engine.clone(),
            model: transcription.model.clone(),
            language: transcription.language.clone(),
            ai_post_processing: transcription.enable_ai_post_processing,
            gemini_model: ai.providers.gemini.model.clone(),
            gemini_api_key: ai.providers.gemini.api_key.clone(),
            whisper_cli_path: transcription.whisper_cli_path.clone(),
            whisperkit_cli_path: transcription.whisperkit_cli_path.clone(),
            ffmpeg_path: transcription.ffmpeg_path.clone(),
            models_dir: transcription.models_dir.clone(),
            auto_update_enabled: general.auto_update_enabled,
            auto_update_repo: general.auto_update_repo.clone(),
            general,
            transcription,
            ai,
            prompts,
        }
    }
}

impl AppSettings {
    pub fn sync_sections_from_legacy(&mut self) {
        self.general.auto_update_enabled = self.auto_update_enabled;
        self.general.auto_update_repo = self.auto_update_repo.clone();

        self.transcription.engine = self.transcription_engine.clone();
        self.transcription.model = self.model.clone();
        self.transcription.language = self.language.clone();
        self.transcription.whisper_cli_path = self.whisper_cli_path.clone();
        self.transcription.whisperkit_cli_path = self.whisperkit_cli_path.clone();
        self.transcription.ffmpeg_path = self.ffmpeg_path.clone();
        self.transcription.models_dir = self.models_dir.clone();
        self.transcription.enable_ai_post_processing = self.ai_post_processing;

        self.ai.providers.gemini.model = self.gemini_model.clone();
        self.ai.providers.gemini.api_key = self.gemini_api_key.clone();
        if self.ai.active_provider == AiProvider::None && self.gemini_api_key.is_some() {
            self.ai.active_provider = AiProvider::Gemini;
        }
        if self.ai.active_remote_service_id.is_none()
            && self.ai.active_provider == AiProvider::Gemini
        {
            self.ai.active_remote_service_id = self
                .ai
                .remote_services
                .iter()
                .find(|service| service.kind == RemoteServiceKind::Google)
                .map(|service| service.id.clone());
        }
        if let Some(active_id) = self.ai.active_remote_service_id.clone() {
            let exists = self
                .ai
                .remote_services
                .iter()
                .any(|service| service.id == active_id);
            if !exists {
                self.ai.active_remote_service_id = None;
            }
        }

        self.ensure_prompt_integrity();
    }

    pub fn sync_legacy_from_sections(&mut self) {
        self.auto_update_enabled = self.general.auto_update_enabled;
        self.auto_update_repo = self.general.auto_update_repo.clone();

        self.transcription_engine = self.transcription.engine.clone();
        self.model = self.transcription.model.clone();
        self.language = self.transcription.language.clone();
        self.whisper_cli_path = self.transcription.whisper_cli_path.clone();
        self.whisperkit_cli_path = self.transcription.whisperkit_cli_path.clone();
        self.ffmpeg_path = self.transcription.ffmpeg_path.clone();
        self.models_dir = self.transcription.models_dir.clone();
        self.ai_post_processing = self.transcription.enable_ai_post_processing;

        self.gemini_model = self.ai.providers.gemini.model.clone();
        self.gemini_api_key = self.ai.providers.gemini.api_key.clone();
        if self.ai.active_provider == AiProvider::Gemini
            && self.ai.active_remote_service_id.is_none()
        {
            self.ai.active_remote_service_id = self
                .ai
                .remote_services
                .iter()
                .find(|service| service.kind == RemoteServiceKind::Google)
                .map(|service| service.id.clone());
        }
        if let Some(active_id) = self.ai.active_remote_service_id.clone() {
            let exists = self
                .ai
                .remote_services
                .iter()
                .any(|service| service.id == active_id);
            if !exists {
                self.ai.active_remote_service_id = None;
            }
        }

        self.ensure_prompt_integrity();
    }

    pub fn ensure_prompt_integrity(&mut self) {
        if self.prompts.templates.is_empty() {
            self.prompts.templates = default_prompt_templates();
        }

        let has_optimize = self
            .prompts
            .templates
            .iter()
            .any(|template| template.id == self.prompts.bindings.optimize_prompt_id);
        if !has_optimize {
            self.prompts.bindings.optimize_prompt_id = PromptBindings::default().optimize_prompt_id;
        }

        let has_summary = self
            .prompts
            .templates
            .iter()
            .any(|template| template.id == self.prompts.bindings.summary_prompt_id);
        if !has_summary {
            self.prompts.bindings.summary_prompt_id = PromptBindings::default().summary_prompt_id;
        }

        let has_faq = self
            .prompts
            .templates
            .iter()
            .any(|template| template.id == self.prompts.bindings.faq_prompt_id);
        if !has_faq {
            self.prompts.bindings.faq_prompt_id = PromptBindings::default().faq_prompt_id;
        }
    }

    pub fn prompt_for_task(&self, task: PromptTask) -> Option<String> {
        let template_id = match task {
            PromptTask::Optimize => &self.prompts.bindings.optimize_prompt_id,
            PromptTask::Summary => &self.prompts.bindings.summary_prompt_id,
            PromptTask::Faq => &self.prompts.bindings.faq_prompt_id,
        };

        self.prompts
            .templates
            .iter()
            .find(|template| &template.id == template_id)
            .map(|template| template.body.clone())
    }
}

pub fn default_prompt_templates() -> Vec<PromptTemplate> {
    vec![
        PromptTemplate {
            id: "builtin_bullet_points".to_string(),
            name: "Bullet Points".to_string(),
            icon: "list".to_string(),
            category: PromptCategory::Summary,
            body: "Turn this transcript into a concise bullet-point summary with key takeaways."
                .to_string(),
            builtin: true,
            updated_at: "".to_string(),
        },
        PromptTemplate {
            id: "builtin_improve_grammar".to_string(),
            name: "Improve Grammar & Punctuation".to_string(),
            icon: "abc".to_string(),
            category: PromptCategory::Cleanup,
            body:
                "Improve spelling, grammar, and punctuation while preserving the original meaning."
                    .to_string(),
            builtin: true,
            updated_at: "".to_string(),
        },
        PromptTemplate {
            id: "builtin_split_paragraphs".to_string(),
            name: "Split Into Paragraphs".to_string(),
            icon: "paragraphs".to_string(),
            category: PromptCategory::Cleanup,
            body: "Split transcript text into readable paragraphs with logical breaks.".to_string(),
            builtin: true,
            updated_at: "".to_string(),
        },
        PromptTemplate {
            id: "builtin_highlight_key_points".to_string(),
            name: "Highlight Key Points".to_string(),
            icon: "star".to_string(),
            category: PromptCategory::Insights,
            body: "Identify and highlight the most important points in this transcript."
                .to_string(),
            builtin: true,
            updated_at: "".to_string(),
        },
        PromptTemplate {
            id: "builtin_extract_questions".to_string(),
            name: "Extract Questions".to_string(),
            icon: "question".to_string(),
            category: PromptCategory::Qa,
            body: "Extract all explicit and implicit questions from this transcript.".to_string(),
            builtin: true,
            updated_at: "".to_string(),
        },
        PromptTemplate {
            id: "builtin_identify_emotions".to_string(),
            name: "Identify Emotions".to_string(),
            icon: "smile".to_string(),
            category: PromptCategory::Insights,
            body: "Identify emotions and sentiment changes throughout the transcript.".to_string(),
            builtin: true,
            updated_at: "".to_string(),
        },
        PromptTemplate {
            id: "builtin_generate_faq".to_string(),
            name: "Generate FAQ".to_string(),
            icon: "faq".to_string(),
            category: PromptCategory::Qa,
            body:
                "Generate a FAQ from this transcript with concise Q/A pairs and practical answers."
                    .to_string(),
            builtin: true,
            updated_at: "".to_string(),
        },
        PromptTemplate {
            id: "builtin_extract_statistics".to_string(),
            name: "Extract Statistics".to_string(),
            icon: "stats".to_string(),
            category: PromptCategory::Insights,
            body: "Extract all numbers, metrics, and statistical statements from this transcript."
                .to_string(),
            builtin: true,
            updated_at: "".to_string(),
        },
        PromptTemplate {
            id: "builtin_paraphrase".to_string(),
            name: "Paraphrase Content".to_string(),
            icon: "paraphrase".to_string(),
            category: PromptCategory::Rewrite,
            body: "Rewrite this transcript with clearer wording while preserving meaning."
                .to_string(),
            builtin: true,
            updated_at: "".to_string(),
        },
        PromptTemplate {
            id: "builtin_mindmap".to_string(),
            name: "Create a Mindmap".to_string(),
            icon: "mindmap".to_string(),
            category: PromptCategory::Insights,
            body: "Create a hierarchical mindmap structure from the transcript content."
                .to_string(),
            builtin: true,
            updated_at: "".to_string(),
        },
    ]
}
