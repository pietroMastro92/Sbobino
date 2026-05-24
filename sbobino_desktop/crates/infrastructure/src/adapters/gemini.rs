use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

use sbobino_application::{dto::SummaryFaq, ApplicationError, TranscriptEnhancer};

#[derive(Debug, Clone)]
pub struct GeminiEnhancer {
    client: Client,
    api_key: String,
    model: String,
    optimize_prompt_override: Option<String>,
    summary_prompt_override: Option<String>,
}

impl GeminiEnhancer {
    pub fn new(
        api_key: String,
        model: String,
        optimize_prompt_override: Option<String>,
        summary_prompt_override: Option<String>,
    ) -> Self {
        Self {
            client: Client::new(),
            api_key,
            model,
            optimize_prompt_override: normalize_prompt(optimize_prompt_override),
            summary_prompt_override: normalize_prompt(summary_prompt_override),
        }
    }

    async fn generate(&self, prompt: &str) -> Result<String, ApplicationError> {
        let endpoint = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, self.api_key
        );

        let response = self
            .client
            .post(endpoint)
            .json(&json!({
                "contents": [{
                    "parts": [{"text": prompt}]
                }],
                "generationConfig": {
                    "temperature": 0.3,
                    "topP": 0.95,
                    "maxOutputTokens": 4096
                }
            }))
            .send()
            .await
            .map_err(|e| ApplicationError::PostProcessing(format!("gemini request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ApplicationError::PostProcessing(format!(
                "gemini API returned {status}: {body}"
            )));
        }

        let payload: GeminiResponse = response.json().await.map_err(|e| {
            ApplicationError::PostProcessing(format!("invalid gemini response: {e}"))
        })?;

        payload
            .candidates
            .into_iter()
            .flat_map(|candidate| candidate.content.parts.into_iter())
            .find_map(|part| part.text)
            .ok_or_else(|| {
                ApplicationError::PostProcessing(
                    "gemini response did not contain generated text".to_string(),
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
impl TranscriptEnhancer for GeminiEnhancer {
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
        GeminiEnhancer::ask(self, prompt).await
    }

    fn summary_direct_prompt_char_budget(&self) -> usize {
        18_000
    }

    fn optimize_direct_prompt_char_budget(&self) -> usize {
        12_000
    }

    fn optimize_chunk_concurrency_limit(&self) -> usize {
        3
    }

    fn emotion_direct_prompt_char_budget(&self) -> usize {
        12_000
    }

    fn telemetry_provider_label(&self) -> &'static str {
        "gemini"
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
            "{template}\n\nLanguage: {language_instruction}\n\nAdditional cleanup rules:\n- Preserve the original wording, structure, and order of the transcript as much as possible.\n- Improve punctuation, capitalization, spacing, and paragraph breaks.\n- Remove obvious accidental repetitions, duplicated lines, and looped sentences.\n- Keep only one occurrence when the same sentence is repeated in sequence by mistake.\n- Correct isolated words or short phrases that are clearly wrong ASR/transcription mistakes when the surrounding context makes the intended meaning highly likely.\n- Prefer minimal local corrections, especially for technical terms, acronyms, library names, product names, and domain-specific jargon.\n- If you are not confident about a correction, keep the original wording.\n- Do not paraphrase whole sentences, summarize, reorder ideas, or invent missing facts.\n\nTranscript:\n{text}\n\nReturn only the cleaned transcript."
        );
    }

    format!(
        "Clean this transcript while preserving the same language as the source text ({language_instruction}). Preserve the original wording, structure, and order as much as possible. Improve punctuation, capitalization, spacing, and paragraph breaks, and remove obvious transcription glitches such as consecutive duplicated lines, repeated phrases, looped sentences, and hallucinated filler. When the same sentence is repeated accidentally in sequence, keep only the single best occurrence. You may correct isolated words or short phrases that are clearly wrong ASR/transcription mistakes when the surrounding context makes the intended term highly likely, especially for technical terms, acronyms, library names, product names, and domain-specific jargon. Prefer minimal local corrections. If uncertain, keep the original wording. Do not paraphrase whole sentences, summarize, reorder ideas, or invent missing facts. Return only the cleaned transcript.\n\n{text}"
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
        "Generate in language {language_code}:\n1) Summary\n2) Exactly 3 FAQs with answers.\n\nSummary requirements:\n- Write a detailed, sectioned briefing note, not a terse recap.\n- Cover all major topics, technical details, examples, numbers, and decisions.\n- Preserve how the ideas relate to each other and explain why they matter.\n- Keep the summary self-contained for a reader who has not heard the recording.\n\nFormat:\nSummary:\n...\nFAQs:\nQ:...\nA:...\n\nText:\n{text}"
    )
}

#[derive(Debug, Deserialize)]
struct GeminiResponse {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
}

#[derive(Debug, Deserialize)]
struct GeminiCandidate {
    content: GeminiContent,
}

#[derive(Debug, Deserialize)]
struct GeminiContent {
    #[serde(default)]
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Deserialize)]
struct GeminiPart {
    #[serde(default)]
    text: Option<String>,
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
