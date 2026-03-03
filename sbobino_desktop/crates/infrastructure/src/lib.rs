pub mod adapters;
pub mod repositories;

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

#[derive(Debug, Clone)]
pub struct RuntimeHealth {
    pub host_os: String,
    pub host_arch: String,
    pub is_apple_silicon: bool,
    pub preferred_engine: TranscriptionEngine,
    pub configured_engine: TranscriptionEngine,
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
}

#[derive(Debug, Clone)]
struct BinaryResolution {
    resolved_path: String,
}

impl RuntimeTranscriptionFactory {
    pub fn new(data_dir: &Path) -> Result<Self, String> {
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
        })
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

        let enhancer = self
            .build_active_enhancer()
            .map_err(|error| format!("failed to build AI enhancer: {error}"))?
            .unwrap_or_else(|| Arc::new(NoopEnhancer));

        Ok(Arc::new(TranscriptionService::new(
            transcoder,
            speech_engine,
            enhancer,
            self.artifacts_repo.clone(),
        )))
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
        if !settings.ai.providers.foundation_apple.enabled {
            return Ok(None);
        }

        Ok(Some(FoundationAppleEnhancer::new(
            optimize_prompt_override.or_else(|| settings.prompt_for_task(PromptTask::Optimize)),
            summary_prompt_override.or_else(|| settings.prompt_for_task(PromptTask::Summary)),
        )))
    }

    pub fn build_active_enhancer(&self) -> Result<Option<Arc<dyn TranscriptEnhancer>>, String> {
        let settings = self.load_settings()?;
        if settings.ai.active_provider == AiProvider::FoundationApple {
            let enhancer = self.build_foundation_enhancer_with_overrides(
                settings.prompt_for_task(PromptTask::Optimize),
                settings.prompt_for_task(PromptTask::Summary),
            )?;
            if enhancer.is_some() {
                return Ok(enhancer.map(|value| Arc::new(value) as Arc<dyn TranscriptEnhancer>));
            }
        }

        if let Some(active_id) = settings.ai.active_remote_service_id.as_ref() {
            if let Some(enhancer) = self.build_remote_service_enhancer(&settings, active_id)? {
                let enhancer: Arc<dyn TranscriptEnhancer> = enhancer.into();
                return Ok(Some(enhancer));
            }
            return Err(format!(
                "Active AI service '{active_id}' is missing or disabled. Reconfigure it in Settings > AI Services."
            ));
        }

        match settings.ai.active_provider {
            AiProvider::Gemini => {
                let enhancer = self.build_gemini_enhancer_with_overrides(
                    None,
                    settings.prompt_for_task(PromptTask::Optimize),
                    settings.prompt_for_task(PromptTask::Summary),
                )?;
                Ok(enhancer.map(|value| Arc::new(value) as Arc<dyn TranscriptEnhancer>))
            }
            AiProvider::FoundationApple | AiProvider::None => Ok(None),
        }
    }

    fn build_remote_service_enhancer(
        &self,
        settings: &AppSettings,
        active_id: &str,
    ) -> Result<Option<Box<dyn TranscriptEnhancer>>, String> {
        let Some(service) = settings
            .ai
            .remote_services
            .iter()
            .find(|entry| entry.id == active_id && entry.enabled)
        else {
            return Ok(None);
        };

        if service.kind == RemoteServiceKind::Google {
            let enhancer = self.build_gemini_for_service(settings, service)?;
            return Ok(enhancer.map(|value| Box::new(value) as Box<dyn TranscriptEnhancer>));
        }

        let enhancer = self.build_openai_compatible_for_service(settings, service)?;
        Ok(enhancer.map(|value| Box::new(value) as Box<dyn TranscriptEnhancer>))
    }

    fn build_gemini_for_service(
        &self,
        settings: &AppSettings,
        service: &RemoteServiceConfig,
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

        let model = service
            .model
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| settings.ai.providers.gemini.model.clone());

        Ok(Some(GeminiEnhancer::new(
            api_key,
            model,
            settings.prompt_for_task(PromptTask::Optimize),
            settings.prompt_for_task(PromptTask::Summary),
        )))
    }

    fn build_openai_compatible_for_service(
        &self,
        settings: &AppSettings,
        service: &RemoteServiceConfig,
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

        let Some(model) = service
            .model
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
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
            settings.prompt_for_task(PromptTask::Optimize),
            settings.prompt_for_task(PromptTask::Summary),
        )
        .map_err(|error| format!("{error}"))?;
        Ok(Some(enhancer))
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

        Ok(RuntimeHealth {
            host_os: env::consts::OS.to_string(),
            host_arch: env::consts::ARCH.to_string(),
            is_apple_silicon: is_apple_silicon_host(),
            preferred_engine: preferred_transcription_engine(),
            configured_engine: settings.transcription.engine.clone(),
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
            // Tauri sidecar: binaries are placed next to the app executable
            if let Ok(exe) = std::env::current_exe() {
                if let Some(exe_dir) = exe.parent() {
                    for name in &names {
                        candidates.push(exe_dir.join(name));
                    }
                }
            }
            // Dev fallback: resolve sidecar wrappers directly from src-tauri/binaries
            // so local runs work without requiring global CLI installations.
            let dev_sidecar_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../apps/desktop/src-tauri/binaries");
            for name in &names {
                candidates.push(dev_sidecar_dir.join(name));
            }
            for name in &names {
                candidates.push(self.data_dir.join("bin").join(name));
                candidates.push(self.data_dir.join(name));
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

#[derive(Clone)]
pub struct InfrastructureBundle {
    pub artifact_service: Arc<ArtifactService>,
    pub settings_service: Arc<SettingsService>,
    pub runtime_factory: Arc<RuntimeTranscriptionFactory>,
}

pub fn bootstrap(data_dir: &Path) -> Result<InfrastructureBundle, String> {
    let runtime_factory = Arc::new(RuntimeTranscriptionFactory::new(data_dir)?);
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
