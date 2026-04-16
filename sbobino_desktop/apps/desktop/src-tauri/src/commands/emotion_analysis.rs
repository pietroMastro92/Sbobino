use std::{
    collections::{BTreeSet, HashMap, HashSet},
    time::Instant,
};

use futures_util::StreamExt;
use serde_json::json;
use tracing::info;

use sbobino_application::{ApplicationError, TranscriptEnhancer};
use sbobino_domain::{
    EmotionAnalysisResult, EmotionBridge, EmotionOverview, EmotionSemanticCluster,
    EmotionSemanticEdge, EmotionSemanticMap, EmotionSemanticNode, EmotionTimelineEntry,
};
use sbobino_infrastructure::AiEnhancerCandidate;

use crate::{
    ai_support::run_with_enhancer_fallback,
    commands::prepared_transcript::PreparedTranscriptContext,
};

const EMOTION_SUMMARY_OVERFLOW_MESSAGE: &str =
    "Exceeded model context window size while analyzing emotions. Try reducing transcript context or speaker/timestamp detail.";
const EMOTION_CHUNK_TARGET_WORDS: usize = 550;
const EMOTION_CHUNK_OVERLAP_WORDS: usize = 24;
const EMOTION_CHUNK_CONCURRENCY_LIMIT: usize = 3;
const EMOTION_SYNTHESIS_BUDGETS: &[usize] = &[6_000, 4_200, 2_800, 1_800, 1_200];
const MAX_SEMANTIC_NODES: usize = 16;
const MAX_BRIDGES: usize = 6;
const MAX_REFLECTION_PROMPTS: usize = 5;
const MAX_PROMPT_TIMELINE_ENTRIES: usize = 8;
const MAX_PROMPT_SEMANTIC_NODES: usize = 8;
const MAX_PROMPT_SEMANTIC_EDGES: usize = 10;
const MAX_PROMPT_CLUSTERS: usize = 3;
const MAX_PROMPT_BRIDGES: usize = 4;
const MAX_PROMPT_REFLECTION_PROMPTS: usize = 3;
const MAX_PROMPT_EVIDENCE_TEXT_CHARS: usize = 140;
const MAX_PROMPT_CLUSTER_SUMMARY_CHARS: usize = 120;

#[derive(Debug, Clone, Default)]
pub struct EmotionAnalysisOptions {
    pub language: String,
    pub include_timestamps: bool,
    pub include_speakers: bool,
    pub speaker_dynamics: bool,
    pub prompt_override: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct EmotionAnalysisInput {
    pub title: String,
    pub prepared: PreparedTranscriptContext,
}

#[derive(Debug, Clone)]
struct SegmentEmotionEvidence {
    segment_index: usize,
    time_label: Option<String>,
    start_seconds: Option<f32>,
    end_seconds: Option<f32>,
    speaker_label: Option<String>,
    text: String,
    dominant_emotions: Vec<String>,
    concept_terms: Vec<String>,
    valence_score: f32,
    intensity_score: f32,
}

#[derive(Debug, Clone)]
struct LocalEmotionAnalysis {
    overview: EmotionOverview,
    timeline: Vec<EmotionTimelineEntry>,
    semantic_map: EmotionSemanticMap,
    bridges: Vec<EmotionBridge>,
    reflection_prompts: Vec<String>,
    evidence_json: String,
}

#[derive(Debug, Clone)]
struct LanguageResources {
    stopwords: &'static [&'static str],
    negators: &'static [&'static str],
    intensifiers: &'static [&'static str],
    softeners: &'static [&'static str],
    lexicon: &'static [EmotionKeyword],
}

struct ScoringResources<'a> {
    stopwords: HashSet<&'a str>,
    negators: HashSet<&'a str>,
    intensifiers: HashSet<&'a str>,
    softeners: HashSet<&'a str>,
    lexicon_map: HashMap<&'a str, Vec<&'a EmotionKeyword>>,
}

#[derive(Debug, Clone, Copy)]
struct EmotionKeyword {
    token: &'static str,
    emotion: &'static str,
    valence: f32,
}

const STOPWORDS_LATIN: &[&str] = &[
    "a", "adesso", "agli", "ai", "al", "alla", "anche", "ancora", "avere", "by", "che", "chi",
    "con", "come", "da", "degli", "dei", "del", "della", "delle", "di", "do", "dove", "e", "ed",
    "el", "en", "era", "erano", "es", "esta", "for", "gli", "ha", "have", "i", "il", "in", "io",
    "is", "it", "la", "le", "lo", "ma", "mi", "my", "nel", "nella", "no", "non", "o", "of", "on",
    "per", "piu", "plus", "por", "que", "qui", "se", "si", "sono", "su", "the", "to", "tra", "un",
    "una", "we",
];

const NEGATORS_LATIN: &[&str] = &[
    "non", "not", "never", "nessun", "nessuna", "ni", "niente", "sin", "without", "sans",
];

const INTENSIFIERS_LATIN: &[&str] = &[
    "davvero",
    "estremamente",
    "molto",
    "really",
    "super",
    "tantissimo",
    "truly",
    "very",
];

const SOFTENERS_LATIN: &[&str] = &[
    "abbastanza",
    "fairly",
    "kindof",
    "kinda",
    "maybe",
    "perhaps",
];

const EMOTION_KEYWORDS_LATIN: &[EmotionKeyword] = &[
    EmotionKeyword {
        token: "amazing",
        emotion: "joy",
        valence: 1.0,
    },
    EmotionKeyword {
        token: "ansia",
        emotion: "fear",
        valence: -1.0,
    },
    EmotionKeyword {
        token: "anxiety",
        emotion: "fear",
        valence: -1.0,
    },
    EmotionKeyword {
        token: "arrabbiato",
        emotion: "anger",
        valence: -1.0,
    },
    EmotionKeyword {
        token: "calm",
        emotion: "trust",
        valence: 0.7,
    },
    EmotionKeyword {
        token: "calma",
        emotion: "trust",
        valence: 0.7,
    },
    EmotionKeyword {
        token: "confident",
        emotion: "trust",
        valence: 0.9,
    },
    EmotionKeyword {
        token: "contento",
        emotion: "joy",
        valence: 1.0,
    },
    EmotionKeyword {
        token: "curious",
        emotion: "anticipation",
        valence: 0.5,
    },
    EmotionKeyword {
        token: "curioso",
        emotion: "anticipation",
        valence: 0.5,
    },
    EmotionKeyword {
        token: "deluso",
        emotion: "sadness",
        valence: -0.9,
    },
    EmotionKeyword {
        token: "delusione",
        emotion: "sadness",
        valence: -0.9,
    },
    EmotionKeyword {
        token: "disappointed",
        emotion: "sadness",
        valence: -0.9,
    },
    EmotionKeyword {
        token: "excited",
        emotion: "joy",
        valence: 1.0,
    },
    EmotionKeyword {
        token: "felice",
        emotion: "joy",
        valence: 1.0,
    },
    EmotionKeyword {
        token: "focused",
        emotion: "trust",
        valence: 0.7,
    },
    EmotionKeyword {
        token: "frustrated",
        emotion: "anger",
        valence: -0.9,
    },
    EmotionKeyword {
        token: "frustrating",
        emotion: "anger",
        valence: -0.9,
    },
    EmotionKeyword {
        token: "frustrazione",
        emotion: "anger",
        valence: -0.9,
    },
    EmotionKeyword {
        token: "gioia",
        emotion: "joy",
        valence: 1.0,
    },
    EmotionKeyword {
        token: "glad",
        emotion: "joy",
        valence: 1.0,
    },
    EmotionKeyword {
        token: "happy",
        emotion: "joy",
        valence: 1.0,
    },
    EmotionKeyword {
        token: "hope",
        emotion: "anticipation",
        valence: 0.8,
    },
    EmotionKeyword {
        token: "hopeful",
        emotion: "anticipation",
        valence: 0.8,
    },
    EmotionKeyword {
        token: "preoccupa",
        emotion: "fear",
        valence: -0.8,
    },
    EmotionKeyword {
        token: "preoccupato",
        emotion: "fear",
        valence: -0.8,
    },
    EmotionKeyword {
        token: "pressing",
        emotion: "fear",
        valence: -0.6,
    },
    EmotionKeyword {
        token: "relief",
        emotion: "trust",
        valence: 0.9,
    },
    EmotionKeyword {
        token: "relieved",
        emotion: "trust",
        valence: 0.9,
    },
    EmotionKeyword {
        token: "rischio",
        emotion: "fear",
        valence: -0.7,
    },
    EmotionKeyword {
        token: "risk",
        emotion: "fear",
        valence: -0.7,
    },
    EmotionKeyword {
        token: "sad",
        emotion: "sadness",
        valence: -1.0,
    },
    EmotionKeyword {
        token: "sereno",
        emotion: "trust",
        valence: 0.7,
    },
    EmotionKeyword {
        token: "stress",
        emotion: "fear",
        valence: -0.8,
    },
    EmotionKeyword {
        token: "stressed",
        emotion: "fear",
        valence: -0.8,
    },
    EmotionKeyword {
        token: "surprised",
        emotion: "surprise",
        valence: 0.3,
    },
    EmotionKeyword {
        token: "surprise",
        emotion: "surprise",
        valence: 0.3,
    },
    EmotionKeyword {
        token: "tense",
        emotion: "fear",
        valence: -0.8,
    },
    EmotionKeyword {
        token: "teso",
        emotion: "fear",
        valence: -0.8,
    },
    EmotionKeyword {
        token: "thrilled",
        emotion: "joy",
        valence: 1.0,
    },
    EmotionKeyword {
        token: "trust",
        emotion: "trust",
        valence: 0.9,
    },
    EmotionKeyword {
        token: "worried",
        emotion: "fear",
        valence: -0.8,
    },
];

fn language_resources(language_code: &str) -> LanguageResources {
    match language_code.trim() {
        "zh" | "ja" => LanguageResources {
            stopwords: &[],
            negators: &[],
            intensifiers: &[],
            softeners: &[],
            lexicon: &[],
        },
        _ => LanguageResources {
            stopwords: STOPWORDS_LATIN,
            negators: NEGATORS_LATIN,
            intensifiers: INTENSIFIERS_LATIN,
            softeners: SOFTENERS_LATIN,
            lexicon: EMOTION_KEYWORDS_LATIN,
        },
    }
}

fn scoring_resources<'a>(resources: &'a LanguageResources) -> ScoringResources<'a> {
    let lexicon_map = resources.lexicon.iter().fold(
        HashMap::<&str, Vec<&EmotionKeyword>>::new(),
        |mut map, keyword| {
            map.entry(keyword.token).or_default().push(keyword);
            map
        },
    );

    ScoringResources {
        stopwords: resources.stopwords.iter().copied().collect::<HashSet<_>>(),
        negators: resources.negators.iter().copied().collect::<HashSet<_>>(),
        intensifiers: resources
            .intensifiers
            .iter()
            .copied()
            .collect::<HashSet<_>>(),
        softeners: resources.softeners.iter().copied().collect::<HashSet<_>>(),
        lexicon_map,
    }
}

pub async fn analyze_emotions_with_enhancers(
    enhancers: &[AiEnhancerCandidate],
    input: EmotionAnalysisInput,
    options: EmotionAnalysisOptions,
) -> Result<EmotionAnalysisResult, ApplicationError> {
    let transcript = input.prepared.ai_transcript.trim();
    if transcript.is_empty() {
        return Err(ApplicationError::Validation(
            "cannot analyze emotions for an empty transcript".to_string(),
        ));
    }

    let local = build_local_emotion_analysis(&input, &options);
    run_with_enhancer_fallback(enhancers, "emotion analysis", |enhancer| {
        let local = local.clone();
        let input = input.clone();
        let options = options.clone();
        Box::pin(async move { analyze_with_rag(enhancer, &input, &options, &local).await })
    })
    .await
}

fn build_local_emotion_analysis(
    input: &EmotionAnalysisInput,
    options: &EmotionAnalysisOptions,
) -> LocalEmotionAnalysis {
    let resources = language_resources(&options.language);
    let scoring = scoring_resources(&resources);
    let segments = collect_analysis_segments(input, options);
    let mut concept_frequency: HashMap<String, f32> = HashMap::new();
    let mut emotion_frequency: HashMap<String, f32> = HashMap::new();
    let mut timeline_entries = Vec::new();
    let mut evidence_entries = Vec::new();
    let mut previous_valence = 0.0_f32;
    let mut previous_emotion = String::new();

    for (order_index, segment) in segments.iter().enumerate() {
        let tokens = tokenize_text(&segment.text, &scoring.stopwords);
        let scored = score_segment(segment.segment_index, segment, &tokens, &scoring);
        if scored.text.trim().is_empty() {
            continue;
        }

        for token in &scored.concept_terms {
            *concept_frequency.entry(token.clone()).or_insert(0.0) += 1.0;
        }
        for emotion in &scored.dominant_emotions {
            *emotion_frequency.entry(emotion.clone()).or_insert(0.0) +=
                scored.intensity_score.max(1.0);
        }

        let mut entry = EmotionTimelineEntry {
            segment_index: scored.segment_index,
            time_label: scored.time_label.clone(),
            start_seconds: scored.start_seconds,
            end_seconds: scored.end_seconds,
            speaker_label: scored.speaker_label.clone(),
            dominant_emotions: scored.dominant_emotions.clone(),
            valence_score: round_score(scored.valence_score),
            intensity_score: round_score(scored.intensity_score),
            evidence_text: scored.text.clone(),
            shift_label: None,
        };

        let current_top = scored
            .dominant_emotions
            .first()
            .cloned()
            .unwrap_or_default();
        let valence_delta = (scored.valence_score - previous_valence).abs();
        if order_index > 0
            && (valence_delta >= 1.0
                || (!current_top.is_empty() && current_top != previous_emotion))
        {
            entry.shift_label = Some(if scored.valence_score > previous_valence {
                "rising intensity".to_string()
            } else if scored.valence_score < previous_valence {
                "cooling or concern shift".to_string()
            } else {
                "topic-linked emotion shift".to_string()
            });
        }

        previous_valence = scored.valence_score;
        previous_emotion = current_top;
        evidence_entries.push(scored);
        timeline_entries.push(entry);
    }

    let semantic_map =
        build_semantic_map(&evidence_entries, &concept_frequency, &emotion_frequency);
    let bridges = build_bridges(&evidence_entries);
    let overview = build_overview(&emotion_frequency, &timeline_entries, options);
    let reflection_prompts = build_reflection_prompts(&overview, &semantic_map, &bridges);
    let evidence_json = build_local_evidence_json(
        &overview,
        &timeline_entries,
        &semantic_map,
        &bridges,
        &reflection_prompts,
    );
    let timeline = if options.include_speakers {
        timeline_entries
    } else {
        timeline_entries
            .into_iter()
            .map(|mut entry| {
                entry.speaker_label = None;
                entry
            })
            .collect()
    };

    LocalEmotionAnalysis {
        overview,
        timeline,
        semantic_map,
        bridges,
        reflection_prompts,
        evidence_json,
    }
}

fn local_analysis_to_result(
    local: &LocalEmotionAnalysis,
    narrative_markdown: String,
) -> EmotionAnalysisResult {
    EmotionAnalysisResult {
        overview: local.overview.clone(),
        timeline: local.timeline.clone(),
        semantic_map: local.semantic_map.clone(),
        bridges: local.bridges.clone(),
        reflection_prompts: local.reflection_prompts.clone(),
        narrative_markdown,
    }
}

fn local_overflow_fallback_result(
    local: &LocalEmotionAnalysis,
    input: &EmotionAnalysisInput,
    options: &EmotionAnalysisOptions,
) -> EmotionAnalysisResult {
    local_analysis_to_result(local, build_local_narrative(local, input, options))
}

async fn analyze_with_rag(
    enhancer: &dyn TranscriptEnhancer,
    input: &EmotionAnalysisInput,
    options: &EmotionAnalysisOptions,
    local: &LocalEmotionAnalysis,
) -> Result<EmotionAnalysisResult, ApplicationError> {
    let started = Instant::now();
    let chunks = chunk_text_by_words(
        &input.prepared.ai_transcript,
        EMOTION_CHUNK_TARGET_WORDS,
        EMOTION_CHUNK_OVERLAP_WORDS,
    );

    let direct_prompt = build_direct_emotion_prompt(input, options, local, enhancer);
    let direct_prompt_chars = direct_prompt.chars().count();
    let direct_budget = enhancer.emotion_direct_prompt_char_budget().max(2_800);
    let should_skip_direct = direct_prompt_chars > direct_budget && chunks.len() > 1;
    let output = if enhancer.prefers_single_pass_summary() {
        if should_skip_direct {
            match synthesize_from_chunks(enhancer, input, options, local, &chunks).await {
                Ok(output) => output,
                Err(error) if is_context_window_error(&error) => {
                    return Ok(local_overflow_fallback_result(local, input, options));
                }
                Err(error) => return Err(error),
            }
        } else {
            match enhancer.ask(&direct_prompt).await {
                Ok(answer) if !answer.trim().is_empty() => answer,
                Ok(_) => {
                    if chunks.len() == 1 {
                        return Ok(local_analysis_to_result(
                            local,
                            build_local_narrative(local, input, options),
                        ));
                    }
                    synthesize_from_chunks(enhancer, input, options, local, &chunks).await?
                }
                Err(error) if is_context_window_error(&error) && chunks.len() > 1 => {
                    match synthesize_from_chunks(enhancer, input, options, local, &chunks).await {
                        Ok(output) => output,
                        Err(synthesis_error) if is_context_window_error(&synthesis_error) => {
                            return Ok(local_overflow_fallback_result(local, input, options));
                        }
                        Err(synthesis_error) => return Err(synthesis_error),
                    }
                }
                Err(error) if is_context_window_error(&error) => {
                    return Ok(local_overflow_fallback_result(local, input, options));
                }
                Err(error) => return Err(error),
            }
        }
    } else if chunks.len() == 1 {
        match ask_with_overflow_fallback(
            enhancer,
            vec![direct_prompt],
            EMOTION_SUMMARY_OVERFLOW_MESSAGE,
        )
        .await
        {
            Ok(output) => output,
            Err(error) if is_context_window_error(&error) => {
                return Ok(local_overflow_fallback_result(local, input, options));
            }
            Err(error) => return Err(error),
        }
    } else {
        match synthesize_from_chunks(enhancer, input, options, local, &chunks).await {
            Ok(output) => output,
            Err(error) if is_context_window_error(&error) => {
                return Ok(local_overflow_fallback_result(local, input, options));
            }
            Err(error) => return Err(error),
        }
    };

    info!(
        target: "sbobino.emotion",
        provider = enhancer.telemetry_provider_label(),
        transcript_hash = input.prepared.transcript_hash.as_str(),
        raw_transcript_chars = input.prepared.transcript.chars().count(),
        transcript_chars = input.prepared.char_count,
        transcript_words = input.prepared.word_count,
        chunk_count = chunks.len(),
        direct_prompt_chars = direct_prompt_chars,
        direct_budget = direct_budget,
        direct_skipped = should_skip_direct,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "emotion analysis completed"
    );

    Ok(parse_emotion_output(&output, local, input, options))
}

async fn synthesize_from_chunks(
    enhancer: &dyn TranscriptEnhancer,
    input: &EmotionAnalysisInput,
    options: &EmotionAnalysisOptions,
    local: &LocalEmotionAnalysis,
    chunks: &[String],
) -> Result<String, ApplicationError> {
    let total_chunks = chunks.len();
    let chunk_concurrency = chunks.len().clamp(
        1,
        enhancer
            .summary_chunk_concurrency_limit()
            .clamp(1, EMOTION_CHUNK_CONCURRENCY_LIMIT),
    );

    let chunk_notes = futures_util::stream::iter(chunks.iter().cloned().enumerate())
        .map(|(index, chunk)| async move {
            let prompt =
                build_chunk_emotion_prompt(index + 1, total_chunks, &chunk, options, local);
            let note = ask_with_overflow_fallback(
                enhancer,
                vec![
                    prompt.clone(),
                    truncate_chars(&prompt, 2200),
                    truncate_chars(&prompt, 1400),
                    truncate_chars(&prompt, 900),
                ],
                EMOTION_SUMMARY_OVERFLOW_MESSAGE,
            )
            .await?;
            Ok::<String, ApplicationError>(format!("Chunk {} notes:\n{}", index + 1, note.trim()))
        })
        .buffered(chunk_concurrency)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;

    let merged = chunk_notes.join("\n\n");
    let candidates = EMOTION_SYNTHESIS_BUDGETS
        .iter()
        .map(|budget| {
            build_synthesis_prompt(input, options, local, &truncate_chars(&merged, *budget))
        })
        .collect::<Vec<_>>();

    ask_with_overflow_fallback(enhancer, candidates, EMOTION_SUMMARY_OVERFLOW_MESSAGE).await
}

fn build_direct_emotion_prompt(
    input: &EmotionAnalysisInput,
    options: &EmotionAnalysisOptions,
    local: &LocalEmotionAnalysis,
    enhancer: &dyn TranscriptEnhancer,
) -> String {
    let extra = options
        .prompt_override
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("");
    let transcript_chars = input.prepared.ai_transcript.chars().count();
    let direct_budget = enhancer.emotion_direct_prompt_char_budget().max(2_800);
    let use_excerpt_view = transcript_chars + local.evidence_json.chars().count() > direct_budget;
    let transcript_label = if use_excerpt_view {
        "Salient transcript excerpts"
    } else {
        "Transcript"
    };
    let transcript_body = if use_excerpt_view {
        input.prepared.top_salient_excerpts(
            direct_budget
                .saturating_sub(local.evidence_json.chars().count())
                .clamp(1_200, 3_200),
            options.include_speakers || options.speaker_dynamics,
        )
    } else {
        input.prepared.ai_transcript.clone()
    };
    format!(
        "You are analyzing emotions in a transcript for reflective review.\n\
         Return valid JSON only.\n\
         Required top-level keys: overview, timeline, semantic_map, bridges, reflection_prompts, narrative_markdown.\n\
         The timeline and semantic_map may be refined, but keep them aligned with the local evidence.\n\
         Focus on emotional shifts, recurring concerns, confidence patterns, unresolved tension, and how themes connect across the transcript.\n\
         Write the narrative_markdown in {language}.\n\
         Use timestamps only when they are present in the local evidence.\n\
         Use speaker dynamics only when speaker labels are present or explicitly requested.\n\
         Additional user instructions: {extra}\n\n\
         Artifact title: {title}\n\n\
         Local evidence JSON:\n{evidence}\n\n\
         {transcript_label}:\n{transcript}",
        language = language_display_name(&options.language),
        title = input.title,
        evidence = local.evidence_json,
        transcript = transcript_body,
        transcript_label = transcript_label,
    )
}

fn build_chunk_emotion_prompt(
    index: usize,
    total: usize,
    chunk: &str,
    options: &EmotionAnalysisOptions,
    local: &LocalEmotionAnalysis,
) -> String {
    format!(
        "You are extracting emotion-analysis notes from transcript chunk {index}/{total}.\n\
         Write in {language}. Focus on emotional cues, turning points, speaker tone, stress/relief markers, and repeated concepts that connect to emotions.\n\
         Keep notes concise but specific. Include short evidence quotes or paraphrases from the chunk.\n\
         Local global evidence:\n{evidence}\n\n\
         Chunk transcript:\n{chunk}",
        language = language_display_name(&options.language),
        evidence = local.evidence_json,
    )
}

fn build_synthesis_prompt(
    input: &EmotionAnalysisInput,
    options: &EmotionAnalysisOptions,
    local: &LocalEmotionAnalysis,
    chunk_notes: &str,
) -> String {
    let extra = options
        .prompt_override
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("");
    format!(
        "You are synthesizing a final emotion analysis from local evidence and chunk notes.\n\
         Return valid JSON only.\n\
         Required top-level keys: overview, timeline, semantic_map, bridges, reflection_prompts, narrative_markdown.\n\
         Write narrative_markdown in {language}.\n\
         Keep the result grounded in evidence instead of speculation.\n\
         Additional user instructions: {extra}\n\n\
         Artifact title: {title}\n\n\
         Local evidence JSON:\n{evidence}\n\n\
         Chunk notes:\n{chunk_notes}",
        language = language_display_name(&options.language),
        title = input.title,
        evidence = local.evidence_json,
    )
}

fn parse_emotion_output(
    output: &str,
    local: &LocalEmotionAnalysis,
    input: &EmotionAnalysisInput,
    options: &EmotionAnalysisOptions,
) -> EmotionAnalysisResult {
    let cleaned = trim_json_fences(output);
    let parsed = extract_json_object(cleaned)
        .and_then(|json_text| serde_json::from_str::<EmotionAnalysisResult>(json_text).ok());

    let mut result = parsed.unwrap_or_else(|| {
        local_analysis_to_result(
            local,
            fallback_narrative_markdown(cleaned, local, input, options),
        )
    });

    if result.overview.primary_emotions.is_empty() {
        result.overview = local.overview.clone();
    }
    if result.timeline.is_empty() || result.timeline.len() < local.timeline.len() {
        result.timeline = local.timeline.clone();
    }
    if result.semantic_map.nodes.is_empty() {
        result.semantic_map = local.semantic_map.clone();
    }
    if result.bridges.is_empty() {
        result.bridges = local.bridges.clone();
    }
    if result.reflection_prompts.is_empty() {
        result.reflection_prompts = local.reflection_prompts.clone();
    }
    if result.narrative_markdown.trim().is_empty()
        || narrative_looks_like_structured_payload(&result.narrative_markdown)
    {
        result.narrative_markdown = build_local_narrative(local, input, options);
    }

    result.bridges.truncate(MAX_BRIDGES);
    result.reflection_prompts.truncate(MAX_REFLECTION_PROMPTS);
    result
}

fn fallback_narrative_markdown(
    output: &str,
    local: &LocalEmotionAnalysis,
    input: &EmotionAnalysisInput,
    options: &EmotionAnalysisOptions,
) -> String {
    let trimmed = output.trim();
    if trimmed.is_empty() || narrative_looks_like_structured_payload(trimmed) {
        build_local_narrative(local, input, options)
    } else {
        trimmed.to_string()
    }
}

fn narrative_looks_like_structured_payload(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }

    trimmed.starts_with('{')
        || trimmed.starts_with('[')
        || (trimmed.contains("\"overview\"") && trimmed.contains("\"timeline\""))
        || (trimmed.contains("\"semantic_map\"") && trimmed.contains("\"bridges\""))
        || (trimmed.contains("\"reflection_prompts\"")
            && trimmed.contains("\"narrative_markdown\""))
}

fn build_local_narrative(
    local: &LocalEmotionAnalysis,
    input: &EmotionAnalysisInput,
    options: &EmotionAnalysisOptions,
) -> String {
    let primary = if local.overview.primary_emotions.is_empty() {
        "mixed emotional signals".to_string()
    } else {
        local.overview.primary_emotions.join(", ")
    };
    let bridges = if local.bridges.is_empty() {
        "The transcript shows mostly local emotional movement without a strong long-range bridge."
            .to_string()
    } else {
        let bridge = &local.bridges[0];
        format!(
            "A recurring bridge links segment {} to segment {} around {}.",
            bridge.from_segment_index + 1,
            bridge.to_segment_index + 1,
            bridge.bridge_theme
        )
    };
    let speaker_line = if options.speaker_dynamics {
        local
            .overview
            .speaker_dynamics
            .clone()
            .unwrap_or_else(|| "Speaker dynamics are limited because the transcript has little reliable speaker separation.".to_string())
    } else {
        String::new()
    };

    format!(
        "## Emotional reading\n\n\
         The transcript \"{title}\" is mainly shaped by {primary}. {arc}\n\n\
         {bridges}\n\n\
         {speaker_line}\n\n\
         ## How to use this analysis\n\n\
         Review the timeline to see where tone shifts, then use the reflection prompts to inspect what triggered those changes and what remained unresolved.",
        title = input.title.trim(),
        primary = primary,
        arc = local.overview.emotional_arc.trim(),
        bridges = bridges,
        speaker_line = speaker_line.trim(),
    )
    .trim()
    .to_string()
}

fn build_local_evidence_json(
    overview: &EmotionOverview,
    timeline: &[EmotionTimelineEntry],
    semantic_map: &EmotionSemanticMap,
    bridges: &[EmotionBridge],
    reflection_prompts: &[String],
) -> String {
    let compact_nodes = semantic_map
        .nodes
        .iter()
        .take(MAX_PROMPT_SEMANTIC_NODES)
        .map(|node| {
            json!({
                "id": node.id,
                "label": node.label,
                "kind": node.kind,
                "weight": node.weight,
            })
        })
        .collect::<Vec<_>>();
    let compact_node_ids = compact_nodes
        .iter()
        .filter_map(|node| node.get("id").and_then(|value| value.as_str()))
        .collect::<HashSet<_>>();
    let compact_edges = semantic_map
        .edges
        .iter()
        .filter(|edge| {
            compact_node_ids.contains(edge.source.as_str())
                && compact_node_ids.contains(edge.target.as_str())
        })
        .take(MAX_PROMPT_SEMANTIC_EDGES)
        .map(|edge| {
            json!({
                "source": edge.source,
                "target": edge.target,
                "relation": edge.relation,
                "weight": edge.weight,
            })
        })
        .collect::<Vec<_>>();
    let compact_clusters = semantic_map
        .clusters
        .iter()
        .take(MAX_PROMPT_CLUSTERS)
        .map(|cluster| {
            json!({
                "id": cluster.id,
                "label": cluster.label,
                "node_ids": cluster.node_ids.iter().take(4).cloned().collect::<Vec<_>>(),
                "segment_indices": cluster.segment_indices.iter().take(4).copied().collect::<Vec<_>>(),
                "summary": truncate_chars(&cluster.summary, MAX_PROMPT_CLUSTER_SUMMARY_CHARS),
            })
        })
        .collect::<Vec<_>>();
    let compact_timeline = timeline
        .iter()
        .take(MAX_PROMPT_TIMELINE_ENTRIES)
        .map(|entry| {
            json!({
                "segment_index": entry.segment_index,
                "time_label": entry.time_label,
                "speaker_label": entry.speaker_label,
                "dominant_emotions": entry.dominant_emotions.iter().take(2).cloned().collect::<Vec<_>>(),
                "valence_score": entry.valence_score,
                "intensity_score": entry.intensity_score,
                "shift_label": entry.shift_label,
                "evidence_text": truncate_chars(&entry.evidence_text, MAX_PROMPT_EVIDENCE_TEXT_CHARS),
            })
        })
        .collect::<Vec<_>>();
    let compact_bridges = bridges
        .iter()
        .take(MAX_PROMPT_BRIDGES)
        .map(|bridge| {
            json!({
                "from_segment_index": bridge.from_segment_index,
                "to_segment_index": bridge.to_segment_index,
                "bridge_theme": bridge.bridge_theme,
                "reason": bridge.reason,
                "shared_keywords": bridge.shared_keywords.iter().take(4).cloned().collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();
    let compact_reflection_prompts = reflection_prompts
        .iter()
        .take(MAX_PROMPT_REFLECTION_PROMPTS)
        .cloned()
        .collect::<Vec<_>>();

    json!({
        "overview": overview,
        "timeline": compact_timeline,
        "semantic_map": {
            "nodes": compact_nodes,
            "edges": compact_edges,
            "clusters": compact_clusters,
        },
        "bridges": compact_bridges,
        "reflection_prompts": compact_reflection_prompts,
    })
    .to_string()
}

fn build_overview(
    emotion_frequency: &HashMap<String, f32>,
    timeline: &[EmotionTimelineEntry],
    options: &EmotionAnalysisOptions,
) -> EmotionOverview {
    let mut ranked = emotion_frequency
        .iter()
        .map(|(emotion, weight)| (emotion.clone(), *weight))
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let primary_emotions = ranked
        .into_iter()
        .take(4)
        .map(|(emotion, _)| emotion)
        .collect::<Vec<_>>();
    let arc = if timeline.is_empty() {
        "The transcript does not expose enough structured segment data to build a detailed emotional arc.".to_string()
    } else {
        let start = timeline
            .first()
            .and_then(|entry| entry.dominant_emotions.first())
            .cloned()
            .unwrap_or_else(|| "neutral".to_string());
        let end = timeline
            .last()
            .and_then(|entry| entry.dominant_emotions.first())
            .cloned()
            .unwrap_or_else(|| "neutral".to_string());
        let high_intensity = timeline
            .iter()
            .filter(|entry| entry.intensity_score >= 1.5)
            .count();
        format!(
            "The conversation opens around {start}, moves through {} notable high-intensity moments, and settles closer to {end}.",
            high_intensity
        )
    };

    let speaker_dynamics = if options.speaker_dynamics {
        let speaker_set = timeline
            .iter()
            .filter_map(|entry| entry.speaker_label.as_deref())
            .collect::<BTreeSet<_>>();
        if speaker_set.len() >= 2 {
            Some(
                "Multiple speakers contribute distinct emotional tones; compare where concern, confidence, or frustration cluster by speaker."
                    .to_string(),
            )
        } else {
            Some("Speaker dynamics are limited because the transcript exposes only one reliable speaker track.".to_string())
        }
    } else {
        None
    };

    let confidence_note = if matches!(options.language.trim(), "zh" | "ja") {
        Some("Lexical emotion cues are lighter for this language, so the interpretation leans more on transcript structure and AI synthesis.".to_string())
    } else {
        None
    };

    EmotionOverview {
        primary_emotions,
        emotional_arc: arc,
        speaker_dynamics,
        confidence_note,
    }
}

fn build_reflection_prompts(
    overview: &EmotionOverview,
    semantic_map: &EmotionSemanticMap,
    bridges: &[EmotionBridge],
) -> Vec<String> {
    let primary = overview
        .primary_emotions
        .first()
        .cloned()
        .unwrap_or_else(|| "the main emotional pattern".to_string());
    let cluster = semantic_map
        .clusters
        .first()
        .map(|value| value.label.clone())
        .unwrap_or_else(|| "the strongest recurring theme".to_string());
    let bridge = bridges
        .first()
        .map(|value| value.bridge_theme.clone())
        .unwrap_or_else(|| {
            "the connection between early and late parts of the transcript".to_string()
        });

    vec![
        format!("What seems to trigger {primary}, and is that trigger explicit or only implied in the conversation?"),
        format!("How does {cluster} influence the emotional tone of the discussion?"),
        format!("What does {bridge} reveal about unresolved concerns or priorities across the transcript?"),
        "If you re-read the most intense segment, what need, fear, or hope is being signaled underneath the words?".to_string(),
        "Which emotional shift feels most important to revisit in chat, and why?".to_string(),
    ]
}

fn build_semantic_map(
    evidence_entries: &[SegmentEmotionEvidence],
    concept_frequency: &HashMap<String, f32>,
    emotion_frequency: &HashMap<String, f32>,
) -> EmotionSemanticMap {
    let mut concept_nodes = concept_frequency
        .iter()
        .map(|(label, weight)| (label.clone(), *weight))
        .collect::<Vec<_>>();
    concept_nodes.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    concept_nodes.truncate(MAX_SEMANTIC_NODES.saturating_sub(emotion_frequency.len().min(5)));

    let mut emotion_nodes = emotion_frequency
        .iter()
        .map(|(label, weight)| (label.clone(), *weight))
        .collect::<Vec<_>>();
    emotion_nodes.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    emotion_nodes.truncate(5);

    let mut nodes = Vec::new();
    let mut selected_node_ids = HashSet::new();
    for (label, weight) in &emotion_nodes {
        let id = node_id("emotion", label);
        selected_node_ids.insert(id.clone());
        nodes.push(EmotionSemanticNode {
            id,
            label: label.clone(),
            kind: "emotion".to_string(),
            weight: round_score(*weight),
        });
    }
    for (label, weight) in &concept_nodes {
        let id = node_id("concept", label);
        selected_node_ids.insert(id.clone());
        nodes.push(EmotionSemanticNode {
            id,
            label: label.clone(),
            kind: "concept".to_string(),
            weight: round_score(*weight),
        });
    }

    let mut edge_weights: HashMap<(String, String, String), f32> = HashMap::new();
    let mut cluster_segments: HashMap<String, BTreeSet<usize>> = HashMap::new();
    for evidence in evidence_entries {
        for emotion in &evidence.dominant_emotions {
            let emotion_id = node_id("emotion", emotion);
            if !selected_node_ids.contains(&emotion_id) {
                continue;
            }
            let bucket = cluster_segments.entry(emotion.clone()).or_default();
            bucket.insert(evidence.segment_index);
            for concept in &evidence.concept_terms {
                let concept_id = node_id("concept", concept);
                if !selected_node_ids.contains(&concept_id) {
                    continue;
                }
                let pair = ordered_pair(&emotion_id, &concept_id);
                *edge_weights
                    .entry((pair.0, pair.1, "co_occurs".to_string()))
                    .or_insert(0.0) += 1.0;
            }
        }
    }

    let mut edges = edge_weights
        .into_iter()
        .map(|((source, target, relation), weight)| EmotionSemanticEdge {
            source,
            target,
            relation,
            weight: round_score(weight),
        })
        .collect::<Vec<_>>();
    edges.sort_by(|left, right| {
        right
            .weight
            .partial_cmp(&left.weight)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    edges.truncate(MAX_SEMANTIC_NODES * 2);

    let mut clusters = emotion_nodes
        .iter()
        .map(|(label, _)| EmotionSemanticCluster {
            id: format!("cluster-{}", slugify(label)),
            label: label.clone(),
            node_ids: edges
                .iter()
                .filter(|edge| edge.source == node_id("emotion", label))
                .flat_map(|edge| [edge.source.clone(), edge.target.clone()])
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            segment_indices: cluster_segments
                .get(label)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .collect(),
            summary: format!(
                "{} appears alongside the concepts most often linked to it.",
                label
            ),
        })
        .collect::<Vec<_>>();
    clusters.truncate(4);

    EmotionSemanticMap {
        nodes,
        edges,
        clusters,
    }
}

fn build_bridges(evidence_entries: &[SegmentEmotionEvidence]) -> Vec<EmotionBridge> {
    let mut bridges = Vec::new();
    for (left_index, left) in evidence_entries.iter().enumerate() {
        for right in evidence_entries.iter().skip(left_index + 2) {
            let left_terms = left
                .concept_terms
                .iter()
                .chain(left.dominant_emotions.iter())
                .cloned()
                .collect::<BTreeSet<_>>();
            let right_terms = right
                .concept_terms
                .iter()
                .chain(right.dominant_emotions.iter())
                .cloned()
                .collect::<BTreeSet<_>>();
            let shared = left_terms
                .intersection(&right_terms)
                .take(4)
                .cloned()
                .collect::<Vec<_>>();
            if shared.len() < 2 {
                continue;
            }
            bridges.push(EmotionBridge {
                from_segment_index: left.segment_index,
                to_segment_index: right.segment_index,
                bridge_theme: shared.join(", "),
                reason: "Recurring concepts and emotional cues reappear after a gap in the conversation.".to_string(),
                shared_keywords: shared,
            });
        }
    }
    bridges.sort_by(|left, right| {
        right
            .shared_keywords
            .len()
            .cmp(&left.shared_keywords.len())
            .then_with(|| {
                (right.to_segment_index - right.from_segment_index)
                    .cmp(&(left.to_segment_index - left.from_segment_index))
            })
    });
    bridges.truncate(MAX_BRIDGES);
    bridges
}

fn collect_analysis_segments(
    input: &EmotionAnalysisInput,
    options: &EmotionAnalysisOptions,
) -> Vec<SegmentEmotionEvidence> {
    let segments = input
        .prepared
        .timeline_segments
        .iter()
        .map(|value| SegmentEmotionEvidence {
            segment_index: value.source_index,
            speaker_label: if options.include_speakers || options.speaker_dynamics {
                value.speaker_label.clone()
            } else {
                None
            },
            time_label: if options.include_timestamps {
                value.time_label.clone()
            } else {
                None
            },
            start_seconds: if options.include_timestamps {
                value.start_seconds
            } else {
                None
            },
            end_seconds: if options.include_timestamps {
                value.end_seconds
            } else {
                None
            },
            text: value.text.clone(),
            dominant_emotions: Vec::new(),
            concept_terms: Vec::new(),
            valence_score: 0.0,
            intensity_score: 0.0,
        })
        .collect::<Vec<_>>();

    if !segments.is_empty() {
        return segments;
    }

    chunk_text_by_words(&input.prepared.ai_transcript, 260, 12)
        .into_iter()
        .enumerate()
        .map(|(segment_index, text)| SegmentEmotionEvidence {
            segment_index,
            time_label: None,
            start_seconds: None,
            end_seconds: None,
            speaker_label: None,
            text,
            dominant_emotions: Vec::new(),
            concept_terms: Vec::new(),
            valence_score: 0.0,
            intensity_score: 0.0,
        })
        .collect()
}

fn score_segment(
    segment_index: usize,
    segment: &SegmentEmotionEvidence,
    tokens: &[String],
    resources: &ScoringResources<'_>,
) -> SegmentEmotionEvidence {
    let mut emotion_scores: HashMap<String, f32> = HashMap::new();
    let mut valence_score = 0.0_f32;
    let mut intensity_score = 0.0_f32;
    let mut concept_terms = HashMap::<String, f32>::new();

    for (index, token) in tokens.iter().enumerate() {
        if let Some(matches) = resources.lexicon_map.get(token.as_str()) {
            let mut modifier = 1.0_f32;
            let left_window = index.saturating_sub(2);
            for context_token in tokens.iter().take(index).skip(left_window) {
                if resources.negators.contains(context_token.as_str()) {
                    modifier *= -0.75;
                } else if resources.intensifiers.contains(context_token.as_str()) {
                    modifier *= 1.35;
                } else if resources.softeners.contains(context_token.as_str()) {
                    modifier *= 0.8;
                }
            }

            for keyword in matches {
                *emotion_scores
                    .entry(keyword.emotion.to_string())
                    .or_insert(0.0) += modifier.abs();
                valence_score += keyword.valence * modifier;
                intensity_score += modifier.abs();
            }
        } else if token.chars().count() >= 4 {
            *concept_terms.entry(token.clone()).or_insert(0.0) += 1.0;
        }
    }

    let mut dominant = emotion_scores
        .into_iter()
        .filter(|(_, score)| *score > 0.5)
        .collect::<Vec<_>>();
    dominant.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let dominant_emotions = dominant
        .into_iter()
        .take(3)
        .map(|(label, _)| label)
        .collect::<Vec<_>>();

    let mut concept_terms = concept_terms.into_iter().collect::<Vec<_>>();
    concept_terms.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let concept_terms = concept_terms
        .into_iter()
        .take(4)
        .map(|(label, _)| label)
        .collect::<Vec<_>>();

    SegmentEmotionEvidence {
        segment_index,
        time_label: segment.time_label.clone(),
        start_seconds: segment.start_seconds,
        end_seconds: segment.end_seconds,
        speaker_label: segment.speaker_label.clone(),
        text: truncate_chars(segment.text.trim(), 320),
        dominant_emotions,
        concept_terms,
        valence_score: round_score(valence_score),
        intensity_score: round_score(intensity_score.max(if tokens.is_empty() {
            0.0
        } else {
            0.6
        })),
    }
}

fn chunk_text_by_words(text: &str, target_words: usize, overlap_words: usize) -> Vec<String> {
    let words = text.split_whitespace().collect::<Vec<_>>();
    if words.is_empty() {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < words.len() {
        let end = (start + target_words).min(words.len());
        chunks.push(words[start..end].join(" "));
        if end >= words.len() {
            break;
        }
        start = end.saturating_sub(overlap_words);
        if start >= end {
            start = end;
        }
    }
    chunks
}

fn tokenize_text(text: &str, stopwords: &HashSet<&str>) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric() && ch != '\'')
        .filter_map(|token| {
            let trimmed = token.trim().to_lowercase();
            if trimmed.chars().count() < 3 || stopwords.contains(trimmed.as_str()) {
                None
            } else {
                Some(trimmed)
            }
        })
        .collect()
}

fn language_display_name(language_code: &str) -> &str {
    match language_code.trim() {
        "auto" => "the same language as the transcript",
        "en" => "English",
        "it" => "Italian",
        "fr" => "French",
        "de" => "German",
        "es" => "Spanish",
        "pt" => "Portuguese",
        "zh" => "Chinese",
        "ja" => "Japanese",
        _ => "the requested language",
    }
}

fn ordered_pair(left: &str, right: &str) -> (String, String) {
    if left <= right {
        (left.to_string(), right.to_string())
    } else {
        (right.to_string(), left.to_string())
    }
}

fn node_id(kind: &str, label: &str) -> String {
    format!("{kind}:{}", slugify(label))
}

fn slugify(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect::<String>()
}

fn round_score(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

fn trim_json_fences(value: &str) -> &str {
    value
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim()
}

fn extract_json_object(value: &str) -> Option<&str> {
    if value.starts_with('{') && value.ends_with('}') {
        return Some(value);
    }

    let start = value.find('{')?;
    let end = value.rfind('}')?;
    value.get(start..=end)
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
    overflow_message: &str,
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

    if last_context_error.is_some() {
        return Err(ApplicationError::PostProcessing(
            overflow_message.to_string(),
        ));
    }

    Err(ApplicationError::PostProcessing(
        "empty response from AI provider".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        build_local_emotion_analysis, chunk_text_by_words, local_analysis_to_result,
        parse_emotion_output, EmotionAnalysisInput, EmotionAnalysisOptions,
        EMOTION_CHUNK_OVERLAP_WORDS, EMOTION_CHUNK_TARGET_WORDS, MAX_PROMPT_BRIDGES,
        MAX_PROMPT_CLUSTERS, MAX_PROMPT_REFLECTION_PROMPTS, MAX_PROMPT_SEMANTIC_EDGES,
        MAX_PROMPT_SEMANTIC_NODES, MAX_PROMPT_TIMELINE_ENTRIES,
    };
    use crate::commands::prepared_transcript::{
        ArtifactAiContextOptions, PreparedTranscriptContext, TimelineV2Document,
    };
    use sbobino_domain::EmotionAnalysisResult;

    fn make_options() -> EmotionAnalysisOptions {
        EmotionAnalysisOptions {
            language: "en".to_string(),
            include_timestamps: false,
            include_speakers: false,
            speaker_dynamics: false,
            prompt_override: None,
        }
    }

    fn make_input(
        title: &str,
        transcript: &str,
        timeline_v2_json: Option<String>,
    ) -> EmotionAnalysisInput {
        let prepared = if let Some(raw) = timeline_v2_json {
            let parsed = serde_json::from_str::<TimelineV2Document>(&raw).expect("timeline");
            let artifact = sbobino_domain::TranscriptArtifact {
                id: "artifact".to_string(),
                job_id: "job".to_string(),
                title: title.to_string(),
                kind: sbobino_domain::ArtifactKind::File,
                source_label: "/tmp/test.wav".to_string(),
                source_origin: sbobino_domain::ArtifactSourceOrigin::Imported,
                audio_available: true,
                audio_backfill_status: sbobino_domain::ArtifactAudioBackfillStatus::Imported,
                revision: 1,
                raw_transcript: transcript.to_string(),
                optimized_transcript: String::new(),
                summary: String::new(),
                faqs: String::new(),
                metadata: std::iter::once((
                    "timeline_v2".to_string(),
                    serde_json::to_string(&parsed).expect("serialize"),
                ))
                .collect(),
                parent_artifact_id: None,
                processing_engine: Some("whisper_cpp".to_string()),
                processing_model: Some("base".to_string()),
                processing_language: Some("en".to_string()),
                audio_duration_seconds: Some(12.0),
                audio_byte_size: Some(1024),
                source_external_path: None,
                whisper_options_json: None,
                diarization_settings_json: None,
                ai_provider_snapshot_json: None,
                source_fingerprint_json: None,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            };
            PreparedTranscriptContext::from_artifact(&artifact, ArtifactAiContextOptions::default())
        } else {
            PreparedTranscriptContext {
                transcript: transcript.to_string(),
                ai_transcript: transcript.to_string(),
                transcript_hash: "test".to_string(),
                char_count: transcript.chars().count(),
                word_count: transcript.split_whitespace().count(),
                timeline_segments: Vec::new(),
            }
        };

        EmotionAnalysisInput {
            title: title.to_string(),
            prepared,
        }
    }

    #[test]
    fn local_analysis_detects_emotions_and_clusters() {
        let input = make_input(
            "Sprint retro",
            "We are worried about the deadline but excited about the launch. The team feels frustrated by blockers, then relieved after the fix.",
            None,
        );
        let options = make_options();

        let local = build_local_emotion_analysis(&input, &options);
        assert!(!local.overview.primary_emotions.is_empty());
        assert!(!local.timeline.is_empty());
        assert!(!local.semantic_map.nodes.is_empty());
        assert!(!local.reflection_prompts.is_empty());
    }

    #[test]
    fn local_result_fallback_builds_markdown() {
        let input = make_input(
            "One-on-one",
            "I am really happy about the progress, but a bit worried about the risk.",
            None,
        );
        let options = make_options();

        let local = build_local_emotion_analysis(&input, &options);
        let result = local_analysis_to_result(&local, "Narrative".to_string());
        assert_eq!(result.narrative_markdown, "Narrative");
        assert!(!result.timeline.is_empty());
    }

    #[test]
    fn negators_and_intensifiers_shift_scores() {
        let plain = build_local_emotion_analysis(
            &make_input("Plain", "I am worried about the launch.", None),
            &make_options(),
        );
        let negated = build_local_emotion_analysis(
            &make_input("Negated", "I am not worried about the launch.", None),
            &make_options(),
        );
        let intensified = build_local_emotion_analysis(
            &make_input("Intensified", "I am very worried about the launch.", None),
            &make_options(),
        );

        assert!(plain.timeline[0].valence_score < 0.0);
        assert!(negated.timeline[0].valence_score > plain.timeline[0].valence_score);
        assert!(intensified.timeline[0].intensity_score > plain.timeline[0].intensity_score);
    }

    #[test]
    fn bridges_and_timestamps_follow_local_controls() {
        let input = make_input(
            "Status review",
            "unused",
            Some(
                r#"{
                    "segments": [
                        {
                            "text": "We are worried about the budget and deadline.",
                            "start_seconds": 5.0,
                            "speaker_label": "PM"
                        },
                        {
                            "words": [
                                {"text": "We", "start_seconds": 18.0, "end_seconds": 18.2},
                                {"text": "reviewed", "start_seconds": 18.2, "end_seconds": 18.5},
                                {"text": "options", "start_seconds": 18.5, "end_seconds": 18.9}
                            ],
                            "speaker_label": "ENG"
                        },
                        {
                            "text": "The team is still worried about the deadline and budget.",
                            "start_seconds": 34.0,
                            "speaker_label": "PM"
                        }
                    ]
                }"#
                .to_string(),
            ),
        );

        let local_without_timestamps = build_local_emotion_analysis(&input, &make_options());
        assert!(local_without_timestamps
            .timeline
            .iter()
            .all(|entry| entry.time_label.is_none()));
        assert!(local_without_timestamps
            .timeline
            .iter()
            .all(|entry| entry.speaker_label.is_none()));
        assert!(!local_without_timestamps.bridges.is_empty());

        let local_with_timestamps = build_local_emotion_analysis(
            &input,
            &EmotionAnalysisOptions {
                include_timestamps: true,
                include_speakers: true,
                ..make_options()
            },
        );
        assert_eq!(
            local_with_timestamps.timeline[0].time_label.as_deref(),
            Some("00:05")
        );
        assert_eq!(
            local_with_timestamps.timeline[0].speaker_label.as_deref(),
            Some("PM")
        );
        assert_eq!(
            local_with_timestamps.timeline[1].time_label.as_deref(),
            Some("00:18")
        );
    }

    #[test]
    fn emotion_result_round_trips_through_json() {
        let local = build_local_emotion_analysis(
            &make_input(
                "Retro",
                "We felt frustrated, then relieved, and finally hopeful.",
                None,
            ),
            &make_options(),
        );
        let result = local_analysis_to_result(&local, "Narrative".to_string());

        let serialized = serde_json::to_string(&result).expect("result should serialize");
        let deserialized = serde_json::from_str::<EmotionAnalysisResult>(&serialized)
            .expect("result should deserialize");

        assert_eq!(deserialized.narrative_markdown, "Narrative");
        assert_eq!(deserialized.timeline.len(), result.timeline.len());
        assert_eq!(
            deserialized.overview.primary_emotions,
            result.overview.primary_emotions
        );
    }

    #[test]
    fn emotion_chunk_budget_splits_medium_transcripts() {
        let transcript = (0..1_400)
            .map(|index| format!("word{index}"))
            .collect::<Vec<_>>()
            .join(" ");

        let chunks = chunk_text_by_words(
            &transcript,
            EMOTION_CHUNK_TARGET_WORDS,
            EMOTION_CHUNK_OVERLAP_WORDS,
        );

        assert!(chunks.len() >= 3);
        assert!(chunks.iter().all(|chunk| !chunk.trim().is_empty()));
    }

    #[test]
    fn prompt_evidence_json_is_compact() {
        let timeline_segments = (0..14)
            .map(|index| {
                format!(
                    r#"{{
                        "text": "Segment {index} discusses risk, blockers, relief, and repeated planning details that keep the evidence long enough to test compaction.",
                        "start_seconds": {start},
                        "speaker_label": "Speaker {speaker}"
                    }}"#,
                    start = index * 11,
                    speaker = index % 3
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let input = make_input(
            "Long sync",
            &"risk relief blocker plan ".repeat(900),
            Some(format!(r#"{{"segments":[{timeline_segments}]}}"#)),
        );

        let local = build_local_emotion_analysis(
            &input,
            &EmotionAnalysisOptions {
                include_timestamps: true,
                include_speakers: true,
                speaker_dynamics: true,
                ..make_options()
            },
        );

        let parsed = serde_json::from_str::<serde_json::Value>(&local.evidence_json)
            .expect("prompt evidence should parse");

        assert!(
            parsed["timeline"].as_array().expect("timeline array").len()
                <= MAX_PROMPT_TIMELINE_ENTRIES
        );
        assert!(
            parsed["semantic_map"]["nodes"]
                .as_array()
                .expect("node array")
                .len()
                <= MAX_PROMPT_SEMANTIC_NODES
        );
        assert!(
            parsed["semantic_map"]["edges"]
                .as_array()
                .expect("edge array")
                .len()
                <= MAX_PROMPT_SEMANTIC_EDGES
        );
        assert!(
            parsed["semantic_map"]["clusters"]
                .as_array()
                .expect("cluster array")
                .len()
                <= MAX_PROMPT_CLUSTERS
        );
        assert!(parsed["bridges"].as_array().expect("bridges array").len() <= MAX_PROMPT_BRIDGES);
        assert!(
            parsed["reflection_prompts"]
                .as_array()
                .expect("reflection prompt array")
                .len()
                <= MAX_PROMPT_REFLECTION_PROMPTS
        );
    }

    #[test]
    fn parsed_result_keeps_full_local_timeline_when_ai_returns_subset() {
        let timeline_segments = (0..24)
            .map(|index| {
                format!(
                    r#"{{
                        "text": "Segment {index} keeps the conversation moving through planning, concern, and coordination.",
                        "start_seconds": {start}
                    }}"#,
                    start = index * 15
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let input = make_input(
            "Full timeline",
            &"planning concern coordination ".repeat(500),
            Some(format!(r#"{{"segments":[{timeline_segments}]}}"#)),
        );
        let options = EmotionAnalysisOptions {
            include_timestamps: true,
            ..make_options()
        };
        let local = build_local_emotion_analysis(&input, &options);

        let partial_output = r#"{
            "overview": {"primary_emotions":["fear"],"emotional_arc":"Short arc","speaker_dynamics":null,"confidence_note":null},
            "timeline": [
                {"segment_index":0,"time_label":"00:00","start_seconds":0.0,"end_seconds":null,"speaker_label":null,"dominant_emotions":["fear"],"valence_score":-0.4,"intensity_score":0.8,"evidence_text":"Opening concern.","shift_label":null}
            ],
            "semantic_map": {"nodes":[],"edges":[],"clusters":[]},
            "bridges": [],
            "reflection_prompts": [],
            "narrative_markdown": "Narrative"
        }"#;

        let parsed = parse_emotion_output(partial_output, &local, &input, &options);
        assert_eq!(parsed.timeline.len(), local.timeline.len());
        assert!(parsed.timeline.len() > 20);
    }

    #[test]
    fn invalid_structured_output_falls_back_to_local_narrative() {
        let input = make_input(
            "Fallback",
            "We are worried about the launch but feel calmer after the review.",
            None,
        );
        let options = make_options();
        let local = build_local_emotion_analysis(&input, &options);
        let invalid_output = r#"{
            "overview": {
                "emotional_arc": "Broken payload",
            }
        }"#;

        let parsed = parse_emotion_output(invalid_output, &local, &input, &options);

        assert!(!parsed.narrative_markdown.trim_start().starts_with('{'));
        assert!(parsed.narrative_markdown.contains("## Emotional reading"));
    }

    #[test]
    fn structured_narrative_field_is_replaced_with_local_markdown() {
        let input = make_input(
            "Narrative cleanup",
            "The team sounds tense before becoming more confident.",
            None,
        );
        let options = make_options();
        let local = build_local_emotion_analysis(&input, &options);
        let output = r#"{
            "overview": {"primary_emotions":["tension"],"emotional_arc":"Tense to calm","speaker_dynamics":null,"confidence_note":null},
            "timeline": [],
            "semantic_map": {"nodes":[],"edges":[],"clusters":[]},
            "bridges": [],
            "reflection_prompts": [],
            "narrative_markdown": "{\"overview\":{\"primary_emotions\":[\"tension\"]}}"
        }"#;

        let parsed = parse_emotion_output(output, &local, &input, &options);

        assert!(!parsed.narrative_markdown.contains("\"overview\""));
        assert!(parsed.narrative_markdown.contains("## Emotional reading"));
    }

    #[test]
    fn local_timeline_preserves_original_source_indexes() {
        let input = make_input(
            "Source indexes",
            "placeholder",
            Some(
                r#"{
                    "segments": [
                        {"text": "Opening concern.", "start_seconds": 0.0},
                        {"text": "   "},
                        {"text": "Resolution and relief.", "start_seconds": 12.0}
                    ]
                }"#
                .to_string(),
            ),
        );
        let options = EmotionAnalysisOptions {
            include_timestamps: true,
            ..make_options()
        };

        let local = build_local_emotion_analysis(&input, &options);
        assert_eq!(local.timeline.len(), 2);
        assert_eq!(local.timeline[0].segment_index, 0);
        assert_eq!(local.timeline[1].segment_index, 2);
    }
}
