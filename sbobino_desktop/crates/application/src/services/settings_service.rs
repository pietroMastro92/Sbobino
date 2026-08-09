use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use sbobino_domain::{
    AiProvider, AiSettings, AppSettings, AutomaticImportSettings, GeneralSettings,
    OrganizationSettings, PromptSettings, PromptTask, PromptTemplate, TranscriptionSettings,
};

use crate::{ApplicationError, SettingsRepository};

#[derive(Clone)]
pub struct SettingsService {
    settings_repo: Arc<dyn SettingsRepository>,
    /// Serializes every load-modify-save transaction owned by this service.
    ///
    /// Settings are also read by the UI while a mutation is in flight.  The
    /// lock deliberately covers the load as well as the save so two callers
    /// cannot both load the same snapshot and then overwrite one another.
    write_lock: Arc<tokio::sync::Mutex<()>>,
}

impl SettingsService {
    pub fn new(settings_repo: Arc<dyn SettingsRepository>) -> Self {
        Self {
            settings_repo,
            write_lock: Arc::new(tokio::sync::Mutex::new(())),
        }
    }

    pub async fn get(&self) -> Result<AppSettings, ApplicationError> {
        self.snapshot().await
    }

    pub async fn update(&self, settings: AppSettings) -> Result<AppSettings, ApplicationError> {
        let _write_guard = self.write_lock.lock().await;
        self.persist_with_source(settings, SettingsSyncSource::Legacy)
            .await
    }

    pub async fn snapshot(&self) -> Result<AppSettings, ApplicationError> {
        self.load_synced().await
    }

    pub async fn update_partial(
        &self,
        general: Option<GeneralSettings>,
        transcription: Option<TranscriptionSettings>,
        automation: Option<AutomaticImportSettings>,
        organization: Option<OrganizationSettings>,
        ai: Option<AiSettings>,
        prompts: Option<PromptSettings>,
    ) -> Result<AppSettings, ApplicationError> {
        let _write_guard = self.write_lock.lock().await;
        let mut settings = self.load_synced().await?;
        if let Some(value) = general {
            settings.general = value;
        }
        if let Some(value) = transcription {
            settings.transcription = value;
        }
        if let Some(value) = automation {
            settings.automation = value;
        }
        if let Some(value) = organization {
            settings.organization = value;
        }
        if let Some(value) = ai {
            settings.ai = value;
        }
        if let Some(value) = prompts {
            settings.prompts = value;
        }

        self.persist_with_source(settings, SettingsSyncSource::Sections)
            .await
    }

    pub async fn ai_settings(&self) -> Result<AiSettings, ApplicationError> {
        Ok(self.load_synced().await?.ai)
    }

    pub async fn update_ai_settings(
        &self,
        active_provider: Option<AiProvider>,
        foundation_apple_enabled: Option<bool>,
        gemini_api_key: Option<Option<String>>,
        gemini_model: Option<String>,
    ) -> Result<AiSettings, ApplicationError> {
        let _write_guard = self.write_lock.lock().await;
        let mut settings = self.load_synced().await?;

        if let Some(value) = active_provider {
            settings.ai.active_provider = value;
        }
        if let Some(value) = foundation_apple_enabled {
            settings.ai.providers.foundation_apple.enabled = value;
        }
        if let Some(value) = gemini_api_key {
            settings.ai.providers.gemini.api_key = normalize_optional_string(value);
        }
        if let Some(value) = gemini_model {
            settings.ai.providers.gemini.model =
                normalize_optional_string(Some(value)).unwrap_or_else(default_gemini_model);
        }

        let settings = self
            .persist_with_source(settings, SettingsSyncSource::Sections)
            .await?;

        Ok(settings.ai)
    }

    pub async fn list_prompts(&self) -> Result<Vec<PromptTemplate>, ApplicationError> {
        Ok(self.load_synced().await?.prompts.templates)
    }

    pub async fn save_prompt(
        &self,
        mut template: PromptTemplate,
        bind_task: Option<PromptTask>,
    ) -> Result<AppSettings, ApplicationError> {
        let _write_guard = self.write_lock.lock().await;
        template.id = normalize_optional_string(Some(template.id.clone())).unwrap_or_else(|| {
            format!(
                "custom_{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
            )
        });
        template.name =
            normalize_optional_string(Some(template.name.clone())).ok_or_else(|| {
                ApplicationError::Validation("prompt template name cannot be empty".to_string())
            })?;
        template.body =
            normalize_optional_string(Some(template.body.clone())).ok_or_else(|| {
                ApplicationError::Validation("prompt template body cannot be empty".to_string())
            })?;
        if template.updated_at.trim().is_empty() {
            template.updated_at = format!(
                "{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            );
        }

        let mut settings = self.load_synced().await?;
        if let Some(existing) = settings
            .prompts
            .templates
            .iter_mut()
            .find(|item| item.id == template.id)
        {
            *existing = template.clone();
        } else {
            settings.prompts.templates.push(template.clone());
        }

        if let Some(task) = bind_task {
            set_prompt_binding(&mut settings, task, &template.id);
        }

        self.persist_with_source(settings, SettingsSyncSource::Sections)
            .await
    }

    pub async fn delete_prompt(&self, prompt_id: String) -> Result<AppSettings, ApplicationError> {
        let _write_guard = self.write_lock.lock().await;
        let Some(prompt_id) = normalize_optional_string(Some(prompt_id)) else {
            return Err(ApplicationError::Validation(
                "prompt template id cannot be empty".to_string(),
            ));
        };

        let mut settings = self.load_synced().await?;
        let Some(index) = settings
            .prompts
            .templates
            .iter()
            .position(|item| item.id == prompt_id)
        else {
            return Err(ApplicationError::Validation(format!(
                "prompt template not found: {prompt_id}"
            )));
        };

        if settings.prompts.templates[index].builtin {
            return Err(ApplicationError::Validation(
                "built-in prompt templates cannot be deleted".to_string(),
            ));
        }

        settings.prompts.templates.remove(index);
        self.persist_with_source(settings, SettingsSyncSource::Sections)
            .await
    }

    pub async fn reset_prompts(&self) -> Result<AppSettings, ApplicationError> {
        let _write_guard = self.write_lock.lock().await;
        let mut settings = self.load_synced().await?;
        settings.prompts = PromptSettings::default();
        self.persist_with_source(settings, SettingsSyncSource::Sections)
            .await
    }

    async fn load_synced(&self) -> Result<AppSettings, ApplicationError> {
        self.settings_repo.load().await
    }

    async fn persist_with_source(
        &self,
        mut settings: AppSettings,
        source: SettingsSyncSource,
    ) -> Result<AppSettings, ApplicationError> {
        match source {
            SettingsSyncSource::Legacy => {
                settings.sync_sections_from_legacy();
                settings.sync_legacy_from_sections();
            }
            SettingsSyncSource::Sections => {
                settings.sync_legacy_from_sections();
                settings.sync_sections_from_legacy();
            }
        }

        self.settings_repo.save(&settings).await?;
        self.load_synced().await
    }
}

#[derive(Debug, Clone, Copy)]
enum SettingsSyncSource {
    Legacy,
    Sections,
}

fn normalize_optional_string(input: Option<String>) -> Option<String> {
    input.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn default_gemini_model() -> String {
    "gemini-2.5-flash-lite".to_string()
}

fn set_prompt_binding(settings: &mut AppSettings, task: PromptTask, prompt_id: &str) {
    match task {
        PromptTask::Optimize => {
            settings.prompts.bindings.optimize_prompt_id = prompt_id.to_string()
        }
        PromptTask::Summary => settings.prompts.bindings.summary_prompt_id = prompt_id.to_string(),
        PromptTask::Faq => settings.prompts.bindings.faq_prompt_id = prompt_id.to_string(),
        PromptTask::EmotionAnalysis => {
            settings.prompts.bindings.emotion_prompt_id = prompt_id.to_string()
        }
    }
}
