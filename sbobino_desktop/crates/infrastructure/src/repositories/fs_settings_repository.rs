use std::{fs, path::PathBuf};

use async_trait::async_trait;
use serde_json::json;

use sbobino_application::{ApplicationError, SettingsRepository};
use sbobino_domain::AppSettings;

use crate::secure_storage::SecureStorage;

#[derive(Debug, Clone)]
pub struct FsSettingsRepository {
    path: PathBuf,
    secure_storage: SecureStorage,
}

impl FsSettingsRepository {
    pub fn new(path: PathBuf) -> Self {
        let fallback_root = path
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let secure_storage = SecureStorage::load_or_create_with_fallback(&fallback_root)
            .expect("secure storage should initialize before settings repository");
        Self { path, secure_storage }
    }

    pub fn load_sync(&self) -> Result<AppSettings, ApplicationError> {
        if !self.path.exists() {
            let defaults = AppSettings::default();
            let serialized = serde_json::to_string_pretty(&defaults).map_err(|e| {
                ApplicationError::Settings(format!("failed to serialize default settings: {e}"))
            })?;
            if let Some(parent) = self.path.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    ApplicationError::Settings(format!(
                        "failed to create settings directory {}: {e}",
                        parent.display()
                    ))
                })?;
            }
            fs::write(&self.path, serialized).map_err(|e| {
                ApplicationError::Settings(format!(
                    "failed to write settings file {}: {e}",
                    self.path.display()
                ))
            })?;
            return Ok(defaults);
        }

        let content = fs::read_to_string(&self.path).map_err(|e| {
            ApplicationError::Settings(format!(
                "failed to read settings file {}: {e}",
                self.path.display()
            ))
        })?;

        let raw_json = serde_json::from_str::<serde_json::Value>(&content).map_err(|e| {
            ApplicationError::Settings(format!(
                "invalid settings JSON in {}: {e}",
                self.path.display()
            ))
        })?;

        let mut settings =
            serde_json::from_value::<AppSettings>(raw_json.clone()).map_err(|e| {
                ApplicationError::Settings(format!(
                    "invalid settings JSON in {}: {e}",
                    self.path.display()
                ))
            })?;

        // Migrate legacy defaults from Python bundle assumptions to runtime-friendly defaults.
        if settings.ffmpeg_path == "resources/ffmpeg_bin/ffmpeg" {
            settings.ffmpeg_path = "ffmpeg".to_string();
        }
        if settings.whisper_cli_path == "whisper.cpp/build/bin/whisper-cli" {
            settings.whisper_cli_path = "whisper-cli".to_string();
        }

        let has_general = raw_json
            .get("general")
            .is_some_and(|value| value.is_object());
        let has_transcription = raw_json
            .get("transcription")
            .is_some_and(|value| value.is_object());
        let has_ai = raw_json.get("ai").is_some_and(|value| value.is_object());
        let has_prompts = raw_json
            .get("prompts")
            .is_some_and(|value| value.is_object());

        if has_general && has_transcription && has_ai && has_prompts {
            settings.sync_legacy_from_sections();
        } else {
            settings.sync_sections_from_legacy();
        }

        self.populate_secrets(&mut settings)?;

        Ok(settings)
    }

    pub fn save_sync(&self, settings: &AppSettings) -> Result<(), ApplicationError> {
        let mut normalized = settings.clone();
        self.populate_secrets(&mut normalized)?;
        if should_treat_legacy_fields_as_source(&normalized) {
            normalized.sync_sections_from_legacy();
        }
        normalized.sync_legacy_from_sections();
        normalized.refresh_secret_presence_flags();

        self.persist_secrets(&normalized)?;

        let mut file_settings = normalized.redacted_clone();
        file_settings.refresh_secret_presence_flags();

        let serialized = serde_json::to_string_pretty(&file_settings).map_err(|e| {
            ApplicationError::Settings(format!("failed to serialize settings: {e}"))
        })?;

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                ApplicationError::Settings(format!(
                    "failed to create settings directory {}: {e}",
                    parent.display()
                ))
            })?;
        }

        fs::write(&self.path, serialized).map_err(|e| {
            ApplicationError::Settings(format!(
                "failed to save settings file {}: {e}",
                self.path.display()
            ))
        })
    }

    fn populate_secrets(&self, settings: &mut AppSettings) -> Result<(), ApplicationError> {
        let gemini_account = "settings.gemini_api_key";
        settings.ai.providers.gemini.api_key = self.secure_storage.read_secret(gemini_account)?;
        settings.gemini_api_key = settings.ai.providers.gemini.api_key.clone();
        settings.ai.providers.gemini.has_api_key = settings.ai.providers.gemini.api_key.is_some();
        settings.gemini_api_key_present = settings.ai.providers.gemini.has_api_key;

        for service in &mut settings.ai.remote_services {
            let account = format!("remote_service.{}.api_key", service.id);
            service.api_key = self.secure_storage.read_secret(&account)?;
            service.has_api_key = service.api_key.is_some();
        }

        Ok(())
    }

    fn persist_secrets(&self, settings: &AppSettings) -> Result<(), ApplicationError> {
        match settings.ai.providers.gemini.api_key.as_deref() {
            Some(secret) if !secret.trim().is_empty() => {
                self.secure_storage
                    .write_secret("settings.gemini_api_key", secret.trim())?;
            }
            _ if !settings.ai.providers.gemini.has_api_key => {
                self.secure_storage.delete_secret("settings.gemini_api_key")?;
            }
            _ => {}
        }

        for service in &settings.ai.remote_services {
            let account = format!("remote_service.{}.api_key", service.id);
            match service.api_key.as_deref() {
                Some(secret) if !secret.trim().is_empty() => {
                    self.secure_storage.write_secret(&account, secret.trim())?;
                }
                _ if !service.has_api_key => {
                    self.secure_storage.delete_secret(&account)?;
                }
                _ => {}
            }
        }

        Ok(())
    }
}

fn should_treat_legacy_fields_as_source(settings: &AppSettings) -> bool {
    let defaults = AppSettings::default();

    let legacy_differs = settings.transcription_engine != defaults.transcription_engine
        || settings.model != defaults.model
        || settings.language != defaults.language
        || settings.ai_post_processing != defaults.ai_post_processing
        || settings.gemini_model != defaults.gemini_model
        || settings.gemini_api_key != defaults.gemini_api_key
        || settings.whisper_cli_path != defaults.whisper_cli_path
        || settings.whisperkit_cli_path != defaults.whisperkit_cli_path
        || settings.ffmpeg_path != defaults.ffmpeg_path
        || settings.models_dir != defaults.models_dir
        || settings.auto_update_enabled != defaults.auto_update_enabled
        || settings.auto_update_repo != defaults.auto_update_repo;

    let sections_match_defaults = json!({
        "general": {
            "auto_update_enabled": settings.general.auto_update_enabled,
            "auto_update_repo": &settings.general.auto_update_repo,
        },
        "transcription": &settings.transcription,
        "ai": &settings.ai,
        "prompts": &settings.prompts,
    }) == json!({
        "general": {
            "auto_update_enabled": defaults.general.auto_update_enabled,
            "auto_update_repo": &defaults.general.auto_update_repo,
        },
        "transcription": &defaults.transcription,
        "ai": &defaults.ai,
        "prompts": &defaults.prompts,
    });

    legacy_differs && sections_match_defaults
}

#[async_trait]
impl SettingsRepository for FsSettingsRepository {
    async fn load(&self) -> Result<AppSettings, ApplicationError> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            let repo = FsSettingsRepository::new(path);
            repo.load_sync()
        })
        .await
        .map_err(|e| ApplicationError::Settings(format!("settings load join error: {e}")))?
    }

    async fn save(&self, settings: &AppSettings) -> Result<(), ApplicationError> {
        let path = self.path.clone();
        let settings = settings.clone();
        tokio::task::spawn_blocking(move || {
            let repo = FsSettingsRepository::new(path);
            repo.save_sync(&settings)
        })
        .await
        .map_err(|e| ApplicationError::Settings(format!("settings save join error: {e}")))?
    }
}
