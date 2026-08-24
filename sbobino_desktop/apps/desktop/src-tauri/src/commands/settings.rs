use std::collections::BTreeSet;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};

use sbobino_domain::{
    transcription_language_catalog, AiProvider, AiSettings, AppSettings, AutomaticImportSettings,
    GeneralSettings, LanguageCode, OrganizationSettings, PersonalizationSettings, PromptSettings,
    PromptTask, PromptTemplate, TranscriptionLanguageOption, TranscriptionSettings,
};

use crate::{
    ai_support::{missing_ai_provider_command_error, run_with_enhancer_fallback},
    commands::emotion_analysis::{
        analyze_emotions_with_enhancers, EmotionAnalysisInput, EmotionAnalysisOptions,
    },
    commands::prepared_transcript::PreparedTranscriptContext,
    error::CommandError,
    state::AppState,
};

fn emit_settings_updated(app: &tauri::AppHandle, settings: &AppSettings) {
    let _ = app.emit("settings://updated", settings.redacted_clone());
}

#[derive(Debug, Deserialize, Default)]
pub struct UpdateSettingsPartialPayload {
    pub general: Option<GeneralSettings>,
    pub transcription: Option<TranscriptionSettings>,
    pub automation: Option<AutomaticImportSettings>,
    pub organization: Option<OrganizationSettings>,
    pub ai: Option<AiSettings>,
    pub prompts: Option<PromptSettings>,
    pub personalization: Option<PersonalizationSettings>,
}

#[derive(Debug, Deserialize, Default)]
pub struct UpdateAiProvidersPayload {
    pub active_provider: Option<AiProvider>,
    pub foundation_apple_enabled: Option<bool>,
    pub gemini_api_key: Option<Option<String>>,
    pub gemini_model: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ListGeminiModelsPayload {
    pub api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TestAiServicePayload {
    pub service_id: String,
}

#[derive(Debug, Deserialize)]
pub struct AiServiceIdPayload {
    pub service_id: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TestAiServiceResponse {
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiModelsResponse {
    models: Option<Vec<GeminiModelEntry>>,
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiModelEntry {
    name: Option<String>,
    supported_generation_methods: Option<Vec<String>>,
}

async fn collect_gemini_models_from_endpoint(
    client: &reqwest::Client,
    endpoint: &str,
    api_key: &str,
    collected: &mut BTreeSet<String>,
) -> Result<(), CommandError> {
    let mut page_token: Option<String> = None;

    for _ in 0..25 {
        let mut request = client
            .get(endpoint)
            .query(&[("key", api_key)])
            .query(&[("pageSize", "1000")]);

        if let Some(token) = page_token.as_ref() {
            request = request.query(&[("pageToken", token.as_str())]);
        }

        let response = request
            .send()
            .await
            .map_err(|error| CommandError::new("gemini_models", error.to_string()))?;

        if response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::FORBIDDEN
        {
            return Err(CommandError::new(
                "gemini_models",
                "Gemini API key is invalid or unauthorized",
            ));
        }

        if response.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }

        if !response.status().is_success() {
            return Err(CommandError::new(
                "gemini_models",
                format!(
                    "Gemini models request failed with status {}",
                    response.status()
                ),
            ));
        }

        let payload = response
            .json::<GeminiModelsResponse>()
            .await
            .map_err(|error| CommandError::new("gemini_models", error.to_string()))?;

        for model in payload.models.unwrap_or_default() {
            let supports_generation = model
                .supported_generation_methods
                .as_ref()
                .is_some_and(|methods| methods.iter().any(|method| method == "generateContent"));

            if !supports_generation {
                continue;
            }

            if let Some(name) = model.name {
                let cleaned = name.trim().trim_start_matches("models/").to_string();
                if !cleaned.is_empty() {
                    let _ = collected.insert(cleaned);
                }
            }
        }

        let next = payload
            .next_page_token
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        if next.is_none() {
            break;
        }

        page_token = next;
    }

    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct SavePromptPayload {
    pub template: PromptTemplate,
    pub bind_task: Option<PromptTask>,
}

#[derive(Debug, Deserialize)]
pub struct DeletePromptPayload {
    pub id: String,
}

#[derive(Debug, Deserialize)]
pub struct TestPromptPayload {
    pub input: String,
    pub language: Option<LanguageCode>,
    pub task: Option<PromptTask>,
    pub prompt_override: Option<String>,
    pub model_override: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TestPromptResponse {
    pub output: String,
    pub summary: String,
    pub faqs: String,
    pub model: String,
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, CommandError> {
    state
        .settings_service
        .get()
        .await
        .map(|settings| settings.redacted_clone())
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn list_transcription_languages() -> Result<Vec<TranscriptionLanguageOption>, CommandError>
{
    Ok(transcription_language_catalog())
}

#[tauri::command]
pub async fn update_settings(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<AppSettings, CommandError> {
    let updated = state
        .settings_service
        .update(settings)
        .await
        .map_err(CommandError::from)?;
    emit_settings_updated(&app, &updated);
    Ok(updated.redacted_clone())
}

#[tauri::command]
pub async fn get_settings_snapshot(
    state: State<'_, AppState>,
) -> Result<AppSettings, CommandError> {
    state
        .settings_service
        .snapshot()
        .await
        .map(|settings| settings.redacted_clone())
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn update_settings_partial(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: UpdateSettingsPartialPayload,
) -> Result<AppSettings, CommandError> {
    let updated = state
        .settings_service
        .update_partial_with_personalization(
            payload.general,
            payload.transcription,
            payload.automation,
            payload.organization,
            payload.ai,
            payload.prompts,
            payload.personalization,
        )
        .await
        .map_err(CommandError::from)?;
    emit_settings_updated(&app, &updated);
    Ok(updated.redacted_clone())
}

#[tauri::command]
pub async fn get_ai_providers(state: State<'_, AppState>) -> Result<AiSettings, CommandError> {
    state
        .settings_service
        .ai_settings()
        .await
        .map(|ai| {
            let mut redacted = ai;
            redacted.providers.gemini.has_api_key = redacted.providers.gemini.api_key.is_some();
            redacted.providers.gemini.api_key = None;
            for service in &mut redacted.remote_services {
                service.has_api_key = service.api_key.is_some();
                service.api_key = None;
            }
            redacted
        })
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn update_ai_providers(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: UpdateAiProvidersPayload,
) -> Result<AiSettings, CommandError> {
    let ai = state
        .settings_service
        .update_ai_settings(
            payload.active_provider,
            payload.foundation_apple_enabled,
            payload.gemini_api_key,
            payload.gemini_model,
        )
        .await
        .map_err(CommandError::from)?;

    if let Ok(snapshot) = state.settings_service.snapshot().await {
        emit_settings_updated(&app, &snapshot);
    }

    let mut redacted = ai;
    redacted.providers.gemini.has_api_key = redacted.providers.gemini.api_key.is_some();
    redacted.providers.gemini.api_key = None;
    for service in &mut redacted.remote_services {
        service.has_api_key = service.api_key.is_some();
        service.api_key = None;
    }
    Ok(redacted)
}

#[tauri::command]
pub async fn list_gemini_models(
    state: State<'_, AppState>,
    payload: Option<ListGeminiModelsPayload>,
) -> Result<Vec<String>, CommandError> {
    let settings = state
        .settings_service
        .snapshot()
        .await
        .map_err(CommandError::from)?;

    let api_key = payload
        .and_then(|entry| entry.api_key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            settings
                .ai
                .providers
                .gemini
                .api_key
                .clone()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .ok_or_else(|| CommandError::new("validation", "Gemini API key is required"))?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|error| CommandError::new("gemini_models", error.to_string()))?;

    let mut collected = BTreeSet::new();
    collect_gemini_models_from_endpoint(
        &client,
        "https://generativelanguage.googleapis.com/v1beta/models",
        &api_key,
        &mut collected,
    )
    .await?;

    collect_gemini_models_from_endpoint(
        &client,
        "https://generativelanguage.googleapis.com/v1/models",
        &api_key,
        &mut collected,
    )
    .await?;

    Ok(collected.into_iter().collect())
}

#[tauri::command]
pub async fn list_ai_service_models(
    state: State<'_, AppState>,
    payload: AiServiceIdPayload,
) -> Result<Vec<String>, CommandError> {
    let settings = state
        .settings_service
        .snapshot()
        .await
        .map_err(CommandError::from)?;
    let service = settings
        .ai
        .remote_services
        .iter()
        .find(|service| service.id == payload.service_id)
        .ok_or_else(|| CommandError::new("not_found", "AI service not found"))?;
    if service.kind == sbobino_domain::RemoteServiceKind::Google {
        return Err(CommandError::new(
            "validation",
            "Use the Gemini model catalog for Google services",
        ));
    }
    if matches!(
        service.kind,
        sbobino_domain::RemoteServiceKind::Azure | sbobino_domain::RemoteServiceKind::Custom
    ) {
        return Ok(Vec::new());
    }
    let base_url = draft_or_stored_value(payload.base_url.as_deref(), service.base_url.as_deref())
        .ok_or_else(|| CommandError::new("validation", "AI service base URL is required"))?;
    let base_url = base_url
        .split("/chat/completions")
        .next()
        .unwrap_or(base_url.as_str())
        .trim_end_matches('/');
    let endpoint = format!("{base_url}/models");
    let mut headers = HeaderMap::new();
    if service.kind == sbobino_domain::RemoteServiceKind::Anthropic {
        let key = draft_or_stored_value(payload.api_key.as_deref(), service.api_key.as_deref())
            .ok_or_else(|| CommandError::new("missing_api_key", "Anthropic API key is required"))?;
        headers.insert(
            HeaderName::from_static("x-api-key"),
            HeaderValue::from_str(&key)
                .map_err(|error| CommandError::new("validation", error.to_string()))?,
        );
        headers.insert(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_static("2023-06-01"),
        );
    } else if let Some(key) =
        draft_or_stored_value(payload.api_key.as_deref(), service.api_key.as_deref())
    {
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {key}"))
                .map_err(|error| CommandError::new("validation", error.to_string()))?,
        );
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|error| CommandError::new("ai_models", error.to_string()))?;
    let response = client
        .get(endpoint)
        .headers(headers)
        .send()
        .await
        .map_err(|error| CommandError::new("ai_models", error.to_string()))?;
    if !response.status().is_success() {
        return Err(CommandError::new(
            "ai_models",
            format!(
                "Model catalog request failed with status {}",
                response.status()
            ),
        ));
    }
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|error| CommandError::new("ai_models", error.to_string()))?;
    let mut models = BTreeSet::new();
    if let Some(entries) = body.get("data").and_then(|value| value.as_array()) {
        for entry in entries {
            if let Some(id) = entry.get("id").and_then(|value| value.as_str()) {
                models.insert(id.to_string());
            }
        }
    }
    Ok(models.into_iter().collect())
}

fn draft_or_stored_value(draft: Option<&str>, stored: Option<&str>) -> Option<String> {
    let normalize = |value: &str| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    };
    draft
        .and_then(normalize)
        .or_else(|| stored.and_then(normalize))
}

#[tauri::command]
pub async fn list_prompts(state: State<'_, AppState>) -> Result<Vec<PromptTemplate>, CommandError> {
    state
        .settings_service
        .list_prompts()
        .await
        .map_err(CommandError::from)
}

#[tauri::command]
pub async fn save_prompt(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: SavePromptPayload,
) -> Result<Vec<PromptTemplate>, CommandError> {
    let updated_settings = state
        .settings_service
        .save_prompt(payload.template, payload.bind_task)
        .await
        .map_err(CommandError::from)?;
    emit_settings_updated(&app, &updated_settings);

    Ok(updated_settings.prompts.templates)
}

#[tauri::command]
pub async fn delete_prompt(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    payload: DeletePromptPayload,
) -> Result<Vec<PromptTemplate>, CommandError> {
    let updated_settings = state
        .settings_service
        .delete_prompt(payload.id)
        .await
        .map_err(CommandError::from)?;
    emit_settings_updated(&app, &updated_settings);

    Ok(updated_settings.prompts.templates)
}

#[tauri::command]
pub async fn reset_prompts(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<PromptTemplate>, CommandError> {
    let updated_settings = state
        .settings_service
        .reset_prompts()
        .await
        .map_err(CommandError::from)?;
    emit_settings_updated(&app, &updated_settings);

    Ok(updated_settings.prompts.templates)
}

#[tauri::command]
pub async fn get_ai_capability_status(
    state: State<'_, AppState>,
) -> Result<sbobino_infrastructure::AiCapabilityStatus, CommandError> {
    state
        .runtime_factory
        .ai_capability_status()
        .map_err(|e| CommandError::new("runtime_factory", e))
}

#[tauri::command]
pub async fn test_ai_service(
    state: State<'_, AppState>,
    payload: TestAiServicePayload,
) -> Result<TestAiServiceResponse, CommandError> {
    let enhancer = state
        .runtime_factory
        .build_remote_service_enhancer_for_test(payload.service_id.trim())
        .map_err(|error| CommandError::new("ai_service_configuration", error))?
        .ok_or_else(|| {
            CommandError::new(
                "ai_service_unavailable",
                "The AI service is disabled or its configuration is incomplete",
            )
        })?;
    let answer = enhancer
        .ask("Reply with exactly: OK")
        .await
        .map_err(classify_ai_service_error)?;
    if answer.trim().is_empty() {
        return Err(CommandError::new(
            "ai_service_empty_response",
            "The AI service returned an empty response",
        ));
    }
    Ok(TestAiServiceResponse {
        ok: true,
        message: "AI service connection verified".to_string(),
    })
}

fn classify_ai_service_error(error: sbobino_application::ApplicationError) -> CommandError {
    let message = error.to_string();
    let normalized = message.to_ascii_lowercase();
    let code = if normalized.contains("429")
        || normalized.contains("quota")
        || normalized.contains("rate limit")
    {
        "ai_quota"
    } else if normalized.contains("401")
        || normalized.contains("403")
        || normalized.contains("authentication")
        || normalized.contains("api key")
    {
        "ai_authentication"
    } else if normalized.contains("model")
        && (normalized.contains("not found")
            || normalized.contains("invalid")
            || normalized.contains("404"))
    {
        "ai_model"
    } else if normalized.contains("request failed")
        || normalized.contains("connection")
        || normalized.contains("network")
        || normalized.contains("timed out")
        || normalized.contains("timeout")
    {
        "ai_connection"
    } else if normalized.contains("404")
        || normalized.contains("endpoint")
        || normalized.contains("url")
    {
        "ai_endpoint"
    } else {
        "ai_service_test"
    };
    CommandError::new(code, message)
}

#[tauri::command]
pub async fn test_prompt(
    state: State<'_, AppState>,
    payload: TestPromptPayload,
) -> Result<TestPromptResponse, CommandError> {
    let input = payload.input.trim().to_string();
    if input.is_empty() {
        return Err(CommandError::new(
            "validation",
            "prompt test input cannot be empty",
        ));
    }

    let settings = state
        .settings_service
        .snapshot()
        .await
        .map_err(CommandError::from)?;

    let model = payload
        .model_override
        .clone()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| settings.ai.providers.gemini.model.clone());
    let language = payload
        .language
        .unwrap_or(settings.transcription.language)
        .as_whisper_code()
        .to_string();
    let task = payload.task.unwrap_or(PromptTask::Optimize);
    let (optimize_prompt_override, summary_prompt_override) = match task {
        PromptTask::Optimize => (payload.prompt_override.clone(), None),
        PromptTask::Summary | PromptTask::Faq | PromptTask::EmotionAnalysis => {
            (None, payload.prompt_override.clone())
        }
    };
    let enhancers = state
        .runtime_factory
        .build_enhancer_candidates_with_overrides(
            Some(model.clone()),
            optimize_prompt_override,
            summary_prompt_override,
        )
        .map_err(|e| CommandError::new("runtime_factory", e))?;
    if enhancers.is_empty() {
        let reason = state
            .runtime_factory
            .ai_capability_status()
            .ok()
            .and_then(|status| status.unavailable_reason);
        return Err(missing_ai_provider_command_error(reason.as_deref()));
    }

    match task {
        PromptTask::Optimize => {
            let output =
                run_with_enhancer_fallback(&enhancers, "test optimize prompt", |enhancer| {
                    let input = input.clone();
                    let language = language.clone();
                    Box::pin(async move { enhancer.optimize(&input, &language).await })
                })
                .await
                .map_err(CommandError::from)?;

            Ok(TestPromptResponse {
                output: output.clone(),
                summary: String::new(),
                faqs: String::new(),
                model,
            })
        }
        PromptTask::Summary | PromptTask::Faq => {
            let output =
                run_with_enhancer_fallback(&enhancers, "test summary prompt", |enhancer| {
                    let input = input.clone();
                    let language = language.clone();
                    Box::pin(async move { enhancer.summarize_and_faq(&input, &language).await })
                })
                .await
                .map_err(CommandError::from)?;

            Ok(TestPromptResponse {
                output: format!("Summary:\n{}\n\nFAQs:\n{}", output.summary, output.faqs),
                summary: output.summary,
                faqs: output.faqs,
                model,
            })
        }
        PromptTask::EmotionAnalysis => {
            let result = analyze_emotions_with_enhancers(
                &enhancers,
                EmotionAnalysisInput {
                    title: "Prompt test".to_string(),
                    prepared: PreparedTranscriptContext::from_transcript(&input),
                },
                EmotionAnalysisOptions {
                    language: language.clone(),
                    include_timestamps: false,
                    include_speakers: false,
                    speaker_dynamics: false,
                    prompt_override: payload.prompt_override.clone(),
                },
            )
            .await
            .map_err(CommandError::from)?;

            Ok(TestPromptResponse {
                output: result.narrative_markdown,
                summary: String::new(),
                faqs: String::new(),
                model,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use sbobino_application::ApplicationError;

    use super::{classify_ai_service_error, draft_or_stored_value, AiServiceIdPayload};

    #[test]
    fn quota_errors_are_not_misclassified_as_authentication() {
        let error = ApplicationError::PostProcessing(
            "AI provider returned 429: API key quota exceeded".to_string(),
        );

        let classified = classify_ai_service_error(error);

        assert_eq!(classified.code, "ai_quota");
    }

    #[test]
    fn model_catalog_uses_non_empty_drafts_without_persisting_them() {
        assert_eq!(
            draft_or_stored_value(Some("  draft-key  "), Some("stored-key")),
            Some("draft-key".to_string())
        );
        assert_eq!(
            draft_or_stored_value(Some("  "), Some("stored-value")),
            Some("stored-value".to_string())
        );
        assert_eq!(draft_or_stored_value(Some(" "), None), None);

        let payload = serde_json::from_value::<AiServiceIdPayload>(serde_json::json!({
            "service_id": "service",
            "api_key": "draft-key",
            "base_url": "http://localhost:1234/v1"
        }))
        .expect("draft payload should deserialize");
        assert_eq!(payload.api_key.as_deref(), Some("draft-key"));
        assert_eq!(
            payload.base_url.as_deref(),
            Some("http://localhost:1234/v1")
        );
    }
}
