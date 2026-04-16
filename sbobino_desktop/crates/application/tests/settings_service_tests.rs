use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use sbobino_application::{ApplicationError, SettingsRepository, SettingsService};
use sbobino_domain::{
    AiProvider, AiSettings, AppSettings, LanguageCode, RemoteServiceConfig, RemoteServiceKind,
    SpeechModel,
};

#[derive(Default)]
struct InMemorySettingsRepository {
    settings: Mutex<AppSettings>,
    save_calls: Mutex<usize>,
}

impl InMemorySettingsRepository {
    fn new(settings: AppSettings) -> Self {
        Self {
            settings: Mutex::new(settings),
            save_calls: Mutex::new(0),
        }
    }
}

#[async_trait]
impl SettingsRepository for InMemorySettingsRepository {
    async fn load(&self) -> Result<AppSettings, ApplicationError> {
        Ok(self
            .settings
            .lock()
            .expect("settings lock poisoned")
            .clone())
    }

    async fn save(&self, settings: &AppSettings) -> Result<(), ApplicationError> {
        *self.settings.lock().expect("settings lock poisoned") = settings.clone();
        let mut save_calls = self.save_calls.lock().expect("save_calls lock poisoned");
        *save_calls += 1;
        Ok(())
    }
}

#[tokio::test]
async fn get_returns_current_settings_from_repository() {
    let repo = Arc::new(InMemorySettingsRepository::new(AppSettings {
        language: LanguageCode::It,
        model: SpeechModel::Small,
        ai_post_processing: true,
        ..AppSettings::default()
    }));

    let service = SettingsService::new(repo);
    let loaded = service.get().await.expect("get settings should succeed");

    assert_eq!(loaded.language, LanguageCode::It);
    assert_eq!(loaded.model, SpeechModel::Small);
    assert!(loaded.ai_post_processing);
}

#[tokio::test]
async fn update_persists_and_returns_saved_settings() {
    let repo = Arc::new(InMemorySettingsRepository::new(AppSettings::default()));
    let service = SettingsService::new(repo.clone());

    let updated = service
        .update(AppSettings {
            language: LanguageCode::En,
            model: SpeechModel::Medium,
            ai_post_processing: true,
            gemini_model: "gemini-2.5-pro".to_string(),
            ..AppSettings::default()
        })
        .await
        .expect("update should succeed");

    assert_eq!(updated.language, LanguageCode::En);
    assert_eq!(updated.model, SpeechModel::Medium);
    assert!(updated.ai_post_processing);
    assert_eq!(updated.gemini_model, "gemini-2.5-pro");
    assert_eq!(
        *repo.save_calls.lock().expect("save_calls lock poisoned"),
        1
    );
}

#[tokio::test]
async fn update_partial_persists_ai_remote_services() {
    let repo = Arc::new(InMemorySettingsRepository::new(AppSettings::default()));
    let service = SettingsService::new(repo.clone());

    let mut ai = AppSettings::default().ai;
    ai.active_provider = AiProvider::Gemini;
    ai.remote_services = vec![
        RemoteServiceConfig {
            id: "google_1".to_string(),
            kind: RemoteServiceKind::Google,
            label: "Google".to_string(),
            enabled: true,
            api_key: Some("AIza-test-key".to_string()),
            has_api_key: true,
            model: Some("gemini-2.5-flash".to_string()),
            base_url: Some("https://generativelanguage.googleapis.com/v1beta".to_string()),
        },
        RemoteServiceConfig {
            id: "openai_1".to_string(),
            kind: RemoteServiceKind::OpenAi,
            label: "OpenAI".to_string(),
            enabled: true,
            api_key: Some("sk-test".to_string()),
            has_api_key: true,
            model: Some("gpt-4.1-mini".to_string()),
            base_url: Some("https://api.openai.com/v1".to_string()),
        },
    ];

    let updated = service
        .update_partial(None, None, Some(ai), None)
        .await
        .expect("update_partial should succeed");

    assert_eq!(updated.ai.active_provider, AiProvider::Gemini);
    assert_eq!(updated.ai.remote_services.len(), 2);
    assert_eq!(
        updated.ai.remote_services[0].kind,
        RemoteServiceKind::Google
    );
    assert_eq!(
        updated.ai.remote_services[1].kind,
        RemoteServiceKind::OpenAi
    );
    assert_eq!(
        *repo.save_calls.lock().expect("save_calls lock poisoned"),
        1
    );
}

#[tokio::test]
async fn update_partial_accepts_all_remote_service_kinds() {
    let repo = Arc::new(InMemorySettingsRepository::new(AppSettings::default()));
    let service = SettingsService::new(repo);

    let kinds = [
        RemoteServiceKind::Google,
        RemoteServiceKind::OpenAi,
        RemoteServiceKind::Anthropic,
        RemoteServiceKind::Azure,
        RemoteServiceKind::LmStudio,
        RemoteServiceKind::Ollama,
        RemoteServiceKind::OpenRouter,
        RemoteServiceKind::Xai,
        RemoteServiceKind::HuggingFace,
        RemoteServiceKind::Custom,
    ];

    let ai = AiSettings {
        active_provider: AiProvider::None,
        remote_services: kinds
            .iter()
            .enumerate()
            .map(|(index, kind)| RemoteServiceConfig {
                id: format!("svc_{index}"),
                kind: kind.clone(),
                label: format!("{kind:?}"),
                enabled: true,
                api_key: None,
                has_api_key: false,
                model: None,
                base_url: None,
            })
            .collect(),
        ..AiSettings::default()
    };

    let updated = service
        .update_partial(None, None, Some(ai), None)
        .await
        .expect("all remote service kinds should persist");

    assert_eq!(updated.ai.remote_services.len(), kinds.len());
    for expected in kinds {
        assert!(updated
            .ai
            .remote_services
            .iter()
            .any(|service| service.kind == expected));
    }
}
