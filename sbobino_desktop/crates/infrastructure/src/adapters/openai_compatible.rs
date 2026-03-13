use async_trait::async_trait;
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION},
    Client,
};
use serde::Deserialize;
use serde_json::json;

use sbobino_application::{dto::SummaryFaq, ApplicationError, TranscriptEnhancer};

#[derive(Debug, Clone, Copy)]
pub enum AuthStyle {
    None,
    Bearer,
    ApiKeyHeader,
}

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleEnhancer {
    client: Client,
    endpoint: String,
    model: String,
    headers: HeaderMap,
    optimize_prompt_override: Option<String>,
    summary_prompt_override: Option<String>,
}

impl OpenAiCompatibleEnhancer {
    pub fn new(
        base_url: String,
        model: String,
        api_key: Option<String>,
        auth_style: AuthStyle,
        optimize_prompt_override: Option<String>,
        summary_prompt_override: Option<String>,
    ) -> Result<Self, ApplicationError> {
        let endpoint = normalize_chat_endpoint(&base_url);
        if endpoint.is_empty() {
            return Err(ApplicationError::Settings(
                "AI service base URL cannot be empty".to_string(),
            ));
        }
        let model = model.trim().to_string();
        if model.is_empty() {
            return Err(ApplicationError::Settings(
                "AI service model cannot be empty".to_string(),
            ));
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/json"),
        );

        match auth_style {
            AuthStyle::None => {}
            AuthStyle::Bearer => {
                let key = api_key
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        ApplicationError::Settings("AI service API key is required".to_string())
                    })?;
                let auth_value = HeaderValue::from_str(&format!("Bearer {key}")).map_err(|e| {
                    ApplicationError::Settings(format!("invalid API key header value: {e}"))
                })?;
                headers.insert(AUTHORIZATION, auth_value);
            }
            AuthStyle::ApiKeyHeader => {
                let key = api_key
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        ApplicationError::Settings("AI service API key is required".to_string())
                    })?;
                let header_value = HeaderValue::from_str(key).map_err(|e| {
                    ApplicationError::Settings(format!("invalid API key header value: {e}"))
                })?;
                headers.insert(HeaderName::from_static("api-key"), header_value);
            }
        }

        Ok(Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(90))
                .build()
                .unwrap_or_else(|_| Client::new()),
            endpoint,
            model,
            headers,
            optimize_prompt_override: normalize_prompt(optimize_prompt_override),
            summary_prompt_override: normalize_prompt(summary_prompt_override),
        })
    }

    async fn generate(&self, prompt: &str) -> Result<String, ApplicationError> {
        let response = self
            .client
            .post(&self.endpoint)
            .headers(self.headers.clone())
            .json(&json!({
                "model": self.model,
                "messages": [
                    {
                        "role": "user",
                        "content": prompt
                    }
                ],
                "temperature": 0.3,
                "max_tokens": 4096
            }))
            .send()
            .await
            .map_err(|e| ApplicationError::PostProcessing(format!("AI request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ApplicationError::PostProcessing(format!(
                "AI provider returned {status}: {body}"
            )));
        }

        let payload: OpenAiChatResponse = response.json().await.map_err(|e| {
            ApplicationError::PostProcessing(format!("invalid AI provider response: {e}"))
        })?;

        payload
            .choices
            .into_iter()
            .find_map(|choice| extract_content(choice.message.content))
            .and_then(|text| {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            })
            .ok_or_else(|| {
                ApplicationError::PostProcessing(
                    "AI provider response did not contain generated text".to_string(),
                )
            })
    }

    pub async fn ask(&self, prompt: &str) -> Result<String, ApplicationError> {
        self.generate(prompt).await
    }

    pub async fn optimize_with_prompt(
        &self,
        text: &str,
        language_code: &str,
        prompt_override: Option<&str>,
    ) -> Result<String, ApplicationError> {
        let prompt = build_optimize_prompt(
            text,
            language_code,
            prompt_override,
            self.optimize_prompt_override.as_deref(),
        );
        self.generate(&prompt).await
    }

    pub async fn summarize_and_faq_with_prompt(
        &self,
        text: &str,
        language_code: &str,
        prompt_override: Option<&str>,
    ) -> Result<SummaryFaq, ApplicationError> {
        let prompt = build_summary_prompt(
            text,
            language_code,
            prompt_override,
            self.summary_prompt_override.as_deref(),
        );
        let output = self.generate(&prompt).await?;

        let (summary, faqs) = if let Some((left, right)) = output.split_once("FAQs:") {
            (
                left.replace("Summary:", "").trim().to_string(),
                right.trim().to_string(),
            )
        } else {
            (output.trim().to_string(), String::new())
        };

        Ok(SummaryFaq { summary, faqs })
    }
}

#[async_trait]
impl TranscriptEnhancer for OpenAiCompatibleEnhancer {
    async fn optimize(&self, text: &str, language_code: &str) -> Result<String, ApplicationError> {
        self.optimize_with_prompt(text, language_code, None).await
    }

    async fn summarize_and_faq(
        &self,
        text: &str,
        language_code: &str,
    ) -> Result<SummaryFaq, ApplicationError> {
        self.summarize_and_faq_with_prompt(text, language_code, None)
            .await
    }

    async fn ask(&self, prompt: &str) -> Result<String, ApplicationError> {
        OpenAiCompatibleEnhancer::ask(self, prompt).await
    }
}

fn normalize_prompt(value: Option<String>) -> Option<String> {
    value.and_then(|prompt| {
        let trimmed = prompt.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn normalize_chat_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if trimmed.contains("/chat/completions") {
        return trimmed.to_string();
    }

    if trimmed.ends_with('/') {
        format!("{trimmed}chat/completions")
    } else {
        format!("{trimmed}/chat/completions")
    }
}

fn build_optimize_prompt(
    text: &str,
    language_code: &str,
    prompt_override: Option<&str>,
    default_override: Option<&str>,
) -> String {
    let language_instruction = optimize_language_instruction(language_code);
    if let Some(template) = prompt_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            default_override
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
    {
        return format!(
            "{template}\n\nLanguage: {language_instruction}\n\nAdditional cleanup rules:\n- Preserve the original wording and order of the transcript.\n- Only improve punctuation, capitalization, spacing, and paragraph breaks.\n- Remove obvious accidental repetitions, duplicated lines, and looped sentences.\n- Keep only one occurrence when the same sentence is repeated in sequence by mistake.\n- Do not paraphrase, summarize, rewrite, fix wording, or add any words that are not already present in the transcript.\n- Do not invent missing content.\n\nTranscript:\n{text}\n\nReturn only the cleaned transcript."
        );
    }

    format!(
        "Clean this transcript while preserving the same language as the source text ({language_instruction}). Preserve the original wording and order. Only improve punctuation, capitalization, spacing, and paragraph breaks, and remove obvious transcription glitches such as consecutive duplicated lines, repeated phrases, looped sentences, and hallucinated filler. When the same sentence is repeated accidentally in sequence, keep only the single best occurrence. Do not paraphrase, summarize, rewrite, fix wording, or add any words that are not already present in the transcript. Do not invent missing content. Return only the cleaned transcript.\n\n{text}"
    )
}

fn optimize_language_instruction(language_code: &str) -> &str {
    let normalized = language_code.trim();
    if normalized.is_empty() || normalized == "auto" {
        "the same language as the transcript"
    } else {
        normalized
    }
}

fn build_summary_prompt(
    text: &str,
    language_code: &str,
    prompt_override: Option<&str>,
    default_override: Option<&str>,
) -> String {
    if let Some(template) = prompt_override
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            default_override
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
    {
        return format!(
            "{template}\n\nLanguage: {language_code}\n\nTranscript:\n{text}\n\nFormat strictly as:\nSummary:\n...\nFAQs:\nQ:...\nA:..."
        );
    }

    format!(
        "Generate in language {language_code}:\n1) Summary\n2) Exactly 3 FAQs with answers.\nFormat:\nSummary:\n...\nFAQs:\nQ:...\nA:...\n\nText:\n{text}"
    )
}

#[cfg(test)]
mod tests {
    use super::build_optimize_prompt;

    #[test]
    fn optimize_prompt_defaults_to_source_language_when_auto() {
        let prompt = build_optimize_prompt("ciao", "auto", None, None);
        assert!(prompt.contains("the same language as the source text"));
        assert!(prompt.contains("the same language as the transcript"));
        assert!(prompt.contains("repeated phrases"));
    }
}

fn extract_content(content: MessageContent) -> Option<String> {
    match content {
        MessageContent::Text(value) => Some(value),
        MessageContent::Parts(parts) => {
            let joined = parts
                .into_iter()
                .filter_map(|part| part.text)
                .collect::<Vec<_>>()
                .join("\n");
            if joined.trim().is_empty() {
                None
            } else {
                Some(joined)
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct OpenAiChatResponse {
    #[serde(default)]
    choices: Vec<OpenAiChatChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChatChoice {
    message: OpenAiMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiMessage {
    content: MessageContent,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum MessageContent {
    Text(String),
    Parts(Vec<MessageContentPart>),
}

#[derive(Debug, Deserialize)]
struct MessageContentPart {
    #[serde(default)]
    text: Option<String>,
}
