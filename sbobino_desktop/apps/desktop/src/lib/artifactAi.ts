import type {
  AiCapabilityStatus,
  ChatArtifactPayload,
  EmotionAnalysisPayload,
  EmotionAnalysisResult,
  LanguageCode,
  SummarizeArtifactPayload,
  TranscriptArtifact,
} from "../types";

export const defaultSummaryControls = {
  includeTimestamps: false,
  includeSpeakers: false,
  sections: true,
  bulletPoints: false,
  actionItems: true,
  keyPointsOnly: false,
  language: "en" as LanguageCode,
};

export const defaultEmotionControls = {
  includeTimestamps: true,
  includeSpeakers: false,
  speakerDynamics: true,
  language: "en" as LanguageCode,
};

export function aiActionsAvailable(status: AiCapabilityStatus | null): boolean {
  return Boolean(status?.available);
}

export function buildSummaryArtifactPayload(params: {
  id: string;
  language: LanguageCode;
  includeTimestamps: boolean;
  includeSpeakers: boolean;
  sections: boolean;
  bulletPoints: boolean;
  actionItems: boolean;
  keyPointsOnly: boolean;
  customPrompt: string;
}): SummarizeArtifactPayload {
  const customPrompt = normalizeOptionalPrompt(params.customPrompt);

  return {
    id: params.id,
    language: params.language,
    include_timestamps: params.includeTimestamps,
    include_speakers: params.includeSpeakers,
    sections: params.sections,
    bullet_points: params.bulletPoints,
    action_items: params.actionItems,
    key_points_only: params.keyPointsOnly,
    custom_prompt: customPrompt,
  };
}

export function buildChatArtifactPayload(params: {
  id: string;
  prompt: string;
  includeTimestamps: boolean;
  includeSpeakers: boolean;
  origin?: ChatArtifactPayload["origin"];
}): ChatArtifactPayload {
  return {
    id: params.id,
    prompt: params.prompt.trim(),
    origin: params.origin,
    include_timestamps: params.includeTimestamps,
    include_speakers: params.includeSpeakers,
  };
}

export function buildEmotionAnalysisPayload(params: {
  id: string;
  language: LanguageCode;
  includeTimestamps: boolean;
  includeSpeakers: boolean;
  speakerDynamics: boolean;
}): EmotionAnalysisPayload {
  return {
    id: params.id,
    language: params.language,
    include_timestamps: params.includeTimestamps,
    include_speakers: params.includeSpeakers,
    speaker_dynamics: params.speakerDynamics,
  };
}

export function parsePersistedEmotionAnalysis(
  artifact: TranscriptArtifact | null | undefined,
): EmotionAnalysisResult | null {
  const raw = artifact?.metadata?.emotion_analysis_v1?.trim();
  if (!raw) {
    return null;
  }

  try {
    const parsed = JSON.parse(raw) as Partial<EmotionAnalysisResult> | null;
    if (!parsed || typeof parsed !== "object") {
      return null;
    }

    if (
      !parsed.overview ||
      !Array.isArray(parsed.timeline) ||
      !parsed.semantic_map ||
      !Array.isArray(parsed.bridges) ||
      !Array.isArray(parsed.reflection_prompts) ||
      typeof parsed.narrative_markdown !== "string"
    ) {
      return null;
    }

    return parsed as EmotionAnalysisResult;
  } catch {
    return null;
  }
}

export function normalizeEmotionNarrative(
  narrative: string | null | undefined,
): string {
  const trimmed = narrative?.trim() ?? "";
  if (!trimmed || looksLikeStructuredEmotionPayload(trimmed)) {
    return "";
  }

  return trimmed;
}

export function shouldAutostartSummary(params: {
  enabled: boolean;
  artifactId: string | null;
  persistedSummary: string;
  draftSummary: string;
  hasActiveJob: boolean;
  isGeneratingSummary: boolean;
  triggeredArtifactIds: ReadonlySet<string>;
}): boolean {
  if (!params.enabled || !params.artifactId || params.hasActiveJob || params.isGeneratingSummary) {
    return false;
  }

  if (params.triggeredArtifactIds.has(params.artifactId)) {
    return false;
  }

  if (params.persistedSummary.trim().length > 0 || params.draftSummary.trim().length > 0) {
    return false;
  }

  return true;
}

function normalizeOptionalPrompt(value: string): string | null {
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function looksLikeStructuredEmotionPayload(value: string): boolean {
  const trimmed = value.trim();
  if (!trimmed) {
    return false;
  }

  return (
    trimmed.startsWith("{") ||
    trimmed.startsWith("[") ||
    (trimmed.includes('"overview"') && trimmed.includes('"timeline"')) ||
    (trimmed.includes('"semantic_map"') && trimmed.includes('"bridges"')) ||
    (trimmed.includes('"reflection_prompts"') && trimmed.includes('"narrative_markdown"'))
  );
}
