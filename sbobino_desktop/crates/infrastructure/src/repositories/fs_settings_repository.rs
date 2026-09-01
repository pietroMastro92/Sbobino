use std::{fs, io::Write, path::PathBuf};

use async_trait::async_trait;
use serde_json::json;

use sbobino_application::{ApplicationError, SettingsRepository};
use sbobino_domain::{AppSettings, ParakeetModel};

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
        Self {
            path,
            secure_storage,
        }
    }

    pub fn load_sync(&self) -> Result<AppSettings, ApplicationError> {
        if !self.path.exists() {
            let defaults = AppSettings::default();
            self.write_settings_file(&defaults)?;
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
        let has_automation = raw_json
            .get("automation")
            .is_some_and(|value| value.is_object());
        let has_organization = raw_json
            .get("organization")
            .is_some_and(|value| value.is_object());
        let has_prompts = raw_json
            .get("prompts")
            .is_some_and(|value| value.is_object());

        if has_general
            && has_transcription
            && has_automation
            && has_organization
            && has_ai
            && has_prompts
        {
            settings.sync_legacy_from_sections();
        } else {
            settings.sync_sections_from_legacy();
        }
        backfill_automatic_import_source_transcription_defaults(&mut settings, &raw_json);

        let migrated_file_model = !matches!(
            settings.transcription.parakeet_model,
            ParakeetModel::Tdt06bV3F16 | ParakeetModel::Tdt06bV3Q8 | ParakeetModel::Tdt06bV3Q4
        );
        if migrated_file_model {
            settings.transcription.parakeet_model = ParakeetModel::Tdt06bV3Q4;
        }

        let plaintext_secrets_found = self.populate_secrets(&mut settings)?;
        if plaintext_secrets_found || migrated_file_model {
            // Legacy releases stored API keys in settings.json.  Move them to
            // secure storage and immediately rewrite the file redacted so a
            // successful migration does not leave plaintext behind. Redact
            // even when secure storage already contains a newer key.
            self.write_settings_file(&settings)?;
        }

        Ok(settings)
    }

    pub fn save_sync(&self, settings: &AppSettings) -> Result<(), ApplicationError> {
        let previous_remote_service_ids = self.stored_remote_service_ids().unwrap_or_default();
        let mut normalized = settings.clone();
        if !matches!(
            normalized.transcription.parakeet_model,
            ParakeetModel::Tdt06bV3F16 | ParakeetModel::Tdt06bV3Q8 | ParakeetModel::Tdt06bV3Q4
        ) {
            normalized.transcription.parakeet_model = ParakeetModel::Tdt06bV3Q4;
        }
        self.merge_stored_secrets_for_save(&mut normalized)?;
        if should_treat_legacy_fields_as_source(&normalized) {
            normalized.sync_sections_from_legacy();
        }
        normalized.sync_legacy_from_sections();
        normalized.refresh_secret_presence_flags();

        self.persist_secrets(&normalized)?;

        let current_remote_service_ids = normalized
            .ai
            .remote_services
            .iter()
            .map(|service| service.id.as_str())
            .collect::<std::collections::HashSet<_>>();
        for removed_id in previous_remote_service_ids {
            if !current_remote_service_ids.contains(removed_id.as_str()) {
                self.secure_storage
                    .delete_secret(&format!("remote_service.{removed_id}.api_key"))?;
            }
        }

        self.write_settings_file(&normalized)
    }

    fn write_settings_file(&self, settings: &AppSettings) -> Result<(), ApplicationError> {
        let mut file_settings = settings.redacted_clone();
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

        let parent = self
            .path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        let mut staged = tempfile::Builder::new()
            .prefix(".sbobino-settings-")
            .tempfile_in(parent)
            .map_err(|e| {
                ApplicationError::Settings(format!(
                    "failed to stage settings file in {}: {e}",
                    parent.display()
                ))
            })?;
        staged.write_all(serialized.as_bytes()).map_err(|e| {
            ApplicationError::Settings(format!("failed to write staged settings file: {e}"))
        })?;
        staged.flush().map_err(|e| {
            ApplicationError::Settings(format!("failed to flush staged settings file: {e}"))
        })?;
        staged.as_file().sync_all().map_err(|e| {
            ApplicationError::Settings(format!("failed to sync staged settings file: {e}"))
        })?;
        staged.persist(&self.path).map_err(|e| {
            ApplicationError::Settings(format!(
                "failed to atomically replace settings file {}: {}",
                self.path.display(),
                e.error
            ))
        })?;
        Ok(())
    }

    fn merge_stored_secrets_for_save(
        &self,
        settings: &mut AppSettings,
    ) -> Result<(), ApplicationError> {
        if settings.ai.providers.gemini.api_key.is_none()
            && settings.ai.providers.gemini.has_api_key
        {
            settings.ai.providers.gemini.api_key =
                self.secure_storage.read_secret("settings.gemini_api_key")?;
        }
        if settings.gemini_api_key.is_none() && settings.gemini_api_key_present {
            settings.gemini_api_key = settings
                .ai
                .providers
                .gemini
                .api_key
                .clone()
                .or(self.secure_storage.read_secret("settings.gemini_api_key")?);
        }

        for service in &mut settings.ai.remote_services {
            if service.api_key.is_none() && service.has_api_key {
                let account = format!("remote_service.{}.api_key", service.id);
                service.api_key = self.secure_storage.read_secret(&account)?;
            }
        }

        Ok(())
    }

    fn populate_secrets(&self, settings: &mut AppSettings) -> Result<bool, ApplicationError> {
        let mut plaintext_secrets_found = false;
        let gemini_account = "settings.gemini_api_key";
        let legacy_gemini_key = settings
            .ai
            .providers
            .gemini
            .api_key
            .clone()
            .or_else(|| settings.gemini_api_key.clone())
            .and_then(normalize_secret);
        plaintext_secrets_found |= legacy_gemini_key.is_some();
        let secure_gemini_key = self.secure_storage.read_secret(gemini_account)?;
        let gemini_key = secure_gemini_key.clone().or(legacy_gemini_key.clone());
        if secure_gemini_key.is_none() && legacy_gemini_key.is_some() {
            self.secure_storage.write_secret(
                gemini_account,
                legacy_gemini_key.as_deref().expect("checked above"),
            )?;
        }
        settings.ai.providers.gemini.api_key = gemini_key.clone();
        settings.gemini_api_key = gemini_key.clone();
        settings.ai.providers.gemini.has_api_key = gemini_key.is_some();
        settings.gemini_api_key_present = gemini_key.is_some();

        for service in &mut settings.ai.remote_services {
            let account = format!("remote_service.{}.api_key", service.id);
            let legacy_key = service.api_key.clone().and_then(normalize_secret);
            plaintext_secrets_found |= legacy_key.is_some();
            let secure_key = self.secure_storage.read_secret(&account)?;
            let key = secure_key.clone().or(legacy_key.clone());
            if secure_key.is_none() && legacy_key.is_some() {
                self.secure_storage
                    .write_secret(&account, legacy_key.as_deref().expect("checked above"))?;
            }
            service.api_key = key.clone();
            service.has_api_key = key.is_some();
        }

        Ok(plaintext_secrets_found)
    }

    fn stored_remote_service_ids(&self) -> Result<Vec<String>, ApplicationError> {
        if !self.path.exists() {
            return Ok(Vec::new());
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
        Ok(raw_json
            .get("ai")
            .and_then(|ai| ai.get("remote_services"))
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|service| service.get("id").and_then(serde_json::Value::as_str))
            .map(str::to_string)
            .filter(|id| !id.trim().is_empty())
            .collect())
    }

    fn persist_secrets(&self, settings: &AppSettings) -> Result<(), ApplicationError> {
        match settings.ai.providers.gemini.api_key.as_deref() {
            Some(secret) if !secret.trim().is_empty() => {
                self.secure_storage
                    .write_secret("settings.gemini_api_key", secret.trim())?;
            }
            _ if !settings.ai.providers.gemini.has_api_key => {
                self.secure_storage
                    .delete_secret("settings.gemini_api_key")?;
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
        "automation": &settings.automation,
        "organization": &settings.organization,
        "ai": &settings.ai,
        "prompts": &settings.prompts,
    }) == json!({
        "general": {
            "auto_update_enabled": defaults.general.auto_update_enabled,
            "auto_update_repo": &defaults.general.auto_update_repo,
        },
        "transcription": &defaults.transcription,
        "automation": &defaults.automation,
        "organization": &defaults.organization,
        "ai": &defaults.ai,
        "prompts": &defaults.prompts,
    });

    legacy_differs && sections_match_defaults
}

fn normalize_secret(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn backfill_automatic_import_source_transcription_defaults(
    settings: &mut AppSettings,
    raw_json: &serde_json::Value,
) {
    let Some(raw_sources) = raw_json
        .get("automation")
        .and_then(|automation| automation.get("watched_sources"))
        .and_then(|sources| sources.as_array())
    else {
        return;
    };

    for (index, source) in settings.automation.watched_sources.iter_mut().enumerate() {
        let Some(raw_source) = raw_sources.get(index).and_then(|value| value.as_object()) else {
            continue;
        };
        if !raw_source.contains_key("model") {
            source.model = settings.transcription.model.clone();
        }
        if !raw_source.contains_key("language") {
            source.language = settings.transcription.language.clone();
        }
    }
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
