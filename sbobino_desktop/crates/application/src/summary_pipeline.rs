use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    hash::{Hash, Hasher},
    sync::{Mutex, OnceLock},
    time::Instant,
};

use futures_util::stream::{self, StreamExt};
use tracing::info;

use sbobino_domain::minimize_transcript_repetitions;

use crate::{dto::SummaryFaq, ApplicationError, TranscriptEnhancer};

const SUMMARY_CHUNK_TARGET_CHARS: usize = 4000;
const SUMMARY_CHUNK_OVERLAP_WORDS: usize = 30;
const SUMMARY_CHUNK_CONCURRENCY_LIMIT: usize = 3;
const SUMMARY_SYNTHESIS_BUDGETS: &[usize] = &[12_000, 8_000, 5_000, 3_000];
const SUMMARY_CONTEXT_OVERFLOW_MESSAGE: &str =
    "Exceeded model context window size. The app now uses chunked retrieval, but this request is still too large. Try a shorter custom prompt or fewer summary constraints.";
const SUMMARY_COMPACT_DIRECT_TRANSCRIPT_TARGET_CHARS: usize = 6_400;
const SUMMARY_CACHE_MAX_ENTRIES: usize = 256;

#[derive(Debug, Clone)]
pub struct PreparedSummaryContext {
    pub cleaned_transcript: String,
    pub transcript_hash: String,
    pub char_count: usize,
    pub word_count: usize,
}

#[derive(Debug, Clone)]
pub enum SummaryMode {
    SummaryOnly { user_instructions: String },
    SummaryAndFaq { language_code: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SummaryStrategy {
    DirectFull,
    DirectCompact,
    Chunked,
}

#[derive(Debug, Clone)]
struct SummaryTelemetry {
    provider: &'static str,
    mode: &'static str,
    strategy: SummaryStrategy,
    transcript_chars: usize,
    transcript_words: usize,
    chunk_count: usize,
    direct_attempted: bool,
    direct_skipped_reason: Option<&'static str>,
    cache_hits: usize,
    preprocess_ms: u128,
    ai_ms: u128,
    synthesis_ms: u128,
}

impl PreparedSummaryContext {
    pub fn new(transcript: &str) -> Result<Self, ApplicationError> {
        let cleaned_transcript = minimize_transcript_repetitions(transcript)
            .trim()
            .to_string();
        if cleaned_transcript.is_empty() {
            return Err(ApplicationError::Validation(
                "cannot summarize an empty transcript".to_string(),
            ));
        }

        let mut hasher = DefaultHasher::new();
        cleaned_transcript.hash(&mut hasher);
        let transcript_hash = format!("{:016x}", hasher.finish());
        let char_count = cleaned_transcript.chars().count();
        let word_count = cleaned_transcript.split_whitespace().count();

        Ok(Self {
            cleaned_transcript,
            transcript_hash,
            char_count,
            word_count,
        })
    }
}

pub async fn summarize_transcript_adaptive(
    enhancer: &dyn TranscriptEnhancer,
    transcript: &str,
    user_instructions: &str,
) -> Result<String, ApplicationError> {
    let prepared = PreparedSummaryContext::new(transcript)?;
    let output = run_summary_pipeline(
        enhancer,
        &prepared,
        SummaryMode::SummaryOnly {
            user_instructions: user_instructions.trim().to_string(),
        },
    )
    .await?;
    Ok(output.trim().to_string())
}

pub async fn summarize_and_faq_adaptive(
    enhancer: &dyn TranscriptEnhancer,
    transcript: &str,
    language_code: &str,
) -> Result<SummaryFaq, ApplicationError> {
    let prepared = PreparedSummaryContext::new(transcript)?;
    let output = run_summary_pipeline(
        enhancer,
        &prepared,
        SummaryMode::SummaryAndFaq {
            language_code: language_code.trim().to_string(),
        },
    )
    .await?;
    Ok(parse_summary_faq_output(&output))
}

async fn run_summary_pipeline(
    enhancer: &dyn TranscriptEnhancer,
    prepared: &PreparedSummaryContext,
    mode: SummaryMode,
) -> Result<String, ApplicationError> {
    let preprocess_started = Instant::now();
    let direct_budget = enhancer.summary_direct_prompt_char_budget().max(2_400);
    let full_direct_prompt = build_direct_prompt(&mode, &prepared.cleaned_transcript);
    let compact_transcript = compact_transcript_excerpt(
        &prepared.cleaned_transcript,
        SUMMARY_COMPACT_DIRECT_TRANSCRIPT_TARGET_CHARS,
    );
    let compact_direct_prompt = if compact_transcript != prepared.cleaned_transcript {
        Some(build_compact_direct_prompt(&mode, &compact_transcript))
    } else {
        None
    };
    let chunk_target_chars = compute_summary_chunk_target_chars(&mode, direct_budget);
    let chunks = chunk_text_by_target_chars(
        &prepared.cleaned_transcript,
        chunk_target_chars,
        SUMMARY_CHUNK_OVERLAP_WORDS,
    );
    let strategy = choose_summary_strategy(
        enhancer,
        direct_budget,
        &full_direct_prompt,
        compact_direct_prompt.as_deref(),
        chunks.len(),
    );
    let preprocess_ms = preprocess_started.elapsed().as_millis();
    let mut telemetry = SummaryTelemetry {
        provider: enhancer.telemetry_provider_label(),
        mode: summary_mode_name(&mode),
        strategy,
        transcript_chars: prepared.char_count,
        transcript_words: prepared.word_count,
        chunk_count: chunks.len(),
        direct_attempted: false,
        direct_skipped_reason: None,
        cache_hits: 0,
        preprocess_ms,
        ai_ms: 0,
        synthesis_ms: 0,
    };

    let ai_started = Instant::now();
    let result = match strategy {
        SummaryStrategy::DirectFull => {
            telemetry.direct_attempted = true;
            match enhancer.ask(&full_direct_prompt).await {
                Ok(answer) if !answer.trim().is_empty() => Ok(answer.trim().to_string()),
                Ok(_) => {
                    if chunks.len() <= 1 {
                        ask_with_overflow_fallback(
                            enhancer,
                            vec![
                                full_direct_prompt.clone(),
                                compact_direct_prompt
                                    .clone()
                                    .unwrap_or_else(|| full_direct_prompt.clone()),
                            ],
                        )
                        .await
                    } else {
                        synthesize_from_chunks(enhancer, prepared, &mode, &chunks, &mut telemetry)
                            .await
                    }
                }
                Err(error) if is_context_window_error(&error) => {
                    if let Some(compact_prompt) = compact_direct_prompt.clone() {
                        telemetry.strategy = SummaryStrategy::DirectCompact;
                        match enhancer.ask(&compact_prompt).await {
                            Ok(answer) if !answer.trim().is_empty() => {
                                Ok(answer.trim().to_string())
                            }
                            Ok(_) => {
                                if chunks.len() > 1 {
                                    telemetry.strategy = SummaryStrategy::Chunked;
                                    synthesize_from_chunks(
                                        enhancer,
                                        prepared,
                                        &mode,
                                        &chunks,
                                        &mut telemetry,
                                    )
                                    .await
                                } else {
                                    ask_with_overflow_fallback(enhancer, vec![compact_prompt]).await
                                }
                            }
                            Err(error) if chunks.len() > 1 && is_context_window_error(&error) => {
                                telemetry.strategy = SummaryStrategy::Chunked;
                                synthesize_from_chunks(
                                    enhancer,
                                    prepared,
                                    &mode,
                                    &chunks,
                                    &mut telemetry,
                                )
                                .await
                            }
                            Err(error) => Err(error),
                        }
                    } else if chunks.len() > 1 {
                        telemetry.strategy = SummaryStrategy::Chunked;
                        synthesize_from_chunks(enhancer, prepared, &mode, &chunks, &mut telemetry)
                            .await
                    } else {
                        ask_with_overflow_fallback(enhancer, vec![full_direct_prompt.clone()]).await
                    }
                }
                Err(error) => Err(error),
            }
        }
        SummaryStrategy::DirectCompact => {
            telemetry.direct_attempted = true;
            let compact_prompt = compact_direct_prompt
                .clone()
                .unwrap_or_else(|| full_direct_prompt.clone());
            match enhancer.ask(&compact_prompt).await {
                Ok(answer) if !answer.trim().is_empty() => Ok(answer.trim().to_string()),
                Ok(_) => {
                    if chunks.len() > 1 {
                        telemetry.strategy = SummaryStrategy::Chunked;
                        synthesize_from_chunks(enhancer, prepared, &mode, &chunks, &mut telemetry)
                            .await
                    } else {
                        ask_with_overflow_fallback(enhancer, vec![compact_prompt]).await
                    }
                }
                Err(error) if chunks.len() > 1 && is_context_window_error(&error) => {
                    telemetry.strategy = SummaryStrategy::Chunked;
                    synthesize_from_chunks(enhancer, prepared, &mode, &chunks, &mut telemetry).await
                }
                Err(error) => Err(error),
            }
        }
        SummaryStrategy::Chunked => {
            telemetry.direct_skipped_reason = Some("prompt_budget");
            synthesize_from_chunks(enhancer, prepared, &mode, &chunks, &mut telemetry).await
        }
    };
    telemetry.ai_ms = ai_started.elapsed().as_millis();
    log_summary_telemetry(&telemetry);
    result
}

fn choose_summary_strategy(
    enhancer: &dyn TranscriptEnhancer,
    direct_budget: usize,
    full_direct_prompt: &str,
    compact_direct_prompt: Option<&str>,
    chunk_count: usize,
) -> SummaryStrategy {
    let full_chars = full_direct_prompt.chars().count();
    let compact_chars = compact_direct_prompt.map(|prompt| prompt.chars().count());
    if chunk_count <= 1 {
        if full_chars <= direct_budget {
            return SummaryStrategy::DirectFull;
        }
        if compact_chars.is_some_and(|chars| chars <= direct_budget) {
            return SummaryStrategy::DirectCompact;
        }
        return SummaryStrategy::Chunked;
    }

    if enhancer.prefers_single_pass_summary() {
        if full_chars <= direct_budget {
            SummaryStrategy::DirectFull
        } else if compact_chars.is_some_and(|chars| chars <= direct_budget) {
            SummaryStrategy::DirectCompact
        } else {
            SummaryStrategy::Chunked
        }
    } else {
        SummaryStrategy::Chunked
    }
}

async fn synthesize_from_chunks(
    enhancer: &dyn TranscriptEnhancer,
    prepared: &PreparedSummaryContext,
    mode: &SummaryMode,
    chunks: &[String],
    telemetry: &mut SummaryTelemetry,
) -> Result<String, ApplicationError> {
    if chunks.is_empty() {
        return Err(ApplicationError::Validation(
            "cannot summarize an empty transcript".to_string(),
        ));
    }
    if chunks.len() == 1 {
        let direct_prompt = build_direct_prompt(mode, &chunks[0]);
        return ask_with_overflow_fallback(enhancer, vec![direct_prompt]).await;
    }

    let total = chunks.len();
    let chunk_concurrency_limit = enhancer
        .summary_chunk_concurrency_limit()
        .clamp(1, SUMMARY_CHUNK_CONCURRENCY_LIMIT);
    let chunk_concurrency = total.clamp(1, chunk_concurrency_limit);

    let synthesis_started = Instant::now();
    let chunk_notes = stream::iter(chunks.iter().cloned().enumerate())
        .map(|(index, chunk)| async move {
            let chunk_prompt = build_chunk_note_prompt(index + 1, total, mode, chunk.as_str());
            let cache_key = build_summary_cache_key(
                prepared,
                enhancer.telemetry_provider_label(),
                mode,
                index,
                &chunk_prompt,
            );
            if let Some(cached) = summary_chunk_note_cache()
                .lock()
                .ok()
                .and_then(|cache| cache.get(&cache_key).cloned())
            {
                return Ok::<(String, bool), ApplicationError>((cached, true));
            }

            let note = ask_with_overflow_fallback(
                enhancer,
                vec![
                    chunk_prompt.clone(),
                    truncate_chars(&chunk_prompt, 2600),
                    truncate_chars(&chunk_prompt, 1900),
                ],
            )
            .await?;
            if let Ok(mut cache) = summary_chunk_note_cache().lock() {
                if cache.len() >= SUMMARY_CACHE_MAX_ENTRIES {
                    cache.clear();
                }
                cache.insert(cache_key, note.clone());
            }
            Ok((note, false))
        })
        .buffered(chunk_concurrency)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    telemetry.cache_hits = chunk_notes.iter().filter(|(_, cached)| *cached).count();
    let merged_notes = chunk_notes
        .into_iter()
        .enumerate()
        .map(|(index, (note, _))| format!("Chunk {} notes:\n{}", index + 1, note.trim()))
        .collect::<Vec<_>>()
        .join("\n\n");
    let candidates = SUMMARY_SYNTHESIS_BUDGETS
        .iter()
        .map(|budget| {
            let clipped_notes = truncate_chars(&merged_notes, *budget);
            build_summary_synthesis_prompt(mode, &clipped_notes)
        })
        .collect::<Vec<_>>();
    let result = ask_with_overflow_fallback(enhancer, candidates).await;
    telemetry.synthesis_ms = synthesis_started.elapsed().as_millis();
    result
}

fn build_direct_prompt(mode: &SummaryMode, transcript: &str) -> String {
    match mode {
        SummaryMode::SummaryOnly { user_instructions } => format!(
            "You are writing the final summary of a transcript.\n\n\
             User instructions (follow these exactly — including language, structure, and formatting preferences):\n\
             {user_instructions}\n\n\
             Requirements for the final summary:\n\
             - Produce a dense, polished document — not a terse recap or a sparse outline.\n\
             - Cover all major subjects discussed in the transcript with enough depth that a reader \
             who has not heard the original audio would understand the goals, reasoning, evidence, and outcomes.\n\
             - Preserve specific details that matter: names, numbers, dates, technical terms, examples, constraints, and decisions.\n\
             - Explain how topics relate to one another instead of listing them in isolation.\n\
             - When the transcript is technical, keep the technical content explicit and accurate rather than generalizing it away.\n\
             - If there are debates, alternatives, uncertainties, or tradeoffs, describe them clearly.\n\
             - Maintain logical flow between topics: use transitions and group related ideas together.\n\
             - Respect the user's language, structural, and formatting preferences exactly.\n\
             - Output ONLY the summary text. Do not add meta-commentary or labels like \"Summary:\".\n\n\
             Full transcript:\n{transcript}"
        ),
        SummaryMode::SummaryAndFaq { language_code } => format!(
            "Generate in language {language_code}:\n1) Summary\n2) Exactly 3 FAQs with answers.\n\n\
             Summary requirements:\n\
             - Write a detailed, sectioned briefing note, not a terse recap.\n\
             - Cover all major topics, technical details, examples, numbers, and decisions.\n\
             - Preserve how the ideas relate to each other and explain why they matter.\n\
             - Keep the summary self-contained for a reader who has not heard the recording.\n\n\
             Format:\nSummary:\n...\nFAQs:\nQ:...\nA:...\n\n\
             Transcript:\n{transcript}"
        ),
    }
}

fn build_compact_direct_prompt(mode: &SummaryMode, transcript_excerpt: &str) -> String {
    match mode {
        SummaryMode::SummaryOnly { user_instructions } => format!(
            "You are writing the final summary of a transcript from a compact transcript view.\n\n\
             User instructions (follow these exactly — including language, structure, and formatting preferences):\n\
             {user_instructions}\n\n\
             Requirements for the final summary:\n\
             - Produce a dense, polished document grounded in the excerpted transcript view below.\n\
             - Preserve specific details, decisions, technical terms, examples, and relationships between topics when they appear.\n\
             - Prefer the most central discussion threads and avoid fabricating missing transitions.\n\
             - Output ONLY the summary text. Do not add meta-commentary or labels like \"Summary:\".\n\n\
             Compact transcript view:\n{transcript_excerpt}"
        ),
        SummaryMode::SummaryAndFaq { language_code } => format!(
            "Generate in language {language_code} from the compact transcript view below:\n1) Summary\n2) Exactly 3 FAQs with answers.\n\n\
             Keep the result grounded in the excerpted transcript only.\n\n\
             Format:\nSummary:\n...\nFAQs:\nQ:...\nA:...\n\n\
             Compact transcript view:\n{transcript_excerpt}"
        ),
    }
}

fn build_chunk_note_prompt(
    chunk_index: usize,
    total_chunks: usize,
    mode: &SummaryMode,
    chunk: &str,
) -> String {
    match mode {
        SummaryMode::SummaryOnly { user_instructions } => format!(
            "You are extracting detailed notes from a transcript chunk to support a comprehensive final brief.\n\
             Your goal is to capture ALL substantive content — not just keywords.\n\n\
             User instructions (follow these exactly):\n{user_instructions}\n\n\
             This is chunk {chunk_index}/{total_chunks} of the full transcript.\n\n\
             Extract the following from this chunk:\n\
             - Main topics, subtopics, and arguments discussed, with enough context to understand them\n\
             - Key facts, statistics, names, dates, technical terminology, and specific claims\n\
             - Explanations, reasoning, comparisons, and cause-effect relationships\n\
             - Decisions made, open questions, risks, action items, or next steps mentioned\n\
             - Examples, evidence, or concrete scenarios used by the speakers\n\
             - Any speaker attributions if present\n\n\
             Write thorough, self-contained notes, preferably in short prose bullets or compact paragraphs. \
             Each note should be understandable on its own without the original transcript, and should preserve dense technical detail where present.\n\n\
             Transcript chunk:\n{chunk}"
        ),
        SummaryMode::SummaryAndFaq { language_code } => format!(
            "You are extracting detailed notes from transcript chunk {chunk_index}/{total_chunks}.\n\
             Write in language {language_code}.\n\
             Capture the major topics, technical details, named entities, decisions, open questions, examples, and likely FAQ material.\n\
             Keep the notes concise but information-dense.\n\n\
             Transcript chunk:\n{chunk}"
        ),
    }
}

fn build_summary_synthesis_prompt(mode: &SummaryMode, chunk_notes: &str) -> String {
    match mode {
        SummaryMode::SummaryOnly { user_instructions } => format!(
            "You are writing the final summary of a transcript from the extracted chunk notes below.\n\n\
             User instructions (follow these exactly — including language, structure, and formatting preferences):\n\
             {user_instructions}\n\n\
             Requirements for the final summary:\n\
             - Produce a dense, polished document — not a terse recap or a sparse outline.\n\
             - Cover all major subjects discussed in the transcript with enough depth that a reader \
             who has not heard the original audio would understand the goals, reasoning, evidence, and outcomes.\n\
             - Preserve specific details that matter: names, numbers, dates, technical terms, examples, constraints, and decisions.\n\
             - Explain how topics relate to one another instead of listing them in isolation.\n\
             - When the transcript is technical, keep the technical content explicit and accurate rather than generalizing it away.\n\
             - If there are debates, alternatives, uncertainties, or tradeoffs, describe them clearly.\n\
             - Maintain logical flow between topics: use transitions and group related ideas together.\n\
             - Respect the user's language, structural, and formatting preferences exactly.\n\
             - Output ONLY the summary text. Do not add meta-commentary or labels like \"Summary:\".\n\n\
             Chunk notes:\n{chunk_notes}"
        ),
        SummaryMode::SummaryAndFaq { language_code } => format!(
            "Generate in language {language_code} from the chunk notes below:\n1) Summary\n2) Exactly 3 FAQs with answers.\n\n\
             Summary requirements:\n\
             - Write a detailed, sectioned briefing note, not a terse recap.\n\
             - Cover all major topics, technical details, examples, numbers, and decisions.\n\
             - Preserve how the ideas relate to each other and explain why they matter.\n\
             - Keep the summary self-contained for a reader who has not heard the recording.\n\n\
             Format:\nSummary:\n...\nFAQs:\nQ:...\nA:...\n\n\
             Chunk notes:\n{chunk_notes}"
        ),
    }
}

fn compact_transcript_excerpt(text: &str, target_chars: usize) -> String {
    if text.chars().count() <= target_chars {
        return text.to_string();
    }
    let words = text.split_whitespace().collect::<Vec<_>>();
    if words.is_empty() {
        return String::new();
    }

    let window_count = 4usize;
    let window_target = (target_chars / window_count).max(300);
    let step = (words.len() / window_count).max(1);
    let mut excerpts = Vec::new();

    for window_index in 0..window_count {
        let start = (window_index * step).min(words.len().saturating_sub(1));
        let mut end = start;
        let mut chars = 0usize;
        while end < words.len() {
            let next = words[end].chars().count() + usize::from(end > start);
            if end > start && chars + next > window_target {
                break;
            }
            chars += next;
            end += 1;
        }
        let excerpt = words[start..end].join(" ");
        if !excerpt.trim().is_empty() {
            excerpts.push(format!("[Excerpt {}]\n{}", window_index + 1, excerpt));
        }
    }

    let fallback = truncate_chars(text, target_chars);
    let combined = excerpts.join("\n\n");
    if combined.trim().is_empty() {
        fallback
    } else {
        truncate_chars(&combined, target_chars)
    }
}

fn compute_summary_chunk_target_chars(mode: &SummaryMode, direct_budget: usize) -> usize {
    let overhead = build_chunk_note_prompt(1, 1, mode, "").chars().count();
    direct_budget
        .saturating_sub(overhead)
        .saturating_sub(400)
        .clamp(1_200, SUMMARY_CHUNK_TARGET_CHARS)
}

fn chunk_text_by_target_chars(
    text: &str,
    target_chars: usize,
    overlap_words: usize,
) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }

    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < words.len() {
        let previous_start = start;
        let mut end = start;
        let mut chars = 0usize;
        while end < words.len() {
            let word_len = words[end].chars().count() + usize::from(end > start);
            if end > start && chars + word_len > target_chars {
                break;
            }
            chars += word_len;
            end += 1;
        }
        if end == start {
            end = (start + 1).min(words.len());
        }
        chunks.push(words[start..end].join(" "));
        if end >= words.len() {
            break;
        }
        let next_start = end.saturating_sub(overlap_words);
        start = if next_start <= previous_start {
            end
        } else {
            next_start
        };
    }
    chunks
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    trimmed.chars().take(max_chars).collect::<String>()
}

fn parse_summary_faq_output(output: &str) -> SummaryFaq {
    if let Some((left, right)) = output.split_once("FAQs:") {
        SummaryFaq {
            summary: left.replace("Summary:", "").trim().to_string(),
            faqs: right.trim().to_string(),
        }
    } else {
        SummaryFaq {
            summary: output.trim().to_string(),
            faqs: String::new(),
        }
    }
}

fn is_context_window_error(error: &ApplicationError) -> bool {
    match error {
        ApplicationError::PostProcessing(message) => {
            let text = message.to_lowercase();
            text.contains("context window")
                || text.contains("model context window")
                || text.contains("context length")
                || text.contains("prompt is too long")
        }
        _ => false,
    }
}

async fn ask_with_overflow_fallback(
    enhancer: &dyn TranscriptEnhancer,
    candidates: Vec<String>,
) -> Result<String, ApplicationError> {
    let mut last_context_error: Option<ApplicationError> = None;
    for candidate in candidates {
        match enhancer.ask(&candidate).await {
            Ok(answer) => {
                let trimmed = answer.trim();
                if !trimmed.is_empty() {
                    return Ok(trimmed.to_string());
                }
            }
            Err(error) => {
                if is_context_window_error(&error) {
                    last_context_error = Some(error);
                    continue;
                }
                return Err(error);
            }
        }
    }

    Err(last_context_error.unwrap_or_else(|| {
        ApplicationError::PostProcessing(SUMMARY_CONTEXT_OVERFLOW_MESSAGE.to_string())
    }))
}

fn build_summary_cache_key(
    prepared: &PreparedSummaryContext,
    provider_label: &str,
    mode: &SummaryMode,
    index: usize,
    prompt: &str,
) -> String {
    let mut hasher = DefaultHasher::new();
    prepared.transcript_hash.hash(&mut hasher);
    provider_label.hash(&mut hasher);
    summary_mode_name(mode).hash(&mut hasher);
    index.hash(&mut hasher);
    prompt.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn summary_chunk_note_cache() -> &'static Mutex<HashMap<String, String>> {
    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn summary_mode_name(mode: &SummaryMode) -> &'static str {
    match mode {
        SummaryMode::SummaryOnly { .. } => "summary_only",
        SummaryMode::SummaryAndFaq { .. } => "summary_and_faq",
    }
}

fn log_summary_telemetry(telemetry: &SummaryTelemetry) {
    info!(
        target: "sbobino.summary",
        provider = telemetry.provider,
        mode = telemetry.mode,
        strategy = match telemetry.strategy {
            SummaryStrategy::DirectFull => "direct_full",
            SummaryStrategy::DirectCompact => "direct_compact",
            SummaryStrategy::Chunked => "chunked",
        },
        transcript_chars = telemetry.transcript_chars,
        transcript_words = telemetry.transcript_words,
        chunk_count = telemetry.chunk_count,
        direct_attempted = telemetry.direct_attempted,
        direct_skipped_reason = telemetry.direct_skipped_reason.unwrap_or("none"),
        cache_hits = telemetry.cache_hits,
        preprocess_ms = telemetry.preprocess_ms as u64,
        ai_ms = telemetry.ai_ms as u64,
        synthesis_ms = telemetry.synthesis_ms as u64,
        "adaptive transcript summary completed"
    );
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    };

    use async_trait::async_trait;

    use crate::TranscriptEnhancer;

    use super::{
        build_chunk_note_prompt, build_direct_prompt, build_summary_synthesis_prompt,
        chunk_text_by_target_chars, compact_transcript_excerpt, summarize_and_faq_adaptive,
        summarize_transcript_adaptive, ApplicationError, PreparedSummaryContext, SummaryFaq,
        SummaryMode,
    };

    struct TrackingEnhancer {
        ask_calls: AtomicUsize,
        active_calls: AtomicUsize,
        max_active_calls: AtomicUsize,
        prompts: Mutex<Vec<String>>,
        prefer_single_pass: bool,
        chunk_concurrency_limit: usize,
        fail_full_direct_attempts: AtomicUsize,
        direct_budget: usize,
    }

    impl TrackingEnhancer {
        fn new(
            prefer_single_pass: bool,
            chunk_concurrency_limit: usize,
            fail_full_direct_attempts: usize,
            direct_budget: usize,
        ) -> Self {
            Self {
                ask_calls: AtomicUsize::new(0),
                active_calls: AtomicUsize::new(0),
                max_active_calls: AtomicUsize::new(0),
                prompts: Mutex::new(Vec::new()),
                prefer_single_pass,
                chunk_concurrency_limit,
                fail_full_direct_attempts: AtomicUsize::new(fail_full_direct_attempts),
                direct_budget,
            }
        }

        fn record_peak_concurrency(&self, observed: usize) {
            let mut current = self.max_active_calls.load(Ordering::SeqCst);
            while observed > current {
                match self.max_active_calls.compare_exchange(
                    current,
                    observed,
                    Ordering::SeqCst,
                    Ordering::SeqCst,
                ) {
                    Ok(_) => break,
                    Err(actual) => current = actual,
                }
            }
        }
    }

    #[async_trait]
    impl TranscriptEnhancer for TrackingEnhancer {
        async fn optimize(
            &self,
            text: &str,
            _language_code: &str,
        ) -> Result<String, ApplicationError> {
            Ok(text.to_string())
        }

        async fn summarize_and_faq(
            &self,
            text: &str,
            _language_code: &str,
        ) -> Result<SummaryFaq, ApplicationError> {
            Ok(SummaryFaq {
                summary: text.to_string(),
                faqs: String::new(),
            })
        }

        async fn ask(&self, prompt: &str) -> Result<String, ApplicationError> {
            self.ask_calls.fetch_add(1, Ordering::SeqCst);
            self.prompts
                .lock()
                .expect("prompt log lock poisoned")
                .push(prompt.to_string());

            let active = self.active_calls.fetch_add(1, Ordering::SeqCst) + 1;
            self.record_peak_concurrency(active);
            for _ in 0..4 {
                tokio::task::yield_now().await;
            }
            self.active_calls.fetch_sub(1, Ordering::SeqCst);

            if prompt.contains("Full transcript:")
                && self.fail_full_direct_attempts.load(Ordering::SeqCst) > 0
            {
                self.fail_full_direct_attempts
                    .fetch_sub(1, Ordering::SeqCst);
                return Err(ApplicationError::PostProcessing(
                    "Foundation bridge error: Exceeded model context window size".to_string(),
                ));
            }

            if prompt.contains("FAQs:") && !prompt.contains("Chunk notes:") {
                let transcript = prompt
                    .split("Transcript:\n")
                    .nth(1)
                    .unwrap_or_default()
                    .trim();
                return Ok(format!(
                    "Summary:\nsummary::{transcript}\nFAQs:\nfaqs::{transcript}"
                ));
            }

            if prompt.contains("Chunk notes:")
                || prompt.contains("Full transcript:")
                || prompt.contains("Compact transcript view:")
            {
                Ok("final summary".to_string())
            } else {
                Ok("chunk note".to_string())
            }
        }

        fn prefers_single_pass_summary(&self) -> bool {
            self.prefer_single_pass
        }

        fn summary_chunk_concurrency_limit(&self) -> usize {
            self.chunk_concurrency_limit
        }

        fn summary_direct_prompt_char_budget(&self) -> usize {
            self.direct_budget
        }

        fn telemetry_provider_label(&self) -> &'static str {
            "tracking"
        }
    }

    #[test]
    fn prepared_context_normalizes_transcript_stats() {
        let prepared = PreparedSummaryContext::new("alpha alpha\n\nalpha beta").expect("context");
        assert!(!prepared.cleaned_transcript.is_empty());
        assert!(prepared.word_count >= 2);
        assert_eq!(prepared.transcript_hash.len(), 16);
    }

    #[test]
    fn compact_excerpt_shortens_long_transcripts() {
        let transcript = "alpha beta gamma delta epsilon zeta eta theta ".repeat(220);
        let excerpt = compact_transcript_excerpt(&transcript, 900);
        assert!(excerpt.chars().count() <= 900);
        assert!(excerpt.contains("[Excerpt"));
    }

    #[test]
    fn chunker_splits_and_progresses() {
        let input =
            "one two three four five six seven eight nine ten eleven twelve thirteen fourteen";
        let chunks = chunk_text_by_target_chars(input, 20, 2);
        assert!(chunks.len() >= 3);
        assert!(chunks.iter().all(|chunk| !chunk.trim().is_empty()));
    }

    #[test]
    fn chunker_advances_when_overlap_matches_small_tail_chunks() {
        let input =
            "one two three four five six seven eight nine ten eleven twelve thirteen fourteen";
        let chunks = chunk_text_by_target_chars(input, 20, 2);
        let unique = chunks.iter().collect::<std::collections::HashSet<_>>();

        assert_eq!(unique.len(), chunks.len());
        assert!(chunks
            .last()
            .is_some_and(|chunk| chunk.contains("thirteen fourteen")));
    }

    #[test]
    fn prompts_preserve_dense_summary_requirements() {
        let direct_prompt = build_direct_prompt(
            &SummaryMode::SummaryOnly {
                user_instructions: "Write in English with sections.".to_string(),
            },
            "Technical transcript",
        );
        assert!(direct_prompt.contains("dense, polished document"));

        let chunk_prompt = build_chunk_note_prompt(
            1,
            3,
            &SummaryMode::SummaryOnly {
                user_instructions: "Write in English with sections.".to_string(),
            },
            "Chunk transcript",
        );
        assert!(chunk_prompt.contains("technical terminology"));

        let synthesis_prompt = build_summary_synthesis_prompt(
            &SummaryMode::SummaryOnly {
                user_instructions: "Write in English with sections.".to_string(),
            },
            "Chunk 1 notes:\nDetails",
        );
        assert!(synthesis_prompt.contains("dense, polished document"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn summarize_uses_direct_strategy_for_short_transcripts() {
        let enhancer = TrackingEnhancer::new(true, 3, 0, 14_000);
        let summary = summarize_transcript_adaptive(
            &enhancer,
            "Alice reviews the roadmap and confirms the launch checklist is complete.",
            "Write a concise English summary.",
        )
        .await
        .expect("summary should succeed");

        assert_eq!(summary, "final summary");
        assert_eq!(enhancer.ask_calls.load(Ordering::SeqCst), 1);
        assert!(enhancer
            .prompts
            .lock()
            .expect("prompt log lock poisoned")
            .first()
            .is_some_and(|prompt| prompt.contains("Full transcript:")));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn summarize_uses_compact_direct_strategy_when_budget_is_tight() {
        let enhancer = TrackingEnhancer::new(true, 1, 0, 8_200);
        let transcript =
            "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu ".repeat(180);

        let summary = summarize_transcript_adaptive(
            &enhancer,
            &transcript,
            "Write a detailed English summary with sections.",
        )
        .await
        .expect("summary should succeed");

        assert_eq!(summary, "final summary");
        let prompts = enhancer.prompts.lock().expect("prompt log lock poisoned");
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].contains("Compact transcript view:"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn summarize_chunks_when_direct_budget_is_too_small() {
        let enhancer = Arc::new(TrackingEnhancer::new(false, 3, 0, 2_600));
        let transcript =
            "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu ".repeat(420);

        let summary = summarize_transcript_adaptive(
            enhancer.as_ref(),
            &transcript,
            "Write a detailed English summary with sections.",
        )
        .await
        .expect("summary should succeed");

        assert_eq!(summary, "final summary");
        assert!(enhancer.ask_calls.load(Ordering::SeqCst) >= 3);
        assert!(enhancer.max_active_calls.load(Ordering::SeqCst) > 1);
        let prompts = enhancer.prompts.lock().expect("prompt log lock poisoned");
        assert!(prompts
            .iter()
            .any(|prompt| prompt.contains("Transcript chunk:")));
        assert!(prompts.iter().any(|prompt| prompt.contains("Chunk notes:")));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn summarize_falls_back_from_direct_to_chunking_after_context_error() {
        let enhancer = Arc::new(TrackingEnhancer::new(true, 1, 1, 30_000));
        let transcript =
            "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu ".repeat(420);

        let summary = summarize_transcript_adaptive(
            enhancer.as_ref(),
            &transcript,
            "Write a detailed English summary with sections.",
        )
        .await
        .expect("summary should succeed");

        assert_eq!(summary, "final summary");
        let prompts = enhancer.prompts.lock().expect("prompt log lock poisoned");
        assert!(prompts
            .first()
            .is_some_and(|prompt| prompt.contains("Full transcript:")));
        assert!(prompts.len() >= 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn summarize_and_faq_uses_shared_adaptive_pipeline() {
        let enhancer = TrackingEnhancer::new(true, 1, 0, 14_000);
        let result = summarize_and_faq_adaptive(&enhancer, "meeting raw", "en")
            .await
            .expect("summary faq should succeed");

        assert_eq!(result.summary, "summary::meeting raw");
        assert_eq!(result.faqs, "faqs::meeting raw");
    }
}
