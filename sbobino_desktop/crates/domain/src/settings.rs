use std::collections::BTreeMap;

use serde::{de::Error as DeError, Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
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
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppLanguage {
    #[default]
    En,
    It,
    Es,
    De,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptionComputeDevice {
    #[default]
    Auto,
    Gpu,
    Cpu,
}

impl LanguageCode {
    pub fn from_code(value: &str) -> Self {
        Self::try_from_code(value).unwrap_or_default()
    }

    pub fn try_from_code(value: &str) -> Result<Self, String> {
        let normalized = normalize_language_code(value)?;
        Ok(match normalized.as_str() {
            "auto" => Self::Auto,
            "en" => Self::En,
            "it" => Self::It,
            "fr" => Self::Fr,
            "de" => Self::De,
            "es" => Self::Es,
            "pt" => Self::Pt,
            "zh" => Self::Zh,
            "ja" => Self::Ja,
            _ => Self::Custom(normalized),
        })
    }

    pub fn is_auto(&self) -> bool {
        matches!(self, Self::Auto)
    }

    pub fn as_code(&self) -> &str {
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
            Self::Custom(value) => value.as_str(),
        }
    }

    pub fn as_whisper_code(&self) -> &str {
        self.as_code()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranscriptionLanguagePolicy {
    #[serde(default)]
    pub preferred_language: LanguageCode,
    #[serde(default = "default_true")]
    pub adaptive_detection: bool,
}

fn default_true() -> bool {
    true
}

impl Default for TranscriptionLanguagePolicy {
    fn default() -> Self {
        Self {
            preferred_language: LanguageCode::Auto,
            adaptive_detection: true,
        }
    }
}

impl Serialize for LanguageCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_code())
    }
}

impl<'de> Deserialize<'de> for LanguageCode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from_code(&value).map_err(D::Error::custom)
    }
}

fn normalize_language_code(value: &str) -> Result<String, String> {
    let normalized = value.trim().replace('_', "-").to_ascii_lowercase();
    if normalized.is_empty() {
        return Err("language code cannot be empty".to_string());
    }
    if normalized == "auto" {
        return Ok(normalized);
    }

    let subtags = normalized.split('-').collect::<Vec<_>>();
    let primary = subtags.first().copied().unwrap_or_default();
    if !(2..=3).contains(&primary.len())
        || !primary
            .chars()
            .all(|character| character.is_ascii_alphabetic())
        || subtags.iter().skip(1).any(|subtag| {
            !(2..=8).contains(&subtag.len())
                || !subtag
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
        })
    {
        return Err(format!("invalid language code: {value}"));
    }

    Ok(primary.to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TranscriptionLanguageOption {
    pub code: String,
    pub whisper: bool,
    pub parakeet_tdt: bool,
    pub nemotron: bool,
}

pub fn transcription_language_catalog() -> Vec<TranscriptionLanguageOption> {
    const WHISPER_CODES: &[&str] = &[
        "en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv",
        "it", "id", "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no",
        "th", "ur", "hr", "bg", "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr",
        "az", "sl", "kn", "et", "mk", "br", "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw",
        "gl", "mr", "pa", "si", "km", "sn", "yo", "so", "af", "oc", "ka", "be", "tg", "sd", "gu",
        "am", "yi", "lo", "uz", "fo", "ht", "ps", "tk", "nn", "mt", "sa", "lb", "my", "bo", "tl",
        "mg", "as", "tt", "haw", "ln", "ha", "ba", "jw", "su", "yue",
    ];
    const TDT_CODES: &[&str] = &[
        "bg", "hr", "cs", "da", "nl", "en", "et", "fi", "fr", "de", "el", "hu", "it", "lv", "lt",
        "mt", "pl", "pt", "ro", "sk", "sl", "es", "sv", "ru", "uk",
    ];
    const NEMOTRON_CODES: &[&str] = &[
        "en", "es", "fr", "it", "pt", "nl", "de", "tr", "ru", "ar", "hi", "ja", "ko", "vi", "uk",
        "pl", "sv", "cs", "nb", "da", "bg", "fi", "hr", "sk", "zh", "hu", "ro", "et", "el", "lt",
        "lv", "mt", "sl", "he", "th", "nn",
    ];

    let mut catalog = BTreeMap::<String, TranscriptionLanguageOption>::new();
    for code in WHISPER_CODES {
        catalog.insert(
            (*code).to_string(),
            TranscriptionLanguageOption {
                code: (*code).to_string(),
                whisper: true,
                parakeet_tdt: false,
                nemotron: false,
            },
        );
    }
    for code in TDT_CODES {
        let entry =
            catalog
                .entry((*code).to_string())
                .or_insert_with(|| TranscriptionLanguageOption {
                    code: (*code).to_string(),
                    whisper: false,
                    parakeet_tdt: false,
                    nemotron: false,
                });
        entry.parakeet_tdt = true;
    }
    for code in NEMOTRON_CODES {
        let entry =
            catalog
                .entry((*code).to_string())
                .or_insert_with(|| TranscriptionLanguageOption {
                    code: (*code).to_string(),
                    whisper: false,
                    parakeet_tdt: false,
                    nemotron: false,
                });
        entry.nemotron = true;
    }

    catalog.into_values().collect()
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

/// The only Whisper model currently certified for the live path.
///
/// Keep this manifest in the domain crate so the application provisioning
/// path and release smoke harness consume the same immutable URL and digest.
/// Other Whisper models remain available for file transcription, but must not
/// silently become an unvalidated live runtime.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WhisperLiveCoreMlEncoderManifest {
    pub directory: String,
    pub archive_filename: String,
    pub url: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WhisperLiveModelManifest {
    pub schema_version: u32,
    pub model: SpeechModel,
    pub filename: String,
    pub url: String,
    pub sha256: String,
    pub coreml_encoder: WhisperLiveCoreMlEncoderManifest,
}

pub fn whisper_live_model_manifest() -> WhisperLiveModelManifest {
    serde_json::from_str(include_str!("whisper_live_model.json"))
        .expect("bundled Whisper live model manifest must be valid")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ParakeetModel {
    RealtimeEou120mV1F16,
    RealtimeEou120mV1Q8,
    #[serde(rename = "nemotron35_asr_streaming_06b_f16")]
    Nemotron35AsrStreaming06bF16,
    #[serde(rename = "nemotron35_asr_streaming_06b_q4")]
    Nemotron35AsrStreaming06bQ4,
    #[serde(rename = "nemotron35_asr_streaming_06b_q8")]
    Nemotron35AsrStreaming06bQ8,
    Tdt06bV3F16,
    Tdt06bV3Q8,
    #[default]
    Tdt06bV3Q4,
}

impl ParakeetModel {
    pub fn gguf_filename(&self) -> &'static str {
        match self {
            Self::RealtimeEou120mV1F16 => "realtime_eou_120m-v1-f16.gguf",
            Self::RealtimeEou120mV1Q8 => "realtime_eou_120m-v1-q8_0.gguf",
            Self::Nemotron35AsrStreaming06bF16 => "nemotron-3.5-asr-streaming-0.6b-f16.gguf",
            Self::Nemotron35AsrStreaming06bQ4 => "nemotron-3.5-asr-streaming-0.6b-q4_k.gguf",
            Self::Nemotron35AsrStreaming06bQ8 => "nemotron-3.5-asr-streaming-0.6b-q8_0.gguf",
            Self::Tdt06bV3F16 => "tdt-0.6b-v3-f16.gguf",
            Self::Tdt06bV3Q8 => "tdt-0.6b-v3-q8_0.gguf",
            Self::Tdt06bV3Q4 => "tdt-0.6b-v3-q4_k.gguf",
        }
    }

    pub fn is_english_realtime_eou(&self) -> bool {
        matches!(self, Self::RealtimeEou120mV1F16 | Self::RealtimeEou120mV1Q8)
    }

    pub fn is_multilingual_streaming(&self) -> bool {
        matches!(
            self,
            Self::Nemotron35AsrStreaming06bF16
                | Self::Nemotron35AsrStreaming06bQ4
                | Self::Nemotron35AsrStreaming06bQ8
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptionEngine {
    #[default]
    #[serde(alias = "whisper_kit")]
    WhisperCpp,
    ParakeetCpp,
}

impl TranscriptionEngine {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WhisperCpp => "whisper_cpp",
            Self::ParakeetCpp => "parakeet_cpp",
        }
    }
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
pub enum AppearanceMode {
    #[default]
    System,
    Light,
    Dark,
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
    pub has_api_key: bool,
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
    pub privacy_policy_version_accepted: Option<String>,
    pub privacy_policy_accepted_at: Option<String>,
    pub appearance_mode: AppearanceMode,
    pub app_language: AppLanguage,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            auto_update_enabled: true,
            auto_update_repo: "pietroMastro92/Sbobino".to_string(),
            privacy_policy_version_accepted: None,
            privacy_policy_accepted_at: None,
            appearance_mode: AppearanceMode::System,
            app_language: AppLanguage::En,
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
    // Speaker diarization controls (whisper.cpp)
    pub tinydiarize: bool,
    pub diarize: bool,
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
    // Legacy controls retained for settings compatibility
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
            no_context: true,
            split_on_word: true,
            tinydiarize: false,
            diarize: false,
            temperature: 0.0,
            temperature_increment_on_fallback: 0.1,
            temperature_fallback_count: 5,
            entropy_threshold: 2.5,
            logprob_threshold: -1.0,
            first_token_logprob_threshold: -1.5,
            no_speech_threshold: 0.72,
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
pub struct SpeakerDiarizationSettings {
    pub enabled: bool,
    pub device: String,
    pub speaker_colors: BTreeMap<String, String>,
}

impl Default for SpeakerDiarizationSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            device: "cpu".to_string(),
            speaker_colors: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TranscriptionSettings {
    pub engine: TranscriptionEngine,
    pub model: SpeechModel,
    pub parakeet_model: ParakeetModel,
    pub language: LanguageCode,
    pub compute_device: TranscriptionComputeDevice,
    /// Compute preference used only by realtime/live transcription.
    /// Kept separate from `compute_device` so file jobs retain their setting
    /// when older settings are migrated. Missing legacy values default to
    /// automatic backend selection.
    #[serde(default)]
    pub live_compute_device: TranscriptionComputeDevice,
    pub whisper_cli_path: String,
    #[serde(alias = "whisper_stream_path")]
    pub whisperkit_cli_path: String,
    pub parakeet_cli_path: String,
    pub ffmpeg_path: String,
    pub models_dir: String,
    pub parakeet_models_dir: String,
    pub enable_ai_post_processing: bool,
    pub speaker_diarization: SpeakerDiarizationSettings,
    pub whisper_options: WhisperOptions,
}

impl Default for TranscriptionSettings {
    fn default() -> Self {
        Self {
            engine: TranscriptionEngine::default(),
            model: SpeechModel::Base,
            parakeet_model: ParakeetModel::default(),
            language: LanguageCode::Auto,
            compute_device: TranscriptionComputeDevice::Auto,
            live_compute_device: TranscriptionComputeDevice::Auto,
            whisper_cli_path: "whisper-cli".to_string(),
            whisperkit_cli_path: "whisper-stream".to_string(),
            parakeet_cli_path: "parakeet-cli".to_string(),
            ffmpeg_path: "ffmpeg".to_string(),
            models_dir: "models".to_string(),
            parakeet_models_dir: "parakeet-models".to_string(),
            enable_ai_post_processing: false,
            speaker_diarization: SpeakerDiarizationSettings::default(),
            whisper_options: WhisperOptions::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutomaticImportPreset {
    #[default]
    General,
    Lecture,
    Meeting,
    Interview,
    VoiceMemo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AutomaticImportPostProcessingSettings {
    pub generate_summary: bool,
    pub generate_faqs: bool,
    pub generate_preset_output: bool,
}

impl Default for AutomaticImportPostProcessingSettings {
    fn default() -> Self {
        Self {
            generate_summary: true,
            generate_faqs: true,
            generate_preset_output: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AutomaticImportSource {
    pub id: String,
    pub label: String,
    pub folder_path: String,
    pub enabled: bool,
    pub preset: AutomaticImportPreset,
    pub model: SpeechModel,
    pub language: LanguageCode,
    pub workspace_id: Option<String>,
    pub recursive: bool,
    pub enable_ai_post_processing: bool,
    pub post_processing: AutomaticImportPostProcessingSettings,
}

impl Default for AutomaticImportSource {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            folder_path: String::new(),
            enabled: true,
            preset: AutomaticImportPreset::General,
            model: SpeechModel::default(),
            language: LanguageCode::default(),
            workspace_id: None,
            recursive: true,
            enable_ai_post_processing: false,
            post_processing: AutomaticImportPostProcessingSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AutomaticImportSettings {
    pub enabled: bool,
    pub run_scan_on_app_start: bool,
    pub scan_interval_minutes: u32,
    pub allowed_extensions: Vec<String>,
    pub watched_sources: Vec<AutomaticImportSource>,
    pub excluded_folders: Vec<String>,
    pub source_statuses: Vec<AutomaticImportSourceStatus>,
    pub recent_activity: Vec<AutomaticImportActivityEntry>,
    pub quarantined_items: Vec<AutomaticImportQuarantineItem>,
}

impl Default for AutomaticImportSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            run_scan_on_app_start: true,
            scan_interval_minutes: 15,
            allowed_extensions: default_automatic_import_extensions(),
            watched_sources: Vec::new(),
            excluded_folders: Vec::new(),
            source_statuses: Vec::new(),
            recent_activity: Vec::new(),
            quarantined_items: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutomaticImportSourceHealth {
    #[default]
    Idle,
    Healthy,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AutomaticImportSourceStatus {
    pub source_id: String,
    pub source_label: String,
    pub health: AutomaticImportSourceHealth,
    pub last_scan_at: Option<String>,
    pub last_success_at: Option<String>,
    pub last_failure_at: Option<String>,
    pub last_error: Option<String>,
    pub last_scan_reason: Option<String>,
    pub last_trigger: Option<String>,
    pub last_scanned_files: u32,
    pub last_queued_jobs: u32,
    pub last_skipped_existing: u32,
    pub watcher_mode: String,
}

impl Default for AutomaticImportSourceStatus {
    fn default() -> Self {
        Self {
            source_id: String::new(),
            source_label: String::new(),
            health: AutomaticImportSourceHealth::Idle,
            last_scan_at: None,
            last_success_at: None,
            last_failure_at: None,
            last_error: None,
            last_scan_reason: None,
            last_trigger: None,
            last_scanned_files: 0,
            last_queued_jobs: 0,
            last_skipped_existing: 0,
            watcher_mode: "periodic_scan".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutomaticImportActivityLevel {
    #[default]
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AutomaticImportActivityEntry {
    pub id: String,
    pub timestamp: String,
    pub source_id: Option<String>,
    pub level: AutomaticImportActivityLevel,
    pub message: String,
}

impl Default for AutomaticImportActivityEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            timestamp: String::new(),
            source_id: None,
            level: AutomaticImportActivityLevel::Info,
            message: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct AutomaticImportQuarantineItem {
    pub id: String,
    pub source_id: Option<String>,
    pub source_label: Option<String>,
    pub file_path: String,
    pub fingerprint_key: Option<String>,
    pub reason: String,
    pub first_detected_at: String,
    pub last_detected_at: String,
    pub retry_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorkspaceConfig {
    pub id: String,
    pub label: String,
    pub color: String,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            color: "#4F7CFF".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OrganizationSettings {
    pub workspaces: Vec<WorkspaceConfig>,
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
    pub has_api_key: bool,
    pub model: String,
}

impl Default for GeminiProviderSettings {
    fn default() -> Self {
        Self {
            api_key: None,
            has_api_key: false,
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
    pub emotion_prompt_id: String,
}

impl Default for PromptBindings {
    fn default() -> Self {
        Self {
            optimize_prompt_id: "builtin_improve_grammar".to_string(),
            summary_prompt_id: "builtin_bullet_points".to_string(),
            faq_prompt_id: "builtin_generate_faq".to_string(),
            emotion_prompt_id: "builtin_identify_emotions".to_string(),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PersonalizationSettings {
    pub enabled: bool,
    pub auto_apply_safe_corrections: bool,
}

impl Default for PersonalizationSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_apply_safe_corrections: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromptTask {
    Optimize,
    Summary,
    Faq,
    EmotionAnalysis,
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
    pub gemini_api_key_present: bool,
    pub whisper_cli_path: String,
    #[serde(alias = "whisper_stream_path")]
    pub whisperkit_cli_path: String,
    pub ffmpeg_path: String,
    pub models_dir: String,
    pub auto_update_enabled: bool,
    pub auto_update_repo: String,

    // New structured settings for Whisper-style settings workspace.
    pub general: GeneralSettings,
    pub transcription: TranscriptionSettings,
    pub automation: AutomaticImportSettings,
    pub organization: OrganizationSettings,
    pub ai: AiSettings,
    pub prompts: PromptSettings,
    pub personalization: PersonalizationSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        let general = GeneralSettings::default();
        let transcription = TranscriptionSettings::default();
        let automation = AutomaticImportSettings::default();
        let organization = OrganizationSettings::default();
        let ai = AiSettings::default();
        let prompts = PromptSettings::default();
        let personalization = PersonalizationSettings::default();

        Self {
            transcription_engine: transcription.engine.clone(),
            model: transcription.model.clone(),
            language: transcription.language.clone(),
            ai_post_processing: transcription.enable_ai_post_processing,
            gemini_model: ai.providers.gemini.model.clone(),
            gemini_api_key: ai.providers.gemini.api_key.clone(),
            gemini_api_key_present: ai.providers.gemini.api_key.is_some(),
            whisper_cli_path: transcription.whisper_cli_path.clone(),
            whisperkit_cli_path: transcription.whisperkit_cli_path.clone(),
            ffmpeg_path: transcription.ffmpeg_path.clone(),
            models_dir: transcription.models_dir.clone(),
            auto_update_enabled: general.auto_update_enabled,
            auto_update_repo: general.auto_update_repo.clone(),
            general,
            transcription,
            automation,
            organization,
            ai,
            prompts,
            personalization,
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
        self.automation.scan_interval_minutes =
            self.automation.scan_interval_minutes.clamp(1, 24 * 60);
        if self.automation.allowed_extensions.is_empty() {
            self.automation.allowed_extensions = default_automatic_import_extensions();
        } else {
            self.automation.allowed_extensions = self
                .automation
                .allowed_extensions
                .iter()
                .map(|value| value.trim().trim_start_matches('.').to_lowercase())
                .filter(|value| !value.is_empty())
                .collect();
            if self.automation.allowed_extensions.is_empty() {
                self.automation.allowed_extensions = default_automatic_import_extensions();
            }
        }

        self.ai.providers.gemini.model = self.gemini_model.clone();
        self.ai.providers.gemini.api_key = self.gemini_api_key.clone();
        self.ai.providers.gemini.has_api_key =
            self.gemini_api_key_present || self.gemini_api_key.is_some();
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

        self.refresh_secret_presence_flags();
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
        self.gemini_api_key_present =
            self.ai.providers.gemini.has_api_key || self.gemini_api_key.is_some();
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

        self.refresh_secret_presence_flags();
        self.ensure_prompt_integrity();
    }

    pub fn refresh_secret_presence_flags(&mut self) {
        self.ai.providers.gemini.has_api_key =
            self.ai.providers.gemini.has_api_key || self.ai.providers.gemini.api_key.is_some();
        self.gemini_api_key_present =
            self.gemini_api_key_present || self.ai.providers.gemini.has_api_key;
        for service in &mut self.ai.remote_services {
            service.has_api_key = service.has_api_key || service.api_key.is_some();
        }
    }

    pub fn redacted_clone(&self) -> Self {
        let mut redacted = self.clone();
        redacted.refresh_secret_presence_flags();
        redacted.gemini_api_key = None;
        redacted.ai.providers.gemini.api_key = None;
        for service in &mut redacted.ai.remote_services {
            service.api_key = None;
        }
        redacted
    }

    pub fn ensure_prompt_integrity(&mut self) {
        let default_templates = default_prompt_templates();
        if self.prompts.templates.is_empty() {
            self.prompts.templates = default_templates.clone();
        } else {
            for default_template in &default_templates {
                if !default_template.builtin {
                    continue;
                }

                if let Some(existing) = self
                    .prompts
                    .templates
                    .iter_mut()
                    .find(|template| template.id == default_template.id && template.builtin)
                {
                    existing.name = default_template.name.clone();
                    existing.icon = default_template.icon.clone();
                    existing.category = default_template.category.clone();
                    existing.body = default_template.body.clone();
                } else {
                    self.prompts.templates.push(default_template.clone());
                }
            }
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

        let has_emotion = self
            .prompts
            .templates
            .iter()
            .any(|template| template.id == self.prompts.bindings.emotion_prompt_id);
        if !has_emotion {
            self.prompts.bindings.emotion_prompt_id = PromptBindings::default().emotion_prompt_id;
        }
    }

    pub fn prompt_for_task(&self, task: PromptTask) -> Option<String> {
        let template_id = match task {
            PromptTask::Optimize => &self.prompts.bindings.optimize_prompt_id,
            PromptTask::Summary => &self.prompts.bindings.summary_prompt_id,
            PromptTask::Faq => &self.prompts.bindings.faq_prompt_id,
            PromptTask::EmotionAnalysis => &self.prompts.bindings.emotion_prompt_id,
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
            name: "Detailed Brief".to_string(),
            icon: "notebook".to_string(),
            category: PromptCategory::Summary,
            body: "Create a detailed, sectioned summary that reads like a high-quality briefing note. Preserve all major topics, technical details, examples, numbers, decisions, risks, and next steps, and explain how the ideas connect. Prefer polished prose sections over terse bullets unless bullets materially improve clarity."
                .to_string(),
            builtin: true,
            updated_at: "".to_string(),
        },
        PromptTemplate {
            id: "builtin_improve_grammar".to_string(),
            name: "Improve Transcript".to_string(),
            icon: "abc".to_string(),
            category: PromptCategory::Cleanup,
            body:
                "Preserve the original wording, structure, and order as much as possible. Improve punctuation, capitalization, spacing, and paragraph breaks, remove obvious accidental repetitions, and correct isolated ASR/transcription mistakes when the intended term is highly likely from context, especially for technical words and domain-specific jargon. If unsure, keep the original wording. Do not paraphrase whole sentences or invent new facts."
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

fn default_automatic_import_extensions() -> Vec<String> {
    vec![
        "wav", "m4a", "mp3", "ogg", "opus", "webm", "flac", "aac", "aiff", "aif", "m4b",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

#[cfg(test)]
mod language_tests {
    use super::*;

    #[test]
    fn language_codes_are_normalized_and_round_trip() {
        assert_eq!(LanguageCode::from_code("it-IT").as_code(), "it");
        assert_eq!(LanguageCode::from_code("EN_us").as_code(), "en");
        assert!(LanguageCode::try_from_code("not-a-code").is_err());
        assert_eq!(LanguageCode::from_code("auto"), LanguageCode::Auto);
    }

    #[test]
    fn catalog_is_the_union_of_engine_capabilities() {
        let catalog = transcription_language_catalog();
        assert!(catalog.iter().any(|entry| entry.code == "it"
            && entry.whisper
            && entry.parakeet_tdt
            && entry.nemotron));
        assert!(catalog
            .iter()
            .any(|entry| entry.code == "yue" && entry.whisper));
        assert!(catalog.iter().all(|entry| !entry.code.is_empty()));
    }

    #[test]
    fn adaptive_policy_defaults_to_auto_detection() {
        let policy = TranscriptionLanguagePolicy::default();
        assert!(policy.adaptive_detection);
        assert_eq!(policy.preferred_language, LanguageCode::Auto);
    }

    #[test]
    fn transcription_compute_device_defaults_and_round_trips() {
        let defaults = TranscriptionSettings::default();
        assert_eq!(defaults.compute_device, TranscriptionComputeDevice::Auto);
        assert_eq!(
            defaults.live_compute_device,
            TranscriptionComputeDevice::Auto
        );

        let json = serde_json::to_string(&TranscriptionComputeDevice::Cpu).unwrap();
        assert_eq!(json, "\"cpu\"");
        assert_eq!(
            serde_json::from_str::<TranscriptionComputeDevice>("\"gpu\"").unwrap(),
            TranscriptionComputeDevice::Gpu
        );
    }

    #[test]
    fn legacy_transcription_settings_migrate_live_compute_to_auto_without_changing_file_device() {
        let settings: TranscriptionSettings = serde_json::from_str(
            r#"{
                "engine":"whisper_cpp",
                "model":"base",
                "parakeet_model":"tdt06b_v3_q4",
                "language":"auto",
                "compute_device":"cpu",
                "whisper_cli_path":"whisper-cli",
                "whisperkit_cli_path":"whisper-stream",
                "parakeet_cli_path":"parakeet-cli",
                "ffmpeg_path":"ffmpeg",
                "models_dir":"models",
                "parakeet_models_dir":"parakeet-models",
                "enable_ai_post_processing":false,
                "speaker_diarization":{"enabled":false,"device":"cpu","speaker_colors":{}},
                "whisper_options":{}
            }"#,
        )
        .expect("legacy settings should deserialize");
        assert_eq!(settings.compute_device, TranscriptionComputeDevice::Cpu);
        assert_eq!(
            settings.live_compute_device,
            TranscriptionComputeDevice::Auto
        );
    }

    #[test]
    fn whisper_live_manifest_is_pinned_to_the_certified_realtime_model() {
        let manifest = whisper_live_model_manifest();

        assert_eq!(manifest.schema_version, 1);
        assert_eq!(manifest.model, SpeechModel::Tiny);
        assert_eq!(manifest.filename, "ggml-tiny-q8_0.bin");
        assert!(manifest
            .url
            .contains("/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/"));
        assert!(!manifest.url.contains("/resolve/main/"));
        assert_eq!(manifest.sha256.len(), 64);
        assert_eq!(
            manifest.sha256,
            "c2085835d3f50733e2ff6e4b41ae8a2b8d8110461e18821b09a15c40c42d1cca"
        );
        assert!(manifest
            .sha256
            .chars()
            .all(|character| character.is_ascii_hexdigit()));
        assert_eq!(
            manifest.coreml_encoder.directory,
            "ggml-tiny-encoder.mlmodelc"
        );
        assert_eq!(
            manifest.coreml_encoder.archive_filename,
            "ggml-tiny-encoder.mlmodelc.zip"
        );
        assert!(manifest
            .coreml_encoder
            .url
            .contains("/resolve/c521a4b02f422512d734391fdf08bb08c0862f68/"));
        assert_eq!(
            manifest.coreml_encoder.sha256,
            "c88cbd2648e1f5415092bcf5256add463a0f19943e6938f46e8d4ffdebd47739"
        );
    }

    #[test]
    fn personalization_defaults_are_local_and_user_controlled() {
        let settings = AppSettings::default().personalization;
        assert!(settings.enabled);
        assert!(!settings.auto_apply_safe_corrections);
    }
}
