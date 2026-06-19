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
            "{template}\n\nLanguage: {language_instruction}\n\nAdditional cleanup rules:\n- Preserve the original language and the speaker's TONE, register, and level of formality — not the exact phrasing. The speaker's tone stays, but the words themselves should be the ones a careful editor would have chosen, not the ones that happened to come out of the speaker's mouth.\n- Produce a substantial, readable rewrite, not a light touch-up. Do not be timid: a transcript optimized by leaving 90% of the original words in place has not been optimized at all. Aggressively fix grammar, morphology, agreement, verb tenses, and word order.\n- Add proper punctuation (commas, periods, apostrophes, question marks, dashes, colons) and use correct capitalization. Insert paragraph breaks where the speaker clearly shifts topic or completes a thought.\n- Remove false starts, filler words, restarts, and verbal stumbles that do not change meaning. Merge broken sentences split across a pause or restart so the result reads naturally.\n- Restructure sentences for clarity when the spoken order is confusing, but keep the same ideas and topic order. Do not move ideas between topics.\n- Correct garbled or clearly misheard words and short phrases when the surrounding context makes the intended term highly likely. Pay special attention to technical terms, acronyms, library names, product names, names of people, and domain-specific jargon.\n- Understand the topic being discussed in the transcript. When the speaker's wording is ambiguous and the surrounding context makes the intended meaning clear, prefer the clearer wording. When the speaker's flow of ideas is logically sound but the connection between sentences is implicit, make that connection explicit with a short connective if it improves readability. When the speaker uses vague references like 'la cosa di cui parlavamo' or colloquial fillers like 'fare casino', and the topic is clear from the surrounding context, replace them with the topic-specific term or a more precise editorial form.\n- Normalize numbers, dates, and units to their most common written form when the context makes it unambiguous (for example \"twenty three\" -> \"23\", \"five kilometers\" -> \"5 km\").\n- Do not invent new facts, examples, names, numbers, or conclusions that are not in the original text. Do not summarize. Do not add introductory or meta phrases such as \"The speaker says that...\" or \"In this transcript...\".\n- Return only the cleaned transcript, with no commentary, headings, or labels.\n\nExample of the expected level of rewriting (Italian):\nInput: 'uh allora io dico che è importante capire il problema prima di iniziare a programmare e quindi dobbiamo prima fare una analisi attenta di quello che vogliamo realizzare'\nOutput: 'È importante capire il problema prima di iniziare a programmare. Dobbiamo quindi condurre un'analisi attenta di ciò che vogliamo realizzare.'
Another example of the expected level of rewriting (Italian):
Input: 'il progetto ha avuto successo. il team ha lavorato bene.'
Output: 'Il progetto ha avuto successo perché il team ha lavorato bene.'
Another example of topic-aware rewriting (Italian):
Input: 'allora dobbiamo capire bene la cosa di cui parlavamo prima di iniziare a programmare perche senno facciamo casino'
Output: 'Dobbiamo comprendere a fondo i requisiti del progetto software prima di iniziare a programmare, altrimenti creeremo confusione.'

\n\nTranscript:\n{text}\n\nReturn only the cleaned transcript."
        );
    }

    format!(
        "Clean this transcript while preserving the same language as the source text ({language_instruction}). Produce a substantial, readable rewrite, not a light touch-up. Do not be timid: a transcript optimized by leaving 90% of the original words in place has not been optimized at all. Aggressively fix grammar, morphology, agreement, verb tenses, and word order; add proper punctuation (commas, periods, apostrophes, question marks, dashes, colons) and correct capitalization; insert paragraph breaks where the speaker clearly shifts topic or completes a thought. Remove false starts, filler words, restarts, and verbal stumbles that do not change meaning, and merge broken sentences that the speaker split across a pause or restart so the result reads naturally. Restructure sentences for clarity when the spoken order is confusing, but keep the same ideas and topic order and do not move ideas between topics. Correct garbled or clearly misheard words and short phrases when the surrounding context makes the intended term highly likely, with special attention to technical terms, acronyms, library names, product names, names of people, and domain-specific jargon. Understand the topic being discussed; when the speaker's wording is ambiguous and the surrounding context makes the intended meaning clear, prefer the clearer wording, and when the speaker's flow of ideas is logically sound but the connection between sentences is implicit, make that connection explicit with a short connective if it improves readability, and when the speaker uses vague references like 'la cosa di cui parlavamo' or colloquial fillers like 'fare casino' and the topic is clear from the surrounding context, replace them with the topic-specific term or a more precise editorial form. Normalize numbers, dates, and units to their most common written form when the context makes it unambiguous (for example \"twenty three\" -> \"23\", \"five kilometers\" -> \"5 km\"). Preserve the original speaker's TONE, register, and level of formality — not the exact phrasing. The speaker's tone stays, but the words themselves should be the ones a careful editor would have chosen, not the ones that happened to come out of the speaker's mouth. Do not invent new facts, examples, names, numbers, or conclusions that are not in the original text. Do not summarize. Do not add introductory or meta phrases such as \"The speaker says that...\" or \"In this transcript...\". Return only the cleaned transcript, with no commentary, headings, or labels.\n\nExample of the expected level of rewriting (Italian):\nInput: 'uh allora io dico che è importante capire il problema prima di iniziare a programmare e quindi dobbiamo prima fare una analisi attenta di quello che vogliamo realizzare'\nOutput: 'È importante capire il problema prima di iniziare a programmare. Dobbiamo quindi condurre un'analisi attenta di ciò che vogliamo realizzare.'
Another example of the expected level of rewriting (Italian):
Input: 'il progetto ha avuto successo. il team ha lavorato bene.'
Output: 'Il progetto ha avuto successo perché il team ha lavorato bene.'
Another example of topic-aware rewriting (Italian):
Input: 'allora dobbiamo capire bene la cosa di cui parlavamo prima di iniziare a programmare perche senno facciamo casino'
Output: 'Dobbiamo comprendere a fondo i requisiti del progetto software prima di iniziare a programmare, altrimenti creeremo confusione.'

\n\n{text}"
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
    }

    #[test]
    fn optimize_prompt_default_authorizes_substantial_rewrites() {
        let prompt = build_optimize_prompt("ciao", "it", None, None);
        assert!(prompt.contains("Produce a substantial, readable rewrite"));
        assert!(prompt.contains("Remove false starts, filler words, restarts"));
        assert!(prompt.contains("Restructure sentences for clarity"));
        assert!(prompt.contains("Do not invent new facts"));
        assert!(prompt.contains("Do not summarize"));
        // Stronger framing: the prompt must explicitly tell the LLM not
        // to be timid and to anchor the expected level of change with a
        // concrete before/after example.
        assert!(prompt.contains("Do not be timid"));
        assert!(prompt.contains("90% of the original words"));
        assert!(prompt.contains("speaker's TONE"));
        assert!(prompt.contains("careful editor"));
        assert!(prompt.contains("Example of the expected level of rewriting"));
        // The 2nd example demonstrates the connective case enabled by the
        // is_tail_addition relaxation: a short connective (1 token)
        // inserted to make an implicit logical relationship explicit.
        assert!(prompt.contains("Another example of the expected level of rewriting"));
        assert!(prompt.contains("perché il team ha lavorato bene"));
        // The 3rd example demonstrates the topic-aware rewrite case:
        // vague placeholders ("la cosa di cui parlavamo", "casino")
        // replaced with topic-specific terms. This guards the 3rd
        // example directly so the prompt contract is locked in.
        assert!(prompt.contains("Another example of topic-aware rewriting"));
        assert!(prompt.contains("i requisiti del progetto software"));
    }

    #[test]
    fn optimize_prompt_template_extends_user_template_with_substantial_cleanup_rules() {
        let user_template = "You are a transcript editor.";
        let prompt = build_optimize_prompt("ciao", "en", Some(user_template), None);
        assert!(prompt.starts_with("You are a transcript editor."));
        assert!(prompt.contains("Produce a substantial, readable rewrite"));
        assert!(prompt.contains("Restructure sentences for clarity"));
        assert!(prompt.contains("Do not invent new facts"));
        // The user-template branch must carry the same anti-timid
        // framing and the concrete before/after example as the default
        // branch.
        assert!(prompt.contains("Do not be timid"));
        assert!(prompt.contains("90% of the original words"));
        assert!(prompt.contains("speaker's TONE"));
        assert!(prompt.contains("Example of the expected level of rewriting"));
        // The 2nd example demonstrates the connective case enabled by the
        // is_tail_addition relaxation: a short connective (1 token)
        // inserted to make an implicit logical relationship explicit.
        assert!(prompt.contains("Another example of the expected level of rewriting"));
        assert!(prompt.contains("perché il team ha lavorato bene"));
        // The 3rd example demonstrates the topic-aware rewrite case:
        // vague placeholders ("la cosa di cui parlavamo", "casino")
        // replaced with topic-specific terms. This guards the 3rd
        // example directly so the prompt contract is locked in.
        assert!(prompt.contains("Another example of topic-aware rewriting"));
        assert!(prompt.contains("i requisiti del progetto software"));
    }
    #[test]
    fn optimize_prompt_anchors_topic_and_contextual_logic() {
        // Regression guard: the prompt must explicitly ask the LLM to
        // understand the topic of the transcript and to make implicit
        // logical connections explicit when the surrounding context
        // makes the intended meaning clear. This addresses the
        // "logico e contestuale di ciò di cui si discute" requirement
        // that is not covered by the other substantial-rewrite anchors.
        let default_prompt = build_optimize_prompt("ciao", "it", None, None);
        let user_template = "You are a transcript editor.";
        let template_prompt = build_optimize_prompt("ciao", "en", Some(user_template), None);

        for (label, prompt) in [
            ("default", default_prompt.as_str()),
            ("template", template_prompt.as_str()),
        ] {
            assert!(
                prompt.contains("Understand the topic being discussed"),
                "[{label}] prompt must ask the LLM to understand the topic"
            );
            assert!(
                prompt.contains("surrounding context makes the intended meaning clear"),
                "[{label}] prompt must anchor the topic-aware disambiguation rule"
            );
            assert!(
                prompt.contains("make that connection explicit"),
                "[{label}] prompt must anchor the explicit-connective rule"
            );
            assert!(
                prompt.contains("vague references like 'la cosa di cui parlavamo'"),
                "[{label}] prompt must anchor the topic-aware substitution rule"
            );
        }
    }

    #[test]
    fn optimize_prompt_demonstrates_short_connective_case() {
        // The 2nd example in the prompt demonstrates the connective case
        // enabled by the is_tail_addition relaxation in
        // transcript_cleanup.rs. Without the 2nd example, the LLM has
        // only the substantial-rewrite example to anchor on, and may
        // not realize that adding a short connective (1-2 tokens) to
        // make an implicit logical relationship explicit is part of
        // the expected output. This test guards the 2nd example
        // directly so the prompt contract is locked in.
        let default_prompt = build_optimize_prompt("ciao", "it", None, None);
        let user_template = "You are a transcript editor.";
        let template_prompt = build_optimize_prompt("ciao", "en", Some(user_template), None);

        for (branch, prompt) in [("default", &default_prompt), ("template", &template_prompt)] {
            assert!(
                prompt.contains("Another example of the expected level of rewriting"),
                "{branch} branch must include the 2nd example marker"
            );
            assert!(
                prompt.contains("perché il team ha lavorato bene"),
                "{branch} branch must demonstrate the connective output"
            );
            assert!(
                prompt.contains("Another example of topic-aware rewriting"),
                "{branch} branch must include the 3rd example marker"
            );
            assert!(
                prompt.contains("i requisiti del progetto software"),
                "{branch} branch must demonstrate the topic-aware rewrite"
            );
        }
    }

    #[test]
    fn optimize_prompt_demonstrates_topic_aware_rewrite() {
        // The 3rd example in the prompt demonstrates the topic-aware
        // rewrite case: vague placeholders ("la cosa di cui parlavamo",
        // "casino") replaced with topic-specific terms. Without this
        // example, the LLM has no anchor for substitutions that go
        // beyond the safe "insert connective / fix garbled term"
        // family, and may default to the timid "light touch-up" output
        // the user explicitly wants to avoid. This test guards the
        // 3rd example directly so the prompt contract is locked in.
        let default_prompt = build_optimize_prompt("ciao", "it", None, None);
        let user_template = "You are a transcript editor.";
        let template_prompt = build_optimize_prompt("ciao", "en", Some(user_template), None);

        for (branch, prompt) in [("default", &default_prompt), ("template", &template_prompt)] {
            assert!(
                prompt.contains("Another example of topic-aware rewriting"),
                "{branch} branch must include the 3rd example marker"
            );
            assert!(
                prompt.contains("i requisiti del progetto software"),
                "{branch} branch must demonstrate the topic-aware rewrite"
            );
        }
    }
}
