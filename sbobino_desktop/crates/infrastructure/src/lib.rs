pub mod adapters;
pub mod repositories;
pub mod secure_storage;

use chrono::Utc;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};
use tracing::{info, warn};

use sbobino_application::{
    ArtifactService, SettingsService, TranscriptEnhancer, TranscriptionService,
};
use sbobino_domain::{
    AiProvider, AppSettings, PromptTask, RemoteServiceConfig, RemoteServiceKind,
    TranscriptionEngine,
};

use adapters::{
    ffmpeg::FfmpegAdapter,
    foundation_apple::FoundationAppleEnhancer,
    gemini::GeminiEnhancer,
    noop_enhancer::NoopEnhancer,
    openai_compatible::{AuthStyle, OpenAiCompatibleEnhancer},
    pyannote::{
        embedded_helper_script, PyannoteSpeakerDiarizationEngine, EMBEDDED_HELPER_FILENAME,
    },
    whisper_cpp::WhisperCppEngine,
    whisper_stream::WhisperStreamEngine,
};
use repositories::{
    fs_settings_repository::FsSettingsRepository,
    sqlite_artifact_repository::SqliteArtifactRepository,
};

#[derive(Clone)]
pub struct RuntimeTranscriptionFactory {
    settings_repo: Arc<FsSettingsRepository>,
    artifacts_repo: Arc<SqliteArtifactRepository>,
    data_dir: PathBuf,
    bundle_resources_dir: Option<PathBuf>,
    allow_dev_resource_overrides: bool,
}

const REQUIRED_MODEL_FILES: [&str; 5] = [
    "ggml-tiny.bin",
    "ggml-base.bin",
    "ggml-small.bin",
    "ggml-medium.bin",
    "ggml-large-v3-turbo-q8_0.bin",
];

const REQUIRED_COREML_ENCODERS: [(&str, &str); 5] = [
    ("ggml-tiny.bin", "ggml-tiny-encoder.mlmodelc"),
    ("ggml-base.bin", "ggml-base-encoder.mlmodelc"),
    ("ggml-small.bin", "ggml-small-encoder.mlmodelc"),
    ("ggml-medium.bin", "ggml-medium-encoder.mlmodelc"),
    (
        "ggml-large-v3-turbo-q8_0.bin",
        "ggml-large-v3-turbo-encoder.mlmodelc",
    ),
];

pub const PYANNOTE_MANIFEST_FILENAME: &str = "manifest.json";
pub const PYANNOTE_STATUS_FILENAME: &str = "status.json";
const PYANNOTE_BUNDLED_OVERRIDE_SOURCE: &str = "bundled_override";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ManagedPyannoteManifest {
    pub source: String,
    pub app_version: String,
    pub runtime_asset: String,
    pub runtime_sha256: String,
    pub model_asset: String,
    pub model_sha256: String,
    pub runtime_arch: String,
    pub installed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ManagedPyannoteStatus {
    pub reason_code: String,
    pub message: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PyannoteRuntimeHealth {
    pub enabled: bool,
    pub ready: bool,
    pub runtime_installed: bool,
    pub model_installed: bool,
    pub arch: String,
    pub device: String,
    pub source: String,
    pub reason_code: String,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct RuntimeHealth {
    pub host_os: String,
    pub host_arch: String,
    pub is_apple_silicon: bool,
    pub preferred_engine: TranscriptionEngine,
    pub configured_engine: TranscriptionEngine,
    pub ffmpeg_path: String,
    pub ffmpeg_resolved: String,
    pub ffmpeg_available: bool,
    pub whisper_cli_path: String,
    pub whisper_cli_resolved: String,
    pub whisper_cli_available: bool,
    pub whisper_stream_path: String,
    pub whisper_stream_resolved: String,
    pub whisper_stream_available: bool,
    pub models_dir_configured: String,
    pub models_dir_resolved: String,
    pub model_filename: String,
    pub model_present: bool,
    pub coreml_encoder_present: bool,
    pub missing_models: Vec<String>,
    pub missing_encoders: Vec<String>,
    pub pyannote: PyannoteRuntimeHealth,
    pub setup_complete: bool,
}

#[derive(Debug, Clone)]
struct BinaryResolution {
    resolved_path: String,
}

#[derive(Clone)]
pub struct AiEnhancerCandidate {
    pub key: String,
    pub label: String,
    pub fallback: bool,
    pub enhancer: Arc<dyn TranscriptEnhancer>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiCapabilityStatus {
    pub available: bool,
    pub fallback_available: bool,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone)]
struct EnhancerOverrides {
    model_override: Option<String>,
    optimize_prompt_override: Option<String>,
    summary_prompt_override: Option<String>,
}

#[derive(Debug, Clone)]
enum EnhancerSource {
    FoundationApple,
    LegacyGemini,
    RemoteService(String),
}

#[derive(Debug, Clone)]
struct EnhancerCandidateSpec {
    key: String,
    label: String,
    fallback: bool,
    source: EnhancerSource,
}

impl RuntimeTranscriptionFactory {
    pub fn new(data_dir: &Path, bundle_resources_dir: Option<PathBuf>) -> Result<Self, String> {
        Self::new_with_options(data_dir, bundle_resources_dir, true)
    }

    fn new_with_options(
        data_dir: &Path,
        bundle_resources_dir: Option<PathBuf>,
        allow_dev_resource_overrides: bool,
    ) -> Result<Self, String> {
        std::fs::create_dir_all(data_dir)
            .map_err(|e| format!("failed to create app data dir {}: {e}", data_dir.display()))?;

        let settings_path = data_dir.join("settings.json");
        let artifacts_db = data_dir.join("artifacts.db");

        let settings_repo = Arc::new(FsSettingsRepository::new(settings_path));
        let artifacts_repo = Arc::new(
            SqliteArtifactRepository::new(artifacts_db)
                .map_err(|e| format!("failed to initialize artifacts repository: {e}"))?,
        );

        Ok(Self {
            settings_repo,
            artifacts_repo,
            data_dir: data_dir.to_path_buf(),
            bundle_resources_dir,
            allow_dev_resource_overrides,
        })
    }

    #[cfg(test)]
    fn new_for_tests(
        data_dir: &Path,
        bundle_resources_dir: Option<PathBuf>,
    ) -> Result<Self, String> {
        Self::new_with_options(data_dir, bundle_resources_dir, false)
    }

    pub fn artifacts_db_path(&self) -> PathBuf {
        self.data_dir.join("artifacts.db")
    }

    pub fn vault_dir(&self) -> PathBuf {
        self.data_dir.join("vault")
    }

    pub fn build_service(&self) -> Result<Arc<TranscriptionService>, String> {
        let mut settings = self.load_settings()?;
        let mut platform_settings_changed = false;

        // Foundation Models are only supported on Apple Silicon.
        if !is_apple_silicon_host()
            && (settings.ai.providers.foundation_apple.enabled
                || settings.ai.active_provider == AiProvider::FoundationApple)
        {
            settings.ai.providers.foundation_apple.enabled = false;
            if settings.ai.active_provider == AiProvider::FoundationApple {
                settings.ai.active_provider = AiProvider::None;
            }
            platform_settings_changed = true;
        }

        let ffmpeg_path = self.resolve_binary_path(&settings.transcription.ffmpeg_path, "ffmpeg");
        let whisper_cli_reference =
            sanitize_whisper_cli_reference(&settings.transcription.whisper_cli_path);
        if whisper_cli_reference != settings.transcription.whisper_cli_path {
            settings.transcription.whisper_cli_path = whisper_cli_reference.clone();
            settings.whisper_cli_path = whisper_cli_reference.clone();
            platform_settings_changed = true;
        }
        let whisper_stream_reference = sanitize_whisper_stream_reference(
            &settings.transcription.whisperkit_cli_path,
            &whisper_cli_reference,
        );
        if whisper_stream_reference != settings.transcription.whisperkit_cli_path {
            settings.transcription.whisperkit_cli_path = whisper_stream_reference.clone();
            settings.whisperkit_cli_path = whisper_stream_reference;
            platform_settings_changed = true;
        }
        let whisper_cli_path = self.resolve_binary_path(&whisper_cli_reference, "whisper-cli");
        let models_dir = self.resolve_models_dir(&settings.transcription.models_dir);
        let whisper_cli_runnable = self.binary_path_is_runnable(&whisper_cli_path);

        if !whisper_cli_runnable {
            return Err(format!(
                "Whisper.cpp CLI is not runnable at '{}'. Configure Whisper CLI path in Settings > Local Models.",
                whisper_cli_path
            ));
        }

        let effective_engine = TranscriptionEngine::WhisperCpp;

        if settings.transcription.engine != effective_engine {
            warn!(
                "Adjusting transcription engine for current runtime: configured={:?}, effective={:?}",
                settings.transcription.engine, effective_engine
            );
            settings.transcription.engine = effective_engine.clone();
            settings.transcription_engine = effective_engine.clone();
            platform_settings_changed = true;
        }

        if platform_settings_changed {
            settings.sync_sections_from_legacy();
            settings.sync_legacy_from_sections();
            self.settings_repo
                .save_sync(&settings)
                .map_err(|e| format!("failed to persist platform-specific settings: {e}"))?;
        }

        let transcoder = Arc::new(FfmpegAdapter::new(ffmpeg_path));
        let speech_engine: Arc<dyn sbobino_application::SpeechToTextEngine> =
            Arc::new(WhisperCppEngine::new(whisper_cli_path, models_dir));
        let speaker_diarizer = self.build_speaker_diarizer(&settings)?;

        let enhancer_candidates = self
            .build_enhancer_candidates()
            .map_err(|error| format!("failed to build AI enhancer chain: {error}"))?;
        let enhancer = enhancer_candidates
            .first()
            .map(|candidate| candidate.enhancer.clone())
            .unwrap_or_else(|| Arc::new(NoopEnhancer));
        let fallback_enhancers = enhancer_candidates
            .iter()
            .skip(1)
            .map(|candidate| candidate.enhancer.clone())
            .collect::<Vec<_>>();

        let mut service = TranscriptionService::new(
            transcoder,
            speech_engine,
            enhancer,
            self.artifacts_repo.clone(),
        )
        .with_fallback_enhancers(fallback_enhancers);
        if let Some(diarizer) = speaker_diarizer {
            service = service.with_speaker_diarizer(diarizer);
        }

        Ok(Arc::new(service))
    }

    pub fn settings_service(&self) -> Arc<SettingsService> {
        Arc::new(SettingsService::new(self.settings_repo.clone()))
    }

    pub fn artifact_service(&self) -> Arc<ArtifactService> {
        Arc::new(ArtifactService::new(self.artifacts_repo.clone()))
    }

    pub fn build_gemini_enhancer(&self) -> Result<Option<GeminiEnhancer>, String> {
        self.build_gemini_enhancer_with_overrides(None, None, None)
    }

    pub fn build_gemini_enhancer_with_overrides(
        &self,
        model_override: Option<String>,
        optimize_prompt_override: Option<String>,
        summary_prompt_override: Option<String>,
    ) -> Result<Option<GeminiEnhancer>, String> {
        let settings = self.load_settings()?;

        let Some(api_key) = settings.ai.providers.gemini.api_key.clone() else {
            return Ok(None);
        };

        let model = model_override
            .and_then(|value| {
                let trimmed = value.trim().to_string();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            })
            .unwrap_or_else(|| settings.ai.providers.gemini.model.clone());

        Ok(Some(GeminiEnhancer::new(
            api_key,
            model,
            optimize_prompt_override.or_else(|| settings.prompt_for_task(PromptTask::Optimize)),
            summary_prompt_override.or_else(|| settings.prompt_for_task(PromptTask::Summary)),
        )))
    }

    pub fn build_foundation_enhancer(&self) -> Result<Option<FoundationAppleEnhancer>, String> {
        self.build_foundation_enhancer_with_overrides(None, None)
    }

    pub fn build_foundation_enhancer_with_overrides(
        &self,
        optimize_prompt_override: Option<String>,
        summary_prompt_override: Option<String>,
    ) -> Result<Option<FoundationAppleEnhancer>, String> {
        if !is_apple_silicon_host() {
            return Ok(None);
        }

        let settings = self.load_settings()?;
        self.build_foundation_enhancer_from_settings(
            &settings,
            &EnhancerOverrides {
                model_override: None,
                optimize_prompt_override,
                summary_prompt_override,
            },
        )
    }

    pub fn build_active_enhancer(&self) -> Result<Option<Arc<dyn TranscriptEnhancer>>, String> {
        Ok(self
            .build_enhancer_candidates()?
            .into_iter()
            .next()
            .map(|candidate| candidate.enhancer))
    }

    pub fn build_enhancer_candidates(&self) -> Result<Vec<AiEnhancerCandidate>, String> {
        self.build_enhancer_candidates_with_overrides(None, None, None)
    }

    pub fn build_enhancer_candidates_with_overrides(
        &self,
        model_override: Option<String>,
        optimize_prompt_override: Option<String>,
        summary_prompt_override: Option<String>,
    ) -> Result<Vec<AiEnhancerCandidate>, String> {
        let settings = self.load_settings()?;
        let overrides = EnhancerOverrides {
            model_override,
            optimize_prompt_override,
            summary_prompt_override,
        };

        let mut seen_keys = HashSet::new();
        let mut candidates = Vec::new();

        for spec in self.ordered_enhancer_candidate_specs(&settings) {
            if !seen_keys.insert(spec.key.clone()) {
                continue;
            }

            if let Some(candidate) =
                self.build_enhancer_candidate_from_spec(&settings, &spec, &overrides)?
            {
                candidates.push(candidate);
            }
        }

        Ok(candidates)
    }

    pub fn ai_capability_status(&self) -> Result<AiCapabilityStatus, String> {
        let candidates = self.build_enhancer_candidates()?;
        if !candidates.is_empty() {
            return Ok(AiCapabilityStatus {
                available: true,
                fallback_available: candidates.len() > 1,
                unavailable_reason: None,
            });
        }

        let unavailable_reason = if is_apple_silicon_host() {
            "No usable AI provider is available. Configure an external AI service, enable Apple Foundation, or configure a local model in Settings > AI Services.".to_string()
        } else {
            "No usable AI provider is available. Configure an external AI service or a local model in Settings > AI Services.".to_string()
        };

        Ok(AiCapabilityStatus {
            available: false,
            fallback_available: false,
            unavailable_reason: Some(unavailable_reason),
        })
    }

    fn ordered_enhancer_candidate_specs(
        &self,
        settings: &AppSettings,
    ) -> Vec<EnhancerCandidateSpec> {
        let mut specs = Vec::new();

        if settings.ai.active_provider == AiProvider::FoundationApple {
            specs.push(EnhancerCandidateSpec {
                key: "foundation_apple".to_string(),
                label: "Apple Foundation".to_string(),
                fallback: false,
                source: EnhancerSource::FoundationApple,
            });
        } else if let Some(active_id) = settings.ai.active_remote_service_id.as_ref() {
            specs.push(self.remote_service_candidate_spec(settings, active_id, false));
        } else if settings.ai.active_provider == AiProvider::Gemini {
            if let Some(google_service) = settings
                .ai
                .remote_services
                .iter()
                .find(|service| service.kind == RemoteServiceKind::Google && service.enabled)
            {
                specs.push(self.remote_service_candidate_spec(settings, &google_service.id, false));
            } else {
                specs.push(EnhancerCandidateSpec {
                    key: "legacy_gemini".to_string(),
                    label: "Gemini".to_string(),
                    fallback: false,
                    source: EnhancerSource::LegacyGemini,
                });
            }
        }

        if is_apple_silicon_host() && settings.ai.providers.foundation_apple.enabled {
            specs.push(EnhancerCandidateSpec {
                key: "foundation_apple".to_string(),
                label: "Apple Foundation".to_string(),
                fallback: true,
                source: EnhancerSource::FoundationApple,
            });
        }

        for service in settings
            .ai
            .remote_services
            .iter()
            .filter(|service| self.is_local_fallback_service(service))
        {
            specs.push(self.remote_service_candidate_spec(settings, &service.id, true));
        }

        specs
    }

    fn remote_service_candidate_spec(
        &self,
        settings: &AppSettings,
        service_id: &str,
        fallback: bool,
    ) -> EnhancerCandidateSpec {
        let label = settings
            .ai
            .remote_services
            .iter()
            .find(|service| service.id == service_id)
            .map(|service| self.service_display_label(settings, service))
            .unwrap_or_else(|| format!("AI service {service_id}"));

        EnhancerCandidateSpec {
            key: format!("remote:{service_id}"),
            label,
            fallback,
            source: EnhancerSource::RemoteService(service_id.to_string()),
        }
    }

    fn build_enhancer_candidate_from_spec(
        &self,
        settings: &AppSettings,
        spec: &EnhancerCandidateSpec,
        overrides: &EnhancerOverrides,
    ) -> Result<Option<AiEnhancerCandidate>, String> {
        let enhancer: Option<Arc<dyn TranscriptEnhancer>> = match &spec.source {
            EnhancerSource::FoundationApple => self
                .build_foundation_enhancer_from_settings(settings, overrides)?
                .map(|value| Arc::new(value) as Arc<dyn TranscriptEnhancer>),
            EnhancerSource::LegacyGemini => self
                .build_gemini_enhancer_from_settings(settings, overrides)?
                .map(|value| Arc::new(value) as Arc<dyn TranscriptEnhancer>),
            EnhancerSource::RemoteService(service_id) => self
                .build_remote_service_enhancer_from_settings(settings, service_id, overrides)?
                .map(Arc::from),
        };

        Ok(enhancer.map(|enhancer| AiEnhancerCandidate {
            key: spec.key.clone(),
            label: spec.label.clone(),
            fallback: spec.fallback,
            enhancer,
        }))
    }

    fn build_gemini_enhancer_from_settings(
        &self,
        settings: &AppSettings,
        overrides: &EnhancerOverrides,
    ) -> Result<Option<GeminiEnhancer>, String> {
        let Some(api_key) = settings.ai.providers.gemini.api_key.clone() else {
            return Ok(None);
        };

        let model = overrides
            .model_override
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| settings.ai.providers.gemini.model.clone());

        Ok(Some(GeminiEnhancer::new(
            api_key,
            model,
            overrides
                .optimize_prompt_override
                .clone()
                .or_else(|| settings.prompt_for_task(PromptTask::Optimize)),
            overrides
                .summary_prompt_override
                .clone()
                .or_else(|| settings.prompt_for_task(PromptTask::Summary)),
        )))
    }

    fn build_foundation_enhancer_from_settings(
        &self,
        settings: &AppSettings,
        overrides: &EnhancerOverrides,
    ) -> Result<Option<FoundationAppleEnhancer>, String> {
        if !is_apple_silicon_host() || !settings.ai.providers.foundation_apple.enabled {
            return Ok(None);
        }

        Ok(Some(FoundationAppleEnhancer::new(
            overrides
                .optimize_prompt_override
                .clone()
                .or_else(|| settings.prompt_for_task(PromptTask::Optimize)),
            overrides
                .summary_prompt_override
                .clone()
                .or_else(|| settings.prompt_for_task(PromptTask::Summary)),
        )))
    }

    fn build_remote_service_enhancer_from_settings(
        &self,
        settings: &AppSettings,
        service_id: &str,
        overrides: &EnhancerOverrides,
    ) -> Result<Option<Box<dyn TranscriptEnhancer>>, String> {
        let Some(service) = settings
            .ai
            .remote_services
            .iter()
            .find(|entry| entry.id == service_id && entry.enabled)
        else {
            return Ok(None);
        };

        if service.kind == RemoteServiceKind::Google {
            let enhancer = self.build_gemini_for_service(settings, service, overrides)?;
            return Ok(enhancer.map(|value| Box::new(value) as Box<dyn TranscriptEnhancer>));
        }

        let enhancer = self.build_openai_compatible_for_service(settings, service, overrides)?;
        Ok(enhancer.map(|value| Box::new(value) as Box<dyn TranscriptEnhancer>))
    }

    fn build_gemini_for_service(
        &self,
        settings: &AppSettings,
        service: &RemoteServiceConfig,
        overrides: &EnhancerOverrides,
    ) -> Result<Option<GeminiEnhancer>, String> {
        let api_key = service
            .api_key
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                settings
                    .ai
                    .providers
                    .gemini
                    .api_key
                    .as_ref()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            });
        let Some(api_key) = api_key else {
            return Err("Google service requires a Gemini API key".to_string());
        };

        let model = overrides
            .model_override
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                service
                    .model
                    .as_ref()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            })
            .unwrap_or_else(|| settings.ai.providers.gemini.model.clone());

        Ok(Some(GeminiEnhancer::new(
            api_key,
            model,
            overrides
                .optimize_prompt_override
                .clone()
                .or_else(|| settings.prompt_for_task(PromptTask::Optimize)),
            overrides
                .summary_prompt_override
                .clone()
                .or_else(|| settings.prompt_for_task(PromptTask::Summary)),
        )))
    }

    fn build_openai_compatible_for_service(
        &self,
        settings: &AppSettings,
        service: &RemoteServiceConfig,
        overrides: &EnhancerOverrides,
    ) -> Result<Option<OpenAiCompatibleEnhancer>, String> {
        let Some(base_url) = service
            .base_url
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                default_base_url_for_service_kind(&service.kind).map(|value| value.to_string())
            })
        else {
            return Err(format!("{} service requires a base URL", service.label));
        };

        let Some(model) = overrides
            .model_override
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                service
                    .model
                    .as_ref()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            })
            .or_else(|| {
                default_model_for_service_kind(&service.kind).map(|value| value.to_string())
            })
        else {
            return Err(format!("{} service requires a model name", service.label));
        };

        let auth_style = match &service.kind {
            RemoteServiceKind::LmStudio | RemoteServiceKind::Ollama | RemoteServiceKind::Custom => {
                if service
                    .api_key
                    .as_ref()
                    .map(|value| value.trim().is_empty())
                    .unwrap_or(true)
                {
                    AuthStyle::None
                } else {
                    AuthStyle::Bearer
                }
            }
            RemoteServiceKind::Azure => AuthStyle::ApiKeyHeader,
            RemoteServiceKind::OpenAi
            | RemoteServiceKind::OpenRouter
            | RemoteServiceKind::Xai
            | RemoteServiceKind::Anthropic
            | RemoteServiceKind::HuggingFace => AuthStyle::Bearer,
            RemoteServiceKind::Google => {
                return Ok(None);
            }
        };

        let enhancer = OpenAiCompatibleEnhancer::new(
            base_url,
            model,
            service.api_key.clone(),
            auth_style,
            overrides
                .optimize_prompt_override
                .clone()
                .or_else(|| settings.prompt_for_task(PromptTask::Optimize)),
            overrides
                .summary_prompt_override
                .clone()
                .or_else(|| settings.prompt_for_task(PromptTask::Summary)),
        )
        .map_err(|error| format!("{error}"))?;
        Ok(Some(enhancer))
    }

    fn is_local_fallback_service(&self, service: &RemoteServiceConfig) -> bool {
        if !service.enabled {
            return false;
        }

        if !matches!(
            service.kind,
            RemoteServiceKind::LmStudio | RemoteServiceKind::Ollama | RemoteServiceKind::Custom
        ) {
            return false;
        }

        let has_model = service
            .model
            .as_ref()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
        if !has_model {
            return false;
        }

        let base_url = service
            .base_url
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                default_base_url_for_service_kind(&service.kind).map(|value| value.to_string())
            });

        base_url
            .as_deref()
            .map(is_loopback_base_url)
            .unwrap_or(false)
    }

    fn service_display_label(
        &self,
        settings: &AppSettings,
        service: &RemoteServiceConfig,
    ) -> String {
        if service.kind == RemoteServiceKind::Google {
            let model = service
                .model
                .as_ref()
                .map(|value| value.trim())
                .filter(|value| !value.is_empty())
                .unwrap_or(&settings.ai.providers.gemini.model);
            return format!("Google ({model})");
        }

        if service.label.trim().is_empty() {
            format!("{:?}", service.kind)
        } else {
            service.label.trim().to_string()
        }
    }

    pub fn build_whisper_stream_engine(&self) -> Result<WhisperStreamEngine, String> {
        let settings = self.load_settings()?;
        let whisper_cli_reference =
            sanitize_whisper_cli_reference(&settings.transcription.whisper_cli_path);
        let whisper_stream_reference = sanitize_whisper_stream_reference(
            &settings.transcription.whisperkit_cli_path,
            &whisper_cli_reference,
        );
        let whisper_stream_path =
            self.resolve_binary_path(&whisper_stream_reference, "whisper-stream");
        let models_dir = self.resolve_models_dir(&settings.transcription.models_dir);
        Ok(WhisperStreamEngine::new(whisper_stream_path, models_dir))
    }

    fn build_speaker_diarizer(
        &self,
        settings: &AppSettings,
    ) -> Result<Option<Arc<dyn sbobino_application::SpeakerDiarizationEngine>>, String> {
        let diarization = &settings.transcription.speaker_diarization;
        if !diarization.enabled {
            return Ok(None);
        }

        self.install_bundled_pyannote_override_if_available()?;
        let pyannote_health = self.pyannote_health(settings);
        if !pyannote_health.ready {
            return Err(pyannote_health.message);
        }

        let python_path = self.managed_pyannote_python_path().ok_or_else(|| {
            "Pyannote diarization is enabled, but the managed Python runtime is unavailable."
                .to_string()
        })?;

        let script_path = self.ensure_embedded_pyannote_script()?;

        if !PathBuf::from(&script_path).is_file() {
            return Err(format!(
                "Pyannote diarization script was not found at '{}'.",
                script_path
            ));
        }

        let model_path = self.ensure_managed_pyannote_model_dir()?.ok_or_else(|| {
            "Pyannote diarization is enabled, but the managed offline model is unavailable."
                .to_string()
        })?;
        let ffmpeg_path = self.resolve_binary_path(&settings.transcription.ffmpeg_path, "ffmpeg");
        let ffmpeg_dir = PathBuf::from(&ffmpeg_path)
            .parent()
            .map(PathBuf::from)
            .into_iter()
            .collect::<Vec<_>>();
        let python_home = pyannote_python_home(&self.managed_pyannote_python_dir());

        Ok(Some(Arc::new(PyannoteSpeakerDiarizationEngine::new(
            python_path,
            python_home,
            script_path,
            model_path,
            diarization.device.trim().to_string(),
            ffmpeg_dir,
        ))))
    }

    pub fn load_settings(&self) -> Result<AppSettings, String> {
        let mut settings = self
            .settings_repo
            .load_sync()
            .map_err(|e| format!("failed to load settings: {e}"))?;

        self.migrate_models_dir_if_needed(&mut settings)?;
        Ok(settings)
    }

    pub fn runtime_health(&self) -> Result<RuntimeHealth, String> {
        let settings = self.load_settings()?;
        self.install_bundled_pyannote_override_if_available()?;
        let configured_models_dir = if settings.transcription.models_dir.trim().is_empty() {
            settings.models_dir.clone()
        } else {
            settings.transcription.models_dir.clone()
        };
        let resolved_models_dir = self.resolve_models_dir(&configured_models_dir);
        let models_dir = PathBuf::from(&resolved_models_dir);

        let whisper_cli_configured =
            sanitize_whisper_cli_reference(&settings.transcription.whisper_cli_path);
        let whisper_stream_configured = sanitize_whisper_stream_reference(
            &settings.transcription.whisperkit_cli_path,
            &whisper_cli_configured,
        );

        let whisper_cli_resolution =
            self.resolve_binary_details(&whisper_cli_configured, "whisper-cli");
        let whisper_stream_resolution =
            self.resolve_binary_details(&whisper_stream_configured, "whisper-stream");
        let ffmpeg_resolution =
            self.resolve_binary_details(&settings.transcription.ffmpeg_path, "ffmpeg");
        let ffmpeg_available = self.binary_path_is_runnable(&ffmpeg_resolution.resolved_path);
        let whisper_cli_available =
            self.binary_path_is_runnable(&whisper_cli_resolution.resolved_path);
        let whisper_stream_available =
            self.binary_path_is_runnable(&whisper_stream_resolution.resolved_path);

        let model_filename = settings.transcription.model.ggml_filename().to_string();
        let model_present = models_dir.join(&model_filename).exists();
        let coreml_encoder = encoder_for_model(&model_filename).unwrap_or_default();
        let coreml_encoder_present = if coreml_encoder.is_empty() {
            false
        } else {
            models_dir.join(coreml_encoder).is_dir()
        };
        let pyannote = self.pyannote_health(&settings);
        let required_models_ready = required_initial_setup_models()
            .iter()
            .all(|model_file| models_dir.join(model_file).exists());
        let required_encoders_ready = if is_apple_silicon_host() {
            required_initial_setup_encoders()
                .iter()
                .all(|encoder_dir| models_dir.join(encoder_dir).is_dir())
        } else {
            true
        };
        let setup_complete = ffmpeg_available
            && whisper_cli_available
            && whisper_stream_available
            && pyannote.ready
            && required_models_ready
            && required_encoders_ready;

        Ok(RuntimeHealth {
            host_os: env::consts::OS.to_string(),
            host_arch: env::consts::ARCH.to_string(),
            is_apple_silicon: is_apple_silicon_host(),
            preferred_engine: preferred_transcription_engine(),
            configured_engine: settings.transcription.engine.clone(),
            ffmpeg_path: settings.transcription.ffmpeg_path.clone(),
            ffmpeg_resolved: ffmpeg_resolution.resolved_path,
            ffmpeg_available,
            whisper_cli_path: whisper_cli_configured,
            whisper_cli_resolved: whisper_cli_resolution.resolved_path,
            whisper_cli_available,
            whisper_stream_path: whisper_stream_configured,
            whisper_stream_resolved: whisper_stream_resolution.resolved_path,
            whisper_stream_available,
            models_dir_configured: configured_models_dir,
            models_dir_resolved: resolved_models_dir,
            model_filename,
            model_present,
            coreml_encoder_present,
            missing_models: missing_models(&models_dir),
            missing_encoders: missing_encoders(&models_dir),
            pyannote,
            setup_complete,
        })
    }

    pub fn resolve_binary_path(&self, configured: &str, fallback: &str) -> String {
        self.resolve_binary_details(configured, fallback)
            .resolved_path
    }

    pub fn resolve_models_dir(&self, configured: &str) -> String {
        let trimmed = configured.trim();
        if trimmed.is_empty() {
            return self.data_dir.join("models").to_string_lossy().to_string();
        }

        if let Some(stripped) = trimmed.strip_prefix("~/") {
            if let Some(home) = std::env::var_os("HOME") {
                return PathBuf::from(home)
                    .join(stripped)
                    .to_string_lossy()
                    .to_string();
            }
        }

        let candidate = PathBuf::from(trimmed);
        if candidate.is_absolute() {
            return candidate.to_string_lossy().to_string();
        }

        self.data_dir.join(candidate).to_string_lossy().to_string()
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn has_bundled_pyannote_override_assets(&self) -> bool {
        self.find_bundled_pyannote_python_source().is_some()
            && self.find_bundled_pyannote_model_source().is_some()
    }

    pub fn reinstall_managed_pyannote_from_bundled_override(&self) -> Result<bool, String> {
        if !self.has_bundled_pyannote_override_assets() {
            return Ok(false);
        }

        let runtime_dir = self.managed_pyannote_runtime_dir();
        if runtime_dir.exists() {
            std::fs::remove_dir_all(&runtime_dir).map_err(|e| {
                format!(
                    "failed to remove stale pyannote runtime directory '{}': {e}",
                    runtime_dir.display()
                )
            })?;
        }

        self.install_bundled_pyannote_override_if_available()?;
        Ok(true)
    }

    fn ensure_embedded_pyannote_script(&self) -> Result<String, String> {
        let runtime_dir = self.managed_pyannote_runtime_dir();
        std::fs::create_dir_all(&runtime_dir)
            .map_err(|e| format!("failed to create runtime directory: {e}"))?;

        let script_path = runtime_dir.join(EMBEDDED_HELPER_FILENAME);
        let script_body = embedded_helper_script();
        let needs_write = std::fs::read_to_string(&script_path)
            .map(|current| current != script_body)
            .unwrap_or(true);

        if needs_write {
            std::fs::write(&script_path, script_body)
                .map_err(|e| format!("failed to write embedded pyannote helper: {e}"))?;
        }

        Ok(script_path.to_string_lossy().to_string())
    }

    pub fn managed_pyannote_runtime_dir(&self) -> PathBuf {
        self.data_dir.join("runtime").join("pyannote")
    }

    pub fn managed_pyannote_python_dir(&self) -> PathBuf {
        self.managed_pyannote_runtime_dir().join("python")
    }

    pub fn managed_pyannote_model_dir(&self) -> PathBuf {
        self.managed_pyannote_runtime_dir().join("model")
    }

    pub fn managed_pyannote_manifest_path(&self) -> PathBuf {
        self.managed_pyannote_runtime_dir()
            .join(PYANNOTE_MANIFEST_FILENAME)
    }

    pub fn managed_pyannote_status_path(&self) -> PathBuf {
        self.managed_pyannote_runtime_dir()
            .join(PYANNOTE_STATUS_FILENAME)
    }

    pub fn read_managed_pyannote_manifest(&self) -> Option<ManagedPyannoteManifest> {
        let path = self.managed_pyannote_manifest_path();
        let content = std::fs::read_to_string(path).ok()?;
        serde_json::from_str::<ManagedPyannoteManifest>(&content).ok()
    }

    pub fn write_managed_pyannote_manifest(
        &self,
        manifest: &ManagedPyannoteManifest,
    ) -> Result<(), String> {
        let runtime_dir = self.managed_pyannote_runtime_dir();
        std::fs::create_dir_all(&runtime_dir)
            .map_err(|e| format!("failed to create pyannote runtime directory: {e}"))?;
        let body = serde_json::to_string_pretty(manifest)
            .map_err(|e| format!("failed to serialize pyannote manifest: {e}"))?;
        std::fs::write(self.managed_pyannote_manifest_path(), body)
            .map_err(|e| format!("failed to write pyannote manifest: {e}"))
    }

    pub fn read_managed_pyannote_status(&self) -> Option<ManagedPyannoteStatus> {
        let path = self.managed_pyannote_status_path();
        let content = std::fs::read_to_string(path).ok()?;
        serde_json::from_str::<ManagedPyannoteStatus>(&content).ok()
    }

    pub fn write_managed_pyannote_status(
        &self,
        reason_code: &str,
        message: &str,
    ) -> Result<(), String> {
        let runtime_dir = self.managed_pyannote_runtime_dir();
        std::fs::create_dir_all(&runtime_dir)
            .map_err(|e| format!("failed to create pyannote runtime directory: {e}"))?;
        let status = ManagedPyannoteStatus {
            reason_code: reason_code.trim().to_string(),
            message: message.trim().to_string(),
            updated_at: Utc::now().to_rfc3339(),
        };
        let body = serde_json::to_string_pretty(&status)
            .map_err(|e| format!("failed to serialize pyannote status: {e}"))?;
        std::fs::write(self.managed_pyannote_status_path(), body)
            .map_err(|e| format!("failed to write pyannote status: {e}"))
    }

    fn managed_pyannote_python_path(&self) -> Option<String> {
        let _ = ensure_embedded_libpython_is_present(&self.managed_pyannote_python_dir());
        let _ = ensure_embedded_pyannote_stdlib_is_present(&self.managed_pyannote_python_dir());
        if pyannote_external_framework_reference(&self.managed_pyannote_python_dir()).is_some() {
            return None;
        }
        for candidate in self.managed_pyannote_python_candidates() {
            if is_runnable_binary_file(&candidate) {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
        None
    }

    fn pyannote_runtime_validation_error(&self) -> Option<String> {
        let python_root = self.managed_pyannote_python_dir();
        let python_path = self.managed_pyannote_python_path()?;
        validate_pyannote_python_runtime(&python_root, Path::new(&python_path)).err()
    }

    fn ensure_managed_pyannote_model_dir(&self) -> Result<Option<String>, String> {
        let runtime_dir = self.managed_pyannote_runtime_dir();
        std::fs::create_dir_all(&runtime_dir)
            .map_err(|e| format!("failed to create pyannote runtime directory: {e}"))?;

        let destination = self.managed_pyannote_model_dir();
        if is_pyannote_model_dir(&destination) {
            return Ok(Some(destination.to_string_lossy().to_string()));
        }

        if let Some(source) = self.find_bundled_pyannote_model_source() {
            copy_directory_recursive(&source, &destination).map_err(|e| {
                format!(
                    "failed to install bundled pyannote model from '{}' to '{}': {e}",
                    source.display(),
                    destination.display()
                )
            })?;
            return Ok(Some(destination.to_string_lossy().to_string()));
        }

        Ok(None)
    }

    fn managed_pyannote_python_candidates(&self) -> Vec<PathBuf> {
        let runtime_dir = self.managed_pyannote_runtime_dir();
        vec![
            self.managed_pyannote_python_dir()
                .join("bin")
                .join("python3"),
            self.managed_pyannote_python_dir()
                .join("bin")
                .join("python"),
            runtime_dir.join("venv").join("bin").join("python3"),
            runtime_dir.join("venv").join("bin").join("python"),
        ]
    }

    fn install_bundled_pyannote_override_if_available(&self) -> Result<(), String> {
        let runtime_dir = self.managed_pyannote_runtime_dir();
        std::fs::create_dir_all(&runtime_dir)
            .map_err(|e| format!("failed to create pyannote runtime directory: {e}"))?;

        let _ = self.managed_pyannote_python_path();
        let repaired_runtime =
            match repair_pyannote_python_launcher(&self.managed_pyannote_python_dir()) {
                Ok(repaired) => repaired,
                Err(error) => {
                    warn!("failed to auto-repair pyannote launcher: {error}");
                    false
                }
            };
        let runtime_path = self.managed_pyannote_python_path();
        let runtime_missing = runtime_path.is_none();
        let runtime_invalid = runtime_path
            .as_ref()
            .and_then(|path| {
                validate_pyannote_python_runtime(
                    &self.managed_pyannote_python_dir(),
                    Path::new(path),
                )
                .err()
            })
            .is_some();
        let model_missing = !is_pyannote_model_dir(&self.managed_pyannote_model_dir());
        let mut copied_assets = false;

        if runtime_missing || runtime_invalid {
            if let Some(source) = self.find_bundled_pyannote_python_source() {
                // Skip bundled overrides that still reference an external Python.framework.
                // A stale or machine-specific bundled runtime must not overwrite a valid managed install.
                if pyannote_external_framework_reference(&source).is_none() {
                    copy_directory_recursive(&source, &self.managed_pyannote_python_dir())
                        .map_err(|e| {
                            format!(
                                "failed to install bundled pyannote runtime from '{}' to '{}': {e}",
                                source.display(),
                                self.managed_pyannote_python_dir().display()
                            )
                        })?;
                    copied_assets = true;
                }
            }
        }

        if model_missing {
            if let Some(source) = self.find_bundled_pyannote_model_source() {
                copy_directory_recursive(&source, &self.managed_pyannote_model_dir()).map_err(
                    |e| {
                        format!(
                            "failed to install bundled pyannote model from '{}' to '{}': {e}",
                            source.display(),
                            self.managed_pyannote_model_dir().display()
                        )
                    },
                )?;
                copied_assets = true;
            }
        }

        let runtime_ready = self.managed_pyannote_python_path().is_some()
            && self.pyannote_runtime_validation_error().is_none();
        let model_ready = is_pyannote_model_dir(&self.managed_pyannote_model_dir());
        let manifest_missing = self.read_managed_pyannote_manifest().is_none();
        let status = self.read_managed_pyannote_status();
        let status_missing = status.is_none();
        let status_not_ok = status
            .as_ref()
            .map(|value| {
                let code = value.reason_code.trim();
                !code.is_empty() && code != "ok"
            })
            .unwrap_or(false);

        if runtime_ready && model_ready {
            let manifest = ManagedPyannoteManifest {
                source: PYANNOTE_BUNDLED_OVERRIDE_SOURCE.to_string(),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                runtime_asset: "bundled-override".to_string(),
                runtime_sha256: String::new(),
                model_asset: "bundled-override".to_string(),
                model_sha256: String::new(),
                runtime_arch: target_triple_suffix().to_string(),
                installed_at: Utc::now().to_rfc3339(),
            };

            if copied_assets || manifest_missing {
                self.write_managed_pyannote_manifest(&manifest)?;
            }
            if copied_assets || repaired_runtime || status_missing || status_not_ok {
                self.write_managed_pyannote_status("ok", "Bundled pyannote override is ready.")?;
            }
        } else if copied_assets || runtime_invalid {
            let message = self
                .pyannote_runtime_validation_error()
                .unwrap_or_else(|| "Pyannote installation is incomplete.".to_string());
            let should_overwrite_status = self
                .read_managed_pyannote_status()
                .map(|status| {
                    let code = status.reason_code.trim();
                    code.is_empty() || code == "ok"
                })
                .unwrap_or(true);
            if should_overwrite_status {
                self.write_managed_pyannote_status("pyannote_install_incomplete", &message)?;
            }
        }

        Ok(())
    }

    fn pyannote_health(&self, settings: &AppSettings) -> PyannoteRuntimeHealth {
        let diarization = &settings.transcription.speaker_diarization;
        let runtime_installed = self.managed_pyannote_python_path().is_some();
        let runtime_validation_error = if runtime_installed {
            self.pyannote_runtime_validation_error()
        } else {
            None
        };
        let model_installed = is_pyannote_model_dir(&self.managed_pyannote_model_dir());
        let manifest = self.read_managed_pyannote_manifest();
        let status = self.read_managed_pyannote_status();
        let arch = manifest
            .as_ref()
            .map(|value| value.runtime_arch.clone())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| target_triple_suffix().to_string());
        let source = manifest
            .as_ref()
            .map(|value| value.source.clone())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "managed".to_string());
        let device = diarization.device.trim().to_string();

        let status_override = status.as_ref().and_then(|value| {
            let code = value.reason_code.trim();
            if code.is_empty() || code == "ok" {
                None
            } else {
                Some((code.to_string(), value.message.trim().to_string()))
            }
        });

        let (ready, reason_code, message) = if let Some((code, message)) = status_override {
            let normalized_code = match code.as_str() {
                "pyannote_install_incomplete" => "pyannote_repair_required".to_string(),
                _ => code.clone(),
            };
            let fallback = match normalized_code.as_str() {
                "pyannote_checksum_invalid" => {
                    "Pyannote asset verification failed. Reinstall the diarization runtime from Local Models."
                }
                "pyannote_repair_required" => {
                    "Pyannote installation is incomplete. Repairing the diarization runtime is required."
                }
                _ => "Pyannote diarization is not ready.",
            };
            (
                false,
                normalized_code,
                if message.is_empty() {
                    fallback.to_string()
                } else {
                    message
                },
            )
        } else if !runtime_installed {
            (
                false,
                "pyannote_runtime_missing".to_string(),
                "Pyannote diarization runtime is not installed. Install it from Settings > Local Models.".to_string(),
            )
        } else if !model_installed {
            (
                false,
                "pyannote_model_missing".to_string(),
                "Pyannote diarization model is not installed. Install it from Settings > Local Models.".to_string(),
            )
        } else if let Some(error) = runtime_validation_error {
            (false, "pyannote_repair_required".to_string(), error)
        } else if let Some(manifest) = manifest.as_ref() {
            if !pyannote_runtime_arch_matches_host(manifest.runtime_arch.trim()) {
                (
                    false,
                    "pyannote_arch_mismatch".to_string(),
                    format!(
                        "Pyannote runtime arch mismatch: installed '{}' but host requires '{}'. Repairing the diarization runtime is required.",
                        manifest.runtime_arch.trim(),
                        target_triple_suffix()
                    ),
                )
            } else if manifest.source != PYANNOTE_BUNDLED_OVERRIDE_SOURCE
                && manifest.app_version.trim() != env!("CARGO_PKG_VERSION")
            {
                (
                    false,
                    "pyannote_version_mismatch".to_string(),
                    format!(
                        "Pyannote runtime targets app version '{}' but this build is '{}'. Repairing the diarization runtime is required.",
                        manifest.app_version.trim(),
                        env!("CARGO_PKG_VERSION")
                    ),
                )
            } else {
                (
                    true,
                    "ok".to_string(),
                    "Pyannote diarization runtime is installed.".to_string(),
                )
            }
        } else {
            (
                false,
                "pyannote_repair_required".to_string(),
                "Pyannote installation is incomplete and must be repaired before setup can continue."
                    .to_string(),
            )
        };

        PyannoteRuntimeHealth {
            enabled: diarization.enabled,
            ready,
            runtime_installed,
            model_installed,
            arch,
            device,
            source,
            reason_code,
            message,
        }
    }

    fn find_bundled_pyannote_model_source(&self) -> Option<PathBuf> {
        let mut candidates = Vec::new();

        if let Some(resources_dir) = self.bundle_resources_dir.as_ref() {
            candidates.push(resources_dir.join("pyannote").join("model"));
            candidates.push(resources_dir.join("pyannote-community-1"));
        }

        candidates.extend([
            self.data_dir.join("bundled").join("pyannote-community-1"),
            self.data_dir.join("resources").join("pyannote-community-1"),
        ]);

        if let Ok(exe) = std::env::current_exe() {
            if let Some(exe_dir) = exe.parent() {
                candidates.push(exe_dir.join("pyannote-community-1"));
                candidates.push(exe_dir.join("resources").join("pyannote-community-1"));
                candidates.push(exe_dir.join("../Resources/pyannote-community-1"));
            }
        }

        if self.allow_dev_resource_overrides {
            let dev_resource_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../apps/desktop/src-tauri/resources");
            candidates.push(dev_resource_dir.join("pyannote").join("model"));
            candidates.push(dev_resource_dir.join("pyannote-community-1"));
        }

        candidates
            .into_iter()
            .find(|path| is_pyannote_model_dir(path))
    }

    fn find_bundled_pyannote_python_source(&self) -> Option<PathBuf> {
        let mut candidates = Vec::new();

        if let Some(resources_dir) = self.bundle_resources_dir.as_ref() {
            candidates.push(
                resources_dir
                    .join("pyannote")
                    .join("python")
                    .join(target_triple_suffix()),
            );
            candidates.push(resources_dir.join("pyannote").join("python"));
        }

        if self.allow_dev_resource_overrides {
            let dev_resource_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../apps/desktop/src-tauri/resources");
            candidates.push(
                dev_resource_dir
                    .join("pyannote")
                    .join("python")
                    .join(target_triple_suffix()),
            );
            candidates.push(dev_resource_dir.join("pyannote").join("python"));
        }

        candidates
            .into_iter()
            .find(|path| is_pyannote_runtime_dir(path))
    }

    fn resolve_binary_details(&self, configured: &str, fallback: &str) -> BinaryResolution {
        let configured_trimmed = configured.trim();

        let configured_candidate = self.find_binary_candidate(configured_trimmed);
        if let Some(path) = configured_candidate.as_ref() {
            if is_runnable_binary_file(path) {
                return BinaryResolution {
                    resolved_path: path.to_string_lossy().to_string(),
                };
            }
        }

        if let Some(path) = self.find_binary_candidate(fallback) {
            return BinaryResolution {
                resolved_path: path.to_string_lossy().to_string(),
            };
        }

        if let Some(path) = configured_candidate {
            return BinaryResolution {
                resolved_path: path.to_string_lossy().to_string(),
            };
        }

        let unresolved = if configured_trimmed.is_empty() {
            fallback.to_string()
        } else {
            configured_trimmed.to_string()
        };
        BinaryResolution {
            resolved_path: unresolved,
        }
    }

    fn find_binary_candidate(&self, value: &str) -> Option<PathBuf> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return None;
        }

        let mut candidates = Vec::<PathBuf>::new();
        let has_separator = trimmed.contains('/') || trimmed.contains('\\');
        let expanded = expand_home(trimmed);
        let expanded_path = PathBuf::from(&expanded);

        if has_separator {
            let Some(file_name) = expanded_path.file_name().and_then(|name| name.to_str()) else {
                if expanded_path.is_absolute() {
                    candidates.push(expanded_path.clone());
                } else {
                    candidates.push(self.data_dir.join(&expanded_path));
                    candidates.push(expanded_path.clone());
                }
                let mut seen = HashSet::<PathBuf>::new();
                return candidates
                    .into_iter()
                    .filter(|candidate| seen.insert(candidate.clone()))
                    .find(|candidate| candidate.is_file());
            };
            let names = binary_name_variants(file_name);
            if expanded_path.is_absolute() {
                for name in &names {
                    let mut candidate = expanded_path.clone();
                    candidate.set_file_name(name);
                    candidates.push(candidate);
                }
            } else {
                for name in &names {
                    let mut local = expanded_path.clone();
                    local.set_file_name(name);
                    candidates.push(self.data_dir.join(&local));
                    candidates.push(local);
                }
            }
        } else {
            let names = binary_name_variants(trimmed);
            for name in &names {
                candidates.push(self.data_dir.join("bin").join(name));
                candidates.push(self.data_dir.join(name));
            }
            // Tauri sidecar wrappers are a compatibility fallback, but once the
            // managed runtime is installed we should prefer it over host-level tools.
            if let Ok(exe) = std::env::current_exe() {
                if let Some(exe_dir) = exe.parent() {
                    for name in &names {
                        candidates.push(exe_dir.join(name));
                    }
                }
            }
            for name in &names {
                candidates.push(PathBuf::from("/opt/homebrew/bin").join(name));
                candidates.push(PathBuf::from("/usr/local/bin").join(name));
                candidates.push(PathBuf::from("/usr/bin").join(name));
            }

            if let Some(path_entries) = env::var_os("PATH") {
                for entry in env::split_paths(&path_entries) {
                    for name in &names {
                        candidates.push(entry.join(name));
                    }
                }
            }

            // Dev fallback: resolve sidecar wrappers directly from src-tauri/binaries
            // so local runs still work without a managed runtime install.
            let dev_sidecar_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../apps/desktop/src-tauri/binaries");
            for name in &names {
                candidates.push(dev_sidecar_dir.join(name));
            }
        }

        let mut seen = HashSet::<PathBuf>::new();
        let deduped = candidates
            .into_iter()
            .filter(|candidate| seen.insert(candidate.clone()))
            .collect::<Vec<_>>();

        if has_separator {
            return deduped
                .iter()
                .find(|candidate| is_runnable_binary_file(candidate))
                .cloned()
                .or_else(|| {
                    deduped
                        .iter()
                        .find(|candidate| is_executable_file(candidate))
                        .cloned()
                })
                .or_else(|| deduped.into_iter().find(|candidate| candidate.is_file()));
        }

        deduped
            .iter()
            .find(|candidate| is_runnable_binary_file(candidate))
            .cloned()
            .or_else(|| {
                deduped
                    .iter()
                    .find(|candidate| is_executable_file(candidate))
                    .cloned()
            })
            .or_else(|| deduped.into_iter().find(|candidate| candidate.is_file()))
    }

    fn binary_path_is_runnable(&self, resolved_path: &str) -> bool {
        let Some(candidate) = self.resolve_existing_binary_path(resolved_path) else {
            return false;
        };

        is_runnable_binary_file(&candidate)
    }

    fn resolve_existing_binary_path(&self, resolved_path: &str) -> Option<PathBuf> {
        let candidate = PathBuf::from(resolved_path);
        if candidate.is_absolute() || resolved_path.contains('/') || resolved_path.contains('\\') {
            if candidate.is_file() {
                return Some(candidate);
            }
        }

        self.find_binary_candidate(resolved_path)
    }
}

fn is_runnable_binary_file(candidate: &Path) -> bool {
    let Ok(metadata) = candidate.metadata() else {
        return false;
    };

    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return false;
        }
    }

    let mut child = match std::process::Command::new(candidate)
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(process) => process,
        Err(_) => return false,
    };

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}
impl RuntimeTranscriptionFactory {
    fn migrate_models_dir_if_needed(&self, settings: &mut AppSettings) -> Result<(), String> {
        let current_models_dir =
            PathBuf::from(self.resolve_models_dir(&settings.transcription.models_dir));
        let Some(legacy_models_dir) = legacy_models_dir() else {
            return Ok(());
        };

        if current_models_dir == legacy_models_dir {
            return Ok(());
        }

        let current_missing_models = missing_models(&current_models_dir);
        let current_missing_encoders = missing_encoders(&current_models_dir);
        if current_missing_models.is_empty() && current_missing_encoders.is_empty() {
            return Ok(());
        }

        let legacy_missing_models = missing_models(&legacy_models_dir);
        let legacy_missing_encoders = missing_encoders(&legacy_models_dir);
        if !legacy_missing_models.is_empty() || !legacy_missing_encoders.is_empty() {
            return Ok(());
        }

        let migrated_models_dir = legacy_models_dir.to_string_lossy().to_string();
        settings.transcription.models_dir = migrated_models_dir.clone();
        settings.models_dir = migrated_models_dir.clone();
        settings.sync_sections_from_legacy();
        settings.sync_legacy_from_sections();

        self.settings_repo
            .save_sync(settings)
            .map_err(|e| format!("failed to persist migrated models path: {e}"))?;
        info!("migrated models directory to {}", migrated_models_dir);
        Ok(())
    }
}

fn is_apple_silicon_host() -> bool {
    #[cfg(target_os = "macos")]
    {
        if cfg!(target_arch = "aarch64") {
            return true;
        }

        if let Ok(output) = std::process::Command::new("sysctl")
            .args(["-n", "hw.optional.arm64"])
            .output()
        {
            if output.status.success() {
                return String::from_utf8_lossy(&output.stdout).trim() == "1";
            }
        }
        false
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

fn preferred_transcription_engine() -> TranscriptionEngine {
    TranscriptionEngine::WhisperCpp
}

fn sanitize_whisper_cli_reference(configured: &str) -> String {
    let trimmed = configured.trim();
    if trimmed.is_empty() || is_legacy_whisperkit_cli_reference(trimmed) {
        return "whisper-cli".to_string();
    }
    trimmed.to_string()
}

fn sanitize_whisper_stream_reference(
    configured_stream: &str,
    whisper_cli_reference: &str,
) -> String {
    let trimmed = configured_stream.trim();
    if trimmed.is_empty() || is_legacy_whisperkit_cli_reference(trimmed) {
        return derive_whisper_stream_reference(whisper_cli_reference);
    }

    if trimmed.contains("whisper-cli") {
        return derive_whisper_stream_reference(trimmed);
    }

    trimmed.to_string()
}

fn is_legacy_whisperkit_cli_reference(value: &str) -> bool {
    value.to_ascii_lowercase().contains("whisperkit-cli")
}

fn derive_whisper_stream_reference(whisper_cli_path: &str) -> String {
    let trimmed = whisper_cli_path.trim();
    if trimmed.is_empty() {
        return "whisper-stream".to_string();
    }

    if trimmed.contains("whisper-cli") {
        return trimmed.replacen("whisper-cli", "whisper-stream", 1);
    }

    "whisper-stream".to_string()
}

fn target_triple_suffix() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        "aarch64-pc-windows-msvc"
    }
}

fn pyannote_runtime_arch_matches_host(installed_arch: &str) -> bool {
    let installed = installed_arch.trim().to_ascii_lowercase();
    if installed.is_empty() {
        return false;
    }

    if installed == target_triple_suffix() {
        return true;
    }

    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return matches!(
            installed.as_str(),
            "macos-aarch64" | "macos-arm64" | "arm64-apple-darwin"
        );
    }

    if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        return matches!(installed.as_str(), "macos-x86_64" | "x86_64-apple-darwin");
    }

    false
}

fn binary_name_variants(base_name: &str) -> Vec<String> {
    let trimmed = base_name.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let suffix = target_triple_suffix();
    let mut variants = vec![trimmed.to_string()];

    if !trimmed.ends_with(suffix) {
        variants.push(format!("{trimmed}-{suffix}"));
    }

    #[cfg(target_os = "windows")]
    {
        if !trimmed.ends_with(".exe") {
            variants.push(format!("{trimmed}.exe"));
        }
        let suffixed = format!("{trimmed}-{suffix}");
        if !suffixed.ends_with(".exe") {
            variants.push(format!("{suffixed}.exe"));
        }
    }

    variants
}

fn required_initial_setup_models() -> [&'static str; 2] {
    ["ggml-base.bin", "ggml-large-v3-turbo-q8_0.bin"]
}

fn required_initial_setup_encoders() -> [&'static str; 2] {
    [
        "ggml-base-encoder.mlmodelc",
        "ggml-large-v3-turbo-encoder.mlmodelc",
    ]
}

#[derive(Clone)]
pub struct InfrastructureBundle {
    pub artifact_service: Arc<ArtifactService>,
    pub settings_service: Arc<SettingsService>,
    pub runtime_factory: Arc<RuntimeTranscriptionFactory>,
}

pub fn bootstrap(
    data_dir: &Path,
    bundle_resources_dir: Option<PathBuf>,
) -> Result<InfrastructureBundle, String> {
    let runtime_factory = Arc::new(RuntimeTranscriptionFactory::new(
        data_dir,
        bundle_resources_dir,
    )?);
    let artifact_service = runtime_factory.artifact_service();
    let settings_service = runtime_factory.settings_service();

    Ok(InfrastructureBundle {
        artifact_service,
        settings_service,
        runtime_factory,
    })
}

fn default_base_url_for_service_kind(kind: &RemoteServiceKind) -> Option<&'static str> {
    match kind {
        RemoteServiceKind::Google => Some("https://generativelanguage.googleapis.com/v1beta"),
        RemoteServiceKind::OpenAi => Some("https://api.openai.com/v1"),
        RemoteServiceKind::OpenRouter => Some("https://openrouter.ai/api/v1"),
        RemoteServiceKind::LmStudio => Some("http://127.0.0.1:1234/v1"),
        RemoteServiceKind::Ollama => Some("http://127.0.0.1:11434/v1"),
        RemoteServiceKind::Xai => Some("https://api.x.ai/v1"),
        RemoteServiceKind::HuggingFace => Some("https://router.huggingface.co/v1"),
        RemoteServiceKind::Anthropic => Some("https://api.anthropic.com/v1"),
        RemoteServiceKind::Azure => Some("https://{resource}.openai.azure.com"),
        RemoteServiceKind::Custom => None,
    }
}

fn default_model_for_service_kind(kind: &RemoteServiceKind) -> Option<&'static str> {
    match kind {
        RemoteServiceKind::Google => Some("gemini-2.5-flash"),
        RemoteServiceKind::OpenAi => Some("gpt-4.1-mini"),
        RemoteServiceKind::OpenRouter => Some("google/gemini-2.5-flash-lite-preview:free"),
        RemoteServiceKind::LmStudio => None,
        RemoteServiceKind::Ollama => Some("llama3.1"),
        RemoteServiceKind::Xai => Some("grok-2-latest"),
        RemoteServiceKind::HuggingFace => None,
        RemoteServiceKind::Anthropic => Some("claude-3-7-sonnet-latest"),
        RemoteServiceKind::Azure => None,
        RemoteServiceKind::Custom => None,
    }
}

fn is_loopback_base_url(value: &str) -> bool {
    let Ok(url) = Url::parse(value.trim()) else {
        return false;
    };

    matches!(
        url.host_str(),
        Some("localhost") | Some("127.0.0.1") | Some("::1")
    )
}

fn missing_models(models_dir: &Path) -> Vec<String> {
    REQUIRED_MODEL_FILES
        .iter()
        .filter_map(|filename| {
            if models_dir.join(filename).exists() {
                None
            } else {
                Some((*filename).to_string())
            }
        })
        .collect::<Vec<_>>()
}

fn missing_encoders(models_dir: &Path) -> Vec<String> {
    REQUIRED_COREML_ENCODERS
        .iter()
        .filter_map(|(_model, encoder_dir)| {
            if models_dir.join(encoder_dir).is_dir() {
                None
            } else {
                Some((*encoder_dir).to_string())
            }
        })
        .collect::<Vec<_>>()
}

fn encoder_for_model(model_filename: &str) -> Option<&'static str> {
    REQUIRED_COREML_ENCODERS
        .iter()
        .find(|(model, _encoder)| *model == model_filename)
        .map(|(_model, encoder)| *encoder)
}

fn legacy_models_dir() -> Option<PathBuf> {
    env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("sbobino")
            .join("models")
    })
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = path.metadata() {
            return metadata.permissions().mode() & 0o111 != 0;
        }
        false
    }

    #[cfg(not(unix))]
    {
        true
    }
}

fn copy_directory_recursive(source: &Path, destination: &Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(destination)?;

    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let entry_type = entry.file_type()?;
        let target_path = destination.join(entry.file_name());

        if entry_type.is_dir() {
            copy_directory_recursive(&entry.path(), &target_path)?;
        } else if entry_type.is_file() {
            std::fs::copy(entry.path(), target_path)?;
        }
    }

    Ok(())
}

fn ensure_embedded_libpython_is_present(runtime_root: &Path) -> Result<(), String> {
    let lib_dir = runtime_root.join("lib");
    if !lib_dir.is_dir() {
        return Ok(());
    }

    let already_embedded = std::fs::read_dir(&lib_dir)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry.path())
        .any(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .map(|value| value.starts_with("libpython") && value.ends_with(".dylib"))
                    .unwrap_or(false)
        });
    if already_embedded {
        return Ok(());
    }

    let Some(source) = find_file_recursive(runtime_root, &|path| {
        path.is_file()
            && path
                .file_name()
                .and_then(|value| value.to_str())
                .map(|value| value.starts_with("libpython") && value.ends_with(".dylib"))
                .unwrap_or(false)
    }) else {
        return Ok(());
    };

    std::fs::create_dir_all(&lib_dir)
        .map_err(|e| format!("failed to create embedded libpython directory: {e}"))?;
    let target = lib_dir.join(
        source
            .file_name()
            .ok_or_else(|| "embedded libpython candidate is missing a file name".to_string())?,
    );
    std::fs::copy(&source, &target).map_err(|e| {
        format!(
            "failed to copy embedded libpython from '{}' to '{}': {e}",
            source.display(),
            target.display()
        )
    })?;
    Ok(())
}

fn ensure_embedded_pyannote_stdlib_is_present(runtime_root: &Path) -> Result<(), String> {
    let Some(version_dir_name) = pyannote_python_version_dir_name(runtime_root) else {
        return Ok(());
    };
    let stdlib_dir = runtime_root.join("lib").join(&version_dir_name);
    let has_local_stdlib =
        stdlib_dir.join("encodings").is_dir() && stdlib_dir.join("lib-dynload").is_dir();
    if has_local_stdlib {
        return Ok(());
    }

    let Some(source_stdlib_dir) = find_pyannote_source_stdlib_dir(runtime_root, &version_dir_name)
    else {
        return Ok(());
    };

    std::fs::create_dir_all(&stdlib_dir)
        .map_err(|e| format!("failed to create bundled stdlib directory: {e}"))?;

    for entry in std::fs::read_dir(&source_stdlib_dir).map_err(|e| {
        format!(
            "failed to read source stdlib '{}': {e}",
            source_stdlib_dir.display()
        )
    })? {
        let entry = entry.map_err(|e| format!("failed to inspect source stdlib entry: {e}"))?;
        let name = entry.file_name();
        if name.to_string_lossy() == "site-packages" {
            continue;
        }
        let source_path = entry.path();
        let target_path = stdlib_dir.join(&name);
        if entry
            .file_type()
            .map_err(|e| format!("failed to inspect stdlib entry type: {e}"))?
            .is_dir()
        {
            copy_directory_recursive(&source_path, &target_path).map_err(|e| {
                format!(
                    "failed to copy stdlib directory '{}' to '{}': {e}",
                    source_path.display(),
                    target_path.display()
                )
            })?;
        } else if source_path.is_file() {
            std::fs::copy(&source_path, &target_path).map_err(|e| {
                format!(
                    "failed to copy stdlib file '{}' to '{}': {e}",
                    source_path.display(),
                    target_path.display()
                )
            })?;
        }
    }

    Ok(())
}

fn pyannote_python_version_dir_name(runtime_root: &Path) -> Option<String> {
    let lib_root = runtime_root.join("lib");
    let entries = std::fs::read_dir(&lib_root).ok()?;
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find_map(|path| {
            let file_name = path.file_name()?.to_str()?;
            if path.is_dir() && file_name.starts_with("python3.") {
                Some(file_name.to_string())
            } else {
                None
            }
        })
}

fn pyannote_python_home(runtime_root: &Path) -> Option<PathBuf> {
    let version_dir_name = pyannote_python_version_dir_name(runtime_root)?;
    let stdlib_dir = runtime_root.join("lib").join(version_dir_name);
    if stdlib_dir.join("encodings").is_dir() && stdlib_dir.join("lib-dynload").is_dir() {
        Some(runtime_root.to_path_buf())
    } else {
        None
    }
}

fn parse_external_python_framework_reference(otool_output: &str) -> Option<String> {
    otool_output.lines().skip(1).find_map(|line| {
        let dependency = line.split_whitespace().next()?;
        if dependency.starts_with('/')
            && dependency.contains("Python.framework/Versions/")
            && dependency.ends_with("/Python")
        {
            Some(dependency.to_string())
        } else {
            None
        }
    })
}

fn pyannote_python_app_binary(runtime_root: &Path) -> PathBuf {
    runtime_root
        .join("lib")
        .join("Resources")
        .join("Python.app")
        .join("Contents")
        .join("MacOS")
        .join("Python")
}

fn parse_pyannote_python_framework_version(reference: &str) -> Option<String> {
    let marker = "/Python.framework/Versions/";
    let start = reference.find(marker)? + marker.len();
    let tail = &reference[start..];
    let version = tail.split('/').next()?.trim();
    if version.is_empty() {
        None
    } else {
        Some(version.to_string())
    }
}

fn parse_otool_rpath_entries(otool_output: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut expect_path = false;

    for line in otool_output.lines() {
        let trimmed = line.trim();
        if trimmed == "cmd LC_RPATH" {
            expect_path = true;
            continue;
        }

        if expect_path && trimmed.starts_with("path ") {
            let path = trimmed
                .trim_start_matches("path ")
                .split(" (offset ")
                .next()
                .unwrap_or_default()
                .trim();
            if !path.is_empty() {
                result.push(path.to_string());
            }
            expect_path = false;
            continue;
        }

        if trimmed.starts_with("cmd ") && trimmed != "cmd LC_RPATH" {
            expect_path = false;
        }
    }

    result
}

fn pyannote_python_app_rpaths(runtime_root: &Path) -> Option<Vec<String>> {
    if !cfg!(target_os = "macos") {
        return Some(Vec::new());
    }

    let python_app = pyannote_python_app_binary(runtime_root);
    if !python_app.is_file() {
        return Some(Vec::new());
    }

    let output = std::process::Command::new("/usr/bin/otool")
        .arg("-l")
        .arg(&python_app)
        .output()
        .ok()?;
    if !output.status.success() {
        return Some(Vec::new());
    }

    Some(parse_otool_rpath_entries(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn repair_pyannote_python_launcher(runtime_root: &Path) -> Result<bool, String> {
    if !cfg!(target_os = "macos") {
        return Ok(false);
    }

    let Some(reference) = pyannote_external_framework_reference(runtime_root) else {
        return Ok(false);
    };

    let python_app = pyannote_python_app_binary(runtime_root);
    if !python_app.is_file() {
        return Ok(false);
    }

    let version = parse_pyannote_python_framework_version(&reference).ok_or_else(|| {
        format!("failed to parse Python.framework version from dependency '{reference}'")
    })?;
    let libpython_name = format!("libpython{version}.dylib");
    let embedded_libpython = runtime_root.join("lib").join(&libpython_name);
    if !embedded_libpython.is_file() {
        return Err(format!(
            "pyannote launcher references external framework '{}' but '{}' is missing in '{}'",
            reference,
            libpython_name,
            runtime_root.join("lib").display()
        ));
    }

    let relocated_reference = format!("@rpath/{libpython_name}");
    let change_output = std::process::Command::new("/usr/bin/install_name_tool")
        .arg("-change")
        .arg(&reference)
        .arg(&relocated_reference)
        .arg(&python_app)
        .output()
        .map_err(|e| format!("failed to run install_name_tool -change: {e}"))?;
    if !change_output.status.success() {
        return Err(format!(
            "install_name_tool -change failed: {}",
            String::from_utf8_lossy(&change_output.stderr).trim()
        ));
    }

    let required_rpath = "@executable_path/../../../../";
    let has_required_rpath = pyannote_python_app_rpaths(runtime_root)
        .unwrap_or_default()
        .iter()
        .any(|entry| entry == required_rpath);
    if !has_required_rpath {
        let add_output = std::process::Command::new("/usr/bin/install_name_tool")
            .arg("-add_rpath")
            .arg(required_rpath)
            .arg(&python_app)
            .output()
            .map_err(|e| format!("failed to run install_name_tool -add_rpath: {e}"))?;
        if !add_output.status.success() {
            let stderr = String::from_utf8_lossy(&add_output.stderr)
                .trim()
                .to_string();
            if !stderr.contains("would duplicate path")
                && !stderr.contains("already contains")
                && !stderr.contains("already has LC_RPATH")
            {
                return Err(format!("install_name_tool -add_rpath failed: {stderr}"));
            }
        }
    }

    let sign_output = std::process::Command::new("/usr/bin/codesign")
        .arg("--force")
        .arg("--sign")
        .arg("-")
        .arg(&python_app)
        .output()
        .map_err(|e| format!("failed to run codesign for pyannote launcher: {e}"))?;
    if !sign_output.status.success() {
        return Err(format!(
            "codesign failed for '{}': {}",
            python_app.display(),
            String::from_utf8_lossy(&sign_output.stderr).trim()
        ));
    }

    Ok(true)
}

fn pyannote_external_framework_reference(runtime_root: &Path) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }

    let python_app = pyannote_python_app_binary(runtime_root);
    if !python_app.is_file() {
        return None;
    }

    let output = std::process::Command::new("/usr/bin/otool")
        .arg("-L")
        .arg(&python_app)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    parse_external_python_framework_reference(&String::from_utf8_lossy(&output.stdout))
}

fn find_pyannote_source_stdlib_dir(runtime_root: &Path, version_dir_name: &str) -> Option<PathBuf> {
    let pyvenv_cfg = runtime_root.join("pyvenv.cfg");
    let body = std::fs::read_to_string(pyvenv_cfg).ok()?;
    let home = body
        .lines()
        .find_map(|line| line.strip_prefix("home = "))
        .map(str::trim)?;
    let home_path = PathBuf::from(home);
    let prefix = if home_path.file_name().and_then(|value| value.to_str()) == Some("bin") {
        home_path.parent().map(PathBuf::from)?
    } else {
        home_path
    };

    let candidates = [
        prefix.join("lib").join(version_dir_name),
        prefix
            .parent()
            .map(|parent| parent.join("lib").join(version_dir_name))
            .unwrap_or_default(),
    ];

    candidates
        .into_iter()
        .find(|candidate| candidate.join("encodings").is_dir())
}

fn validate_pyannote_python_runtime(
    runtime_root: &Path,
    python_binary: &Path,
) -> Result<(), String> {
    if let Some(reference) = pyannote_external_framework_reference(runtime_root) {
        return Err(format!(
            "Pyannote runtime still depends on an external Python framework ('{reference}'). Repair or reinstall it from Settings > Local Models."
        ));
    }

    if !is_runnable_binary_file(python_binary) {
        return Err(format!(
            "Pyannote runtime binary is not runnable at '{}'.",
            python_binary.display()
        ));
    }

    let python_home = pyannote_python_home(runtime_root).ok_or_else(|| {
        "Pyannote runtime is missing the bundled Python standard library. Repair or reinstall it from Settings > Local Models.".to_string()
    })?;

    let mut child = std::process::Command::new(python_binary)
        .arg("-c")
        .arg("import ctypes,csv,encodings; import torch; from pyannote.audio import Pipeline")
        .env("PYTHONHOME", &python_home)
        .env("PYTHONNOUSERSITE", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            format!(
                "failed to start pyannote runtime validation with '{}': {e}",
                python_binary.display()
            )
        })?;

    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => {
                let output = child.wait_with_output().map_err(|e| {
                    format!("failed to read pyannote runtime validation output: {e}")
                })?;
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                return Err(if stderr.is_empty() {
                    "Pyannote runtime validation failed.".to_string()
                } else {
                    format!("Pyannote runtime validation failed: {stderr}")
                });
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(
                        "Pyannote runtime validation timed out while importing dependencies."
                            .to_string(),
                    );
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "failed to wait for pyannote runtime validation: {e}"
                ));
            }
        }
    }
}

fn find_file_recursive<F>(root: &Path, predicate: &F) -> Option<PathBuf>
where
    F: Fn(&Path) -> bool,
{
    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if predicate(&path) {
            return Some(path);
        }
        if path.is_dir() {
            if let Some(found) = find_file_recursive(&path, predicate) {
                return Some(found);
            }
        }
    }
    None
}

fn is_pyannote_model_dir(path: &Path) -> bool {
    path.is_dir() && path.join("config.yaml").is_file()
}

fn is_pyannote_runtime_dir(path: &Path) -> bool {
    path.is_dir()
        && (path.join("bin").join("python3").is_file() || path.join("bin").join("python").is_file())
}

fn expand_home(path: &str) -> String {
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home)
                .join(stripped)
                .to_string_lossy()
                .to_string();
        }
    }
    path.to_string()
}

#[cfg(test)]
mod tests {
    use super::{
        parse_external_python_framework_reference, parse_otool_rpath_entries,
        parse_pyannote_python_framework_version, pyannote_runtime_arch_matches_host,
        target_triple_suffix, ManagedPyannoteManifest, RuntimeTranscriptionFactory,
        PYANNOTE_STATUS_FILENAME,
    };
    use sbobino_domain::{AiProvider, AppSettings, RemoteServiceConfig, RemoteServiceKind};
    use tempfile::tempdir;

    fn build_factory() -> (tempfile::TempDir, RuntimeTranscriptionFactory) {
        std::env::set_var("SBOBINO_ALLOW_INSECURE_LOCAL_SECRETS", "1");
        let temp = tempdir().expect("failed to create tempdir");
        let factory = RuntimeTranscriptionFactory::new_for_tests(temp.path(), None)
            .expect("factory should build");
        (temp, factory)
    }

    fn build_factory_with_bundle_resources() -> (
        tempfile::TempDir,
        RuntimeTranscriptionFactory,
        std::path::PathBuf,
    ) {
        std::env::set_var("SBOBINO_ALLOW_INSECURE_LOCAL_SECRETS", "1");
        let temp = tempdir().expect("failed to create tempdir");
        let resources_dir = temp.path().join("resources");
        let factory =
            RuntimeTranscriptionFactory::new_for_tests(temp.path(), Some(resources_dir.clone()))
                .expect("factory should build");
        (temp, factory, resources_dir)
    }

    fn persist_enabled_diarization(factory: &RuntimeTranscriptionFactory) {
        let mut settings = AppSettings::default();
        settings.transcription.speaker_diarization.enabled = true;
        factory
            .settings_repo
            .save_sync(&settings)
            .expect("settings should persist");
    }

    fn persist_settings(factory: &RuntimeTranscriptionFactory, settings: &AppSettings) {
        factory
            .settings_repo
            .save_sync(settings)
            .expect("settings should persist");
    }

    fn write_executable_file(path: &std::path::Path, contents: &str) {
        std::fs::create_dir_all(
            path.parent()
                .expect("executable file should have parent directory"),
        )
        .expect("parent directory should exist");
        std::fs::write(path, contents).expect("failed to write executable file");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(path)
                .expect("metadata should exist")
                .permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).expect("failed to chmod executable");
        }
    }

    fn write_fake_pyannote_stdlib(python_root: &std::path::Path, version_dir_name: &str) {
        let stdlib_dir = python_root.join("lib").join(version_dir_name);
        std::fs::create_dir_all(stdlib_dir.join("encodings"))
            .expect("encodings directory should exist");
        std::fs::create_dir_all(stdlib_dir.join("lib-dynload"))
            .expect("lib-dynload directory should exist");
        std::fs::write(stdlib_dir.join("encodings").join("__init__.py"), "# test\n")
            .expect("encodings marker should write");
    }

    #[test]
    fn runtime_health_reports_missing_pyannote_runtime_when_enabled() {
        let (_temp, factory) = build_factory();
        persist_enabled_diarization(&factory);

        let health = factory
            .runtime_health()
            .expect("runtime health should load");
        assert!(health.pyannote.enabled);
        assert!(!health.pyannote.ready);
        assert_eq!(health.pyannote.reason_code, "pyannote_runtime_missing");
    }

    #[test]
    fn runtime_health_prefers_checksum_status_when_present() {
        let (_temp, factory) = build_factory();
        persist_enabled_diarization(&factory);

        let runtime_dir = factory.managed_pyannote_runtime_dir();
        std::fs::create_dir_all(&runtime_dir).expect("runtime dir should exist");
        std::fs::write(
            runtime_dir.join(PYANNOTE_STATUS_FILENAME),
            r#"{"reason_code":"pyannote_checksum_invalid","message":"checksum mismatch","updated_at":"2026-03-13T00:00:00Z"}"#,
        )
        .expect("status file should write");

        let health = factory
            .runtime_health()
            .expect("runtime health should load");
        assert_eq!(health.pyannote.reason_code, "pyannote_checksum_invalid");
        assert!(health.pyannote.message.contains("checksum mismatch"));
    }

    #[test]
    fn runtime_health_reports_ready_when_manifest_runtime_and_model_exist() {
        let (_temp, factory) = build_factory();
        persist_enabled_diarization(&factory);

        write_executable_file(
            &factory
                .managed_pyannote_python_dir()
                .join("bin")
                .join("python3"),
            "#!/bin/sh\nexit 0\n",
        );
        write_fake_pyannote_stdlib(&factory.managed_pyannote_python_dir(), "python3.11");
        let model_dir = factory.managed_pyannote_model_dir();
        std::fs::create_dir_all(&model_dir).expect("model dir should exist");
        std::fs::write(model_dir.join("config.yaml"), "name: test\n").expect("config should write");
        factory
            .write_managed_pyannote_manifest(&ManagedPyannoteManifest {
                source: "release_asset".to_string(),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                runtime_asset: "pyannote-runtime-macos-aarch64.zip".to_string(),
                runtime_sha256: "abc".to_string(),
                model_asset: "pyannote-model-community-1.zip".to_string(),
                model_sha256: "def".to_string(),
                runtime_arch: super::target_triple_suffix().to_string(),
                installed_at: "2026-03-13T00:00:00Z".to_string(),
            })
            .expect("manifest should write");
        factory
            .write_managed_pyannote_status("ok", "ready")
            .expect("status should write");

        let health = factory
            .runtime_health()
            .expect("runtime health should load");
        assert!(health.pyannote.ready);
        assert_eq!(health.pyannote.reason_code, "ok");
    }

    #[test]
    fn runtime_health_accepts_legacy_macos_arch_label_when_otherwise_ready() {
        let (_temp, factory) = build_factory();
        persist_enabled_diarization(&factory);

        write_executable_file(
            &factory
                .managed_pyannote_python_dir()
                .join("bin")
                .join("python3"),
            "#!/bin/sh\nexit 0\n",
        );
        write_fake_pyannote_stdlib(&factory.managed_pyannote_python_dir(), "python3.11");
        let model_dir = factory.managed_pyannote_model_dir();
        std::fs::create_dir_all(&model_dir).expect("model dir should exist");
        std::fs::write(model_dir.join("config.yaml"), "name: test\n").expect("config should write");
        let legacy_arch = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            "macos-aarch64".to_string()
        } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
            "macos-x86_64".to_string()
        } else {
            super::target_triple_suffix().to_string()
        };
        factory
            .write_managed_pyannote_manifest(&ManagedPyannoteManifest {
                source: "release_asset".to_string(),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                runtime_asset: "pyannote-runtime-macos-aarch64.zip".to_string(),
                runtime_sha256: "abc".to_string(),
                model_asset: "pyannote-model-community-1.zip".to_string(),
                model_sha256: "def".to_string(),
                runtime_arch: legacy_arch,
                installed_at: "2026-03-13T00:00:00Z".to_string(),
            })
            .expect("manifest should write");
        factory
            .write_managed_pyannote_status("ok", "ready")
            .expect("status should write");

        let health = factory
            .runtime_health()
            .expect("runtime health should load");
        assert!(health.pyannote.ready);
        assert_eq!(health.pyannote.reason_code, "ok");
    }

    #[test]
    fn runtime_health_self_heals_stale_install_incomplete_status_when_runtime_is_ready() {
        let (_temp, factory) = build_factory();
        persist_enabled_diarization(&factory);

        write_executable_file(
            &factory
                .managed_pyannote_python_dir()
                .join("bin")
                .join("python3"),
            "#!/bin/sh\nexit 0\n",
        );
        write_fake_pyannote_stdlib(&factory.managed_pyannote_python_dir(), "python3.11");
        let model_dir = factory.managed_pyannote_model_dir();
        std::fs::create_dir_all(&model_dir).expect("model dir should exist");
        std::fs::write(model_dir.join("config.yaml"), "name: test\n").expect("config should write");
        factory
            .write_managed_pyannote_manifest(&ManagedPyannoteManifest {
                source: "release_asset".to_string(),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                runtime_asset: "pyannote-runtime-macos-aarch64.zip".to_string(),
                runtime_sha256: "abc".to_string(),
                model_asset: "pyannote-model-community-1.zip".to_string(),
                model_sha256: "def".to_string(),
                runtime_arch: super::target_triple_suffix().to_string(),
                installed_at: "2026-03-13T00:00:00Z".to_string(),
            })
            .expect("manifest should write");
        factory
            .write_managed_pyannote_status(
                "pyannote_install_incomplete",
                "stale status from previous setup run",
            )
            .expect("status should write");

        let health = factory
            .runtime_health()
            .expect("runtime health should load");
        assert!(health.pyannote.ready);
        assert_eq!(health.pyannote.reason_code, "ok");

        let status = factory
            .read_managed_pyannote_status()
            .expect("status should still be present");
        assert_eq!(status.reason_code, "ok");
    }

    #[test]
    fn runtime_health_self_heals_missing_manifest_and_status_from_bundled_override() {
        let (_temp, factory, resources_dir) = build_factory_with_bundle_resources();
        persist_enabled_diarization(&factory);

        write_executable_file(
            &resources_dir
                .join("pyannote")
                .join("python")
                .join(target_triple_suffix())
                .join("bin")
                .join("python3"),
            "#!/bin/sh\nexit 0\n",
        );
        write_fake_pyannote_stdlib(
            &resources_dir
                .join("pyannote")
                .join("python")
                .join(target_triple_suffix()),
            "python3.11",
        );
        std::fs::create_dir_all(resources_dir.join("pyannote").join("model"))
            .expect("model directory should exist");
        std::fs::write(
            resources_dir
                .join("pyannote")
                .join("model")
                .join("config.yaml"),
            "name: bundled\n",
        )
        .expect("config should write");

        let health = factory
            .runtime_health()
            .expect("runtime health should load");
        assert!(health.pyannote.ready);
        assert_eq!(health.pyannote.reason_code, "ok");

        let manifest = factory
            .read_managed_pyannote_manifest()
            .expect("manifest should be written");
        assert_eq!(manifest.source, "bundled_override");

        let status = factory
            .read_managed_pyannote_status()
            .expect("status should be written");
        assert_eq!(status.reason_code, "ok");

        assert!(factory
            .managed_pyannote_python_dir()
            .join("bin")
            .join("python3")
            .is_file());
        assert!(factory
            .managed_pyannote_model_dir()
            .join("config.yaml")
            .is_file());
    }

    #[test]
    fn runtime_health_repairs_missing_embedded_libpython_before_runnable_check() {
        let (_temp, factory) = build_factory();
        persist_enabled_diarization(&factory);

        write_executable_file(
            &factory
                .managed_pyannote_python_dir()
                .join("bin")
                .join("python3"),
            "#!/bin/sh\nexit 0\n",
        );
        write_fake_pyannote_stdlib(&factory.managed_pyannote_python_dir(), "python3.11");
        let nested_libpython = factory
            .managed_pyannote_python_dir()
            .join("lib")
            .join("python3.11")
            .join("site-packages")
            .join("torchcodec")
            .join(".dylibs")
            .join("libpython3.11.dylib");
        std::fs::create_dir_all(
            nested_libpython
                .parent()
                .expect("libpython should have a parent directory"),
        )
        .expect("nested libpython parent should exist");
        std::fs::write(&nested_libpython, "fake-libpython").expect("nested libpython should write");

        assert!(!factory
            .managed_pyannote_python_dir()
            .join("lib")
            .join("libpython3.11.dylib")
            .exists());

        let _ = factory.managed_pyannote_python_path();

        assert!(factory
            .managed_pyannote_python_dir()
            .join("lib")
            .join("libpython3.11.dylib")
            .is_file());
    }

    #[test]
    fn runtime_health_reports_install_incomplete_when_python_stdlib_is_missing() {
        let (_temp, factory) = build_factory();
        persist_enabled_diarization(&factory);

        write_executable_file(
            &factory
                .managed_pyannote_python_dir()
                .join("bin")
                .join("python3"),
            "#!/bin/sh\nexit 0\n",
        );
        let model_dir = factory.managed_pyannote_model_dir();
        std::fs::create_dir_all(&model_dir).expect("model dir should exist");
        std::fs::write(model_dir.join("config.yaml"), "name: test\n").expect("config should write");
        factory
            .write_managed_pyannote_manifest(&ManagedPyannoteManifest {
                source: "release_asset".to_string(),
                app_version: env!("CARGO_PKG_VERSION").to_string(),
                runtime_asset: "pyannote-runtime-macos-aarch64.zip".to_string(),
                runtime_sha256: "abc".to_string(),
                model_asset: "pyannote-model-community-1.zip".to_string(),
                model_sha256: "def".to_string(),
                runtime_arch: super::target_triple_suffix().to_string(),
                installed_at: "2026-03-13T00:00:00Z".to_string(),
            })
            .expect("manifest should write");
        factory
            .write_managed_pyannote_status("ok", "ready")
            .expect("status should write");

        let health = factory
            .runtime_health()
            .expect("runtime health should load");
        assert!(!health.pyannote.ready);
        assert_eq!(health.pyannote.reason_code, "pyannote_repair_required");
        assert!(health.pyannote.message.contains("standard library"));
    }

    #[test]
    fn runtime_health_reports_version_mismatch_as_repair_required() {
        let (_temp, factory) = build_factory();
        persist_enabled_diarization(&factory);

        write_executable_file(
            &factory
                .managed_pyannote_python_dir()
                .join("bin")
                .join("python3"),
            "#!/bin/sh\nexit 0\n",
        );
        write_fake_pyannote_stdlib(&factory.managed_pyannote_python_dir(), "python3.11");
        let model_dir = factory.managed_pyannote_model_dir();
        std::fs::create_dir_all(&model_dir).expect("model dir should exist");
        std::fs::write(model_dir.join("config.yaml"), "name: test\n").expect("config should write");
        factory
            .write_managed_pyannote_manifest(&ManagedPyannoteManifest {
                source: "release_asset".to_string(),
                app_version: "0.1.2".to_string(),
                runtime_asset: "pyannote-runtime-macos-aarch64.zip".to_string(),
                runtime_sha256: "abc".to_string(),
                model_asset: "pyannote-model-community-1.zip".to_string(),
                model_sha256: "def".to_string(),
                runtime_arch: super::target_triple_suffix().to_string(),
                installed_at: "2026-03-13T00:00:00Z".to_string(),
            })
            .expect("manifest should write");
        factory
            .write_managed_pyannote_status("ok", "ready")
            .expect("status should write");

        let health = factory
            .runtime_health()
            .expect("runtime health should load");
        assert!(!health.pyannote.ready);
        assert_eq!(health.pyannote.reason_code, "pyannote_version_mismatch");
        assert!(health
            .pyannote
            .message
            .contains("Repairing the diarization runtime is required"));
    }

    #[test]
    fn build_service_fails_when_enabled_pyannote_is_not_ready() {
        let (_temp, factory) = build_factory();
        persist_enabled_diarization(&factory);

        let error = match factory.build_service() {
            Ok(_) => panic!("service should fail when pyannote is required but missing"),
            Err(error) => error,
        };
        assert!(error.contains("Pyannote diarization runtime is not installed"));
    }

    #[test]
    fn enhancer_candidates_prefer_active_remote_then_foundation_then_local() {
        let (_temp, factory) = build_factory();
        let mut settings = AppSettings::default();
        settings.ai.active_provider = AiProvider::Gemini;
        settings.ai.active_remote_service_id = Some("remote-google".to_string());
        settings.ai.providers.gemini.api_key = Some("test-key".to_string());
        settings.ai.providers.foundation_apple.enabled = true;
        settings.ai.remote_services = vec![
            RemoteServiceConfig {
                id: "remote-google".to_string(),
                kind: RemoteServiceKind::Google,
                label: "Google".to_string(),
                enabled: true,
                api_key: Some("test-key".to_string()),
                has_api_key: true,
                model: Some("gemini-2.5-flash".to_string()),
                base_url: None,
            },
            RemoteServiceConfig {
                id: "local-ollama".to_string(),
                kind: RemoteServiceKind::Ollama,
                label: "Local Ollama".to_string(),
                enabled: true,
                api_key: None,
                has_api_key: false,
                model: Some("llama3.1".to_string()),
                base_url: Some("http://127.0.0.1:11434/v1".to_string()),
            },
        ];
        persist_settings(&factory, &settings);

        let candidates = factory
            .build_enhancer_candidates()
            .expect("candidate chain should build");
        let labels = candidates
            .iter()
            .map(|candidate| candidate.label.as_str())
            .collect::<Vec<_>>();

        assert_eq!(labels.first().copied(), Some("Google (gemini-2.5-flash)"));
        if super::is_apple_silicon_host() {
            assert_eq!(labels.get(1).copied(), Some("Apple Foundation"));
            assert_eq!(labels.get(2).copied(), Some("Local Ollama"));
        } else {
            assert_eq!(labels.get(1).copied(), Some("Local Ollama"));
        }
    }

    #[test]
    fn enhancer_candidates_allow_local_only_chain() {
        let (_temp, factory) = build_factory();
        let mut settings = AppSettings::default();
        settings.ai.active_provider = AiProvider::None;
        settings.ai.providers.foundation_apple.enabled = false;
        settings.ai.remote_services = vec![RemoteServiceConfig {
            id: "local-custom".to_string(),
            kind: RemoteServiceKind::Custom,
            label: "Local Custom".to_string(),
            enabled: true,
            api_key: None,
            has_api_key: false,
            model: Some("qwen2.5".to_string()),
            base_url: Some("http://localhost:8080/v1".to_string()),
        }];
        persist_settings(&factory, &settings);

        let candidates = factory
            .build_enhancer_candidates()
            .expect("local candidate should build");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].label, "Local Custom");
    }

    #[test]
    fn ai_capability_status_reports_unavailable_when_no_candidate_exists() {
        let (_temp, factory) = build_factory();
        let mut settings = AppSettings::default();
        settings.ai.active_provider = AiProvider::None;
        settings.ai.providers.foundation_apple.enabled = false;
        settings.ai.remote_services = Vec::new();
        persist_settings(&factory, &settings);

        let status = factory
            .ai_capability_status()
            .expect("capability status should load");
        assert!(!status.available);
        assert!(!status.fallback_available);
        assert!(status
            .unavailable_reason
            .expect("reason should exist")
            .contains("Settings > AI Services"));
    }

    #[test]
    fn parse_external_python_framework_reference_detects_absolute_framework_link() {
        let output = r#"
/tmp/Python:
    /System/Library/Frameworks/CoreFoundation.framework/Versions/A/CoreFoundation (compatibility version 150.0.0, current version 1.0.0)
    /Library/Frameworks/Python.framework/Versions/3.11/Python (compatibility version 3.11.0, current version 3.11.0)
    /usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1.0.0)
"#;

        assert_eq!(
            parse_external_python_framework_reference(output).as_deref(),
            Some("/Library/Frameworks/Python.framework/Versions/3.11/Python")
        );
    }

    #[test]
    fn parse_external_python_framework_reference_ignores_relocatable_rpath_link() {
        let output = r#"
/tmp/Python:
    @rpath/libpython3.11.dylib (compatibility version 3.11.0, current version 3.11.0)
    /usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1.0.0)
"#;

        assert!(parse_external_python_framework_reference(output).is_none());
    }

    #[test]
    fn parse_pyannote_python_framework_version_extracts_expected_version() {
        let reference = "/Library/Frameworks/Python.framework/Versions/3.11/Python";
        assert_eq!(
            parse_pyannote_python_framework_version(reference).as_deref(),
            Some("3.11")
        );
    }

    #[test]
    fn parse_otool_rpath_entries_extracts_lc_rpath_paths() {
        let output = r#"
/tmp/Python:
Load command 13
          cmd LC_RPATH
      cmdsize 32
         path @executable_path/../../../../ (offset 12)
Load command 14
          cmd LC_RPATH
      cmdsize 32
         path /usr/lib (offset 12)
"#;

        let entries = parse_otool_rpath_entries(output);
        assert_eq!(entries, vec!["@executable_path/../../../../", "/usr/lib"]);
    }

    #[test]
    fn pyannote_runtime_arch_matches_host_accepts_current_and_legacy_labels() {
        assert!(pyannote_runtime_arch_matches_host(target_triple_suffix()));
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            assert!(pyannote_runtime_arch_matches_host("macos-aarch64"));
        }
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        {
            assert!(pyannote_runtime_arch_matches_host("macos-x86_64"));
        }
    }
}
