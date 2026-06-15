use std::fs;

use tempfile::tempdir;

use sbobino_application::SettingsRepository;
use sbobino_domain::{LanguageCode, SpeechModel};
use sbobino_infrastructure::repositories::fs_settings_repository::FsSettingsRepository;

fn enable_local_secure_storage_for_tests() {
    std::env::set_var("SBOBINO_ALLOW_INSECURE_LOCAL_SECRETS", "1");
}

#[tokio::test]
async fn load_creates_default_settings_when_file_is_missing() {
    enable_local_secure_storage_for_tests();
    let temp = tempdir().expect("failed to create temp dir");
    let settings_path = temp.path().join("config").join("settings.json");
    let repo = FsSettingsRepository::new(settings_path.clone());

    let settings = repo.load().await.expect("load should create defaults");

    assert!(settings_path.exists(), "settings file should be created");
    assert_eq!(settings.model, SpeechModel::Base);
    assert_eq!(settings.language, LanguageCode::Auto);
    assert!(!settings.ai_post_processing);
    assert_eq!(settings.general.auto_update_repo, "pietroMastro92/Sbobino");
    assert!(settings.general.privacy_policy_version_accepted.is_none());
    assert!(settings.general.privacy_policy_accepted_at.is_none());
}

#[tokio::test]
async fn save_then_load_round_trips_settings() {
    enable_local_secure_storage_for_tests();
    let temp = tempdir().expect("failed to create temp dir");
    let settings_path = temp.path().join("settings.json");
    let repo = FsSettingsRepository::new(settings_path);

    let mut settings = repo.load().await.expect("initial load should succeed");
    settings.model = SpeechModel::LargeTurbo;
    settings.language = LanguageCode::Fr;
    settings.ai_post_processing = true;
    settings.gemini_model = "gemini-2.5-pro".to_string();
    settings.general.privacy_policy_version_accepted = Some("2026-04-03".to_string());
    settings.general.privacy_policy_accepted_at = Some("2026-04-03T12:00:00Z".to_string());

    repo.save(&settings).await.expect("save should succeed");
    let loaded = repo.load().await.expect("second load should succeed");

    assert_eq!(loaded.model, SpeechModel::LargeTurbo);
    assert_eq!(loaded.language, LanguageCode::Fr);
    assert!(loaded.ai_post_processing);
    assert_eq!(loaded.gemini_model, "gemini-2.5-pro");
    assert_eq!(
        loaded.general.privacy_policy_version_accepted.as_deref(),
        Some("2026-04-03")
    );
    assert_eq!(
        loaded.general.privacy_policy_accepted_at.as_deref(),
        Some("2026-04-03T12:00:00Z")
    );
}

#[tokio::test]
async fn save_then_load_preserves_structured_transcription_settings() {
    enable_local_secure_storage_for_tests();
    let temp = tempdir().expect("failed to create temp dir");
    let settings_path = temp.path().join("settings.json");
    let repo = FsSettingsRepository::new(settings_path);

    let mut settings = repo.load().await.expect("initial load should succeed");
    settings.transcription.enable_ai_post_processing = true;
    settings.transcription.speaker_diarization.enabled = true;
    settings.transcription.speaker_diarization.device = "mps".to_string();
    settings
        .transcription
        .speaker_diarization
        .speaker_colors
        .insert("speaker_1".to_string(), "#4F7CFF".to_string());

    repo.save(&settings).await.expect("save should succeed");
    let loaded = repo.load().await.expect("second load should succeed");

    assert!(loaded.transcription.enable_ai_post_processing);
    assert!(loaded.ai_post_processing);
    assert!(loaded.transcription.speaker_diarization.enabled);
    assert_eq!(loaded.transcription.speaker_diarization.device, "mps");
    assert_eq!(
        loaded
            .transcription
            .speaker_diarization
            .speaker_colors
            .get("speaker_1")
            .map(String::as_str),
        Some("#4F7CFF")
    );
}

#[tokio::test]
async fn save_then_load_preserves_automatic_import_and_workspace_settings() {
    enable_local_secure_storage_for_tests();
    let temp = tempdir().expect("failed to create temp dir");
    let settings_path = temp.path().join("settings.json");
    let repo = FsSettingsRepository::new(settings_path);

    let mut settings = repo.load().await.expect("initial load should succeed");
    settings.automation.enabled = true;
    settings.automation.run_scan_on_app_start = true;
    settings.automation.scan_interval_minutes = 30;
    settings.automation.allowed_extensions = vec!["m4a".to_string(), "wav".to_string()];
    settings.automation.watched_sources = vec![sbobino_domain::AutomaticImportSource {
        id: "voice_memos".to_string(),
        label: "Voice Memos".to_string(),
        folder_path: "/Users/test/Voice Memos".to_string(),
        enabled: true,
        preset: sbobino_domain::AutomaticImportPreset::VoiceMemo,
        model: sbobino_domain::SpeechModel::LargeTurbo,
        language: sbobino_domain::LanguageCode::It,
        workspace_id: Some("work".to_string()),
        recursive: false,
        enable_ai_post_processing: true,
        post_processing: sbobino_domain::AutomaticImportPostProcessingSettings::default(),
    }];
    settings.organization.workspaces = vec![sbobino_domain::WorkspaceConfig {
        id: "work".to_string(),
        label: "Work".to_string(),
        color: "#1F8A70".to_string(),
    }];

    repo.save(&settings).await.expect("save should succeed");
    let loaded = repo.load().await.expect("second load should succeed");

    assert!(loaded.automation.enabled);
    assert!(loaded.automation.run_scan_on_app_start);
    assert_eq!(loaded.automation.scan_interval_minutes, 30);
    assert_eq!(
        loaded.automation.allowed_extensions,
        vec!["m4a".to_string(), "wav".to_string()]
    );
    assert_eq!(loaded.automation.watched_sources.len(), 1);
    assert_eq!(loaded.automation.watched_sources[0].id, "voice_memos");
    assert_eq!(
        loaded.automation.watched_sources[0].workspace_id.as_deref(),
        Some("work")
    );
    assert_eq!(
        loaded.automation.watched_sources[0].model,
        sbobino_domain::SpeechModel::LargeTurbo
    );
    assert_eq!(
        loaded.automation.watched_sources[0].language,
        sbobino_domain::LanguageCode::It
    );
    assert!(loaded.automation.watched_sources[0].enable_ai_post_processing);
    assert!(
        loaded.automation.watched_sources[0]
            .post_processing
            .generate_summary
    );
    assert_eq!(loaded.organization.workspaces.len(), 1);
    assert_eq!(loaded.organization.workspaces[0].label, "Work");
}

#[tokio::test]
async fn load_backfills_legacy_automatic_import_source_model_and_language() {
    enable_local_secure_storage_for_tests();
    let temp = tempdir().expect("failed to create temp dir");
    let settings_path = temp.path().join("settings.json");
    let mut settings = sbobino_domain::AppSettings::default();
    settings.transcription.model = SpeechModel::LargeTurbo;
    settings.transcription.language = LanguageCode::It;
    settings.sync_legacy_from_sections();
    settings.automation.enabled = true;
    settings.automation.watched_sources = vec![sbobino_domain::AutomaticImportSource {
        id: "legacy_source".to_string(),
        label: "Legacy Source".to_string(),
        folder_path: "/Users/test/Voice Memos".to_string(),
        enabled: true,
        preset: sbobino_domain::AutomaticImportPreset::VoiceMemo,
        model: SpeechModel::Base,
        language: LanguageCode::Auto,
        workspace_id: None,
        recursive: true,
        enable_ai_post_processing: false,
        post_processing: sbobino_domain::AutomaticImportPostProcessingSettings::default(),
    }];

    let mut raw = serde_json::to_value(&settings).expect("settings should serialize");
    raw["automation"]["watched_sources"][0]
        .as_object_mut()
        .expect("source should be object")
        .remove("model");
    raw["automation"]["watched_sources"][0]
        .as_object_mut()
        .expect("source should be object")
        .remove("language");
    fs::write(
        &settings_path,
        serde_json::to_string_pretty(&raw).expect("settings JSON"),
    )
    .expect("write legacy settings");

    let repo = FsSettingsRepository::new(settings_path);
    let loaded = repo.load().await.expect("legacy load should succeed");

    assert_eq!(
        loaded.automation.watched_sources[0].model,
        SpeechModel::LargeTurbo
    );
    assert_eq!(
        loaded.automation.watched_sources[0].language,
        LanguageCode::It
    );
}
