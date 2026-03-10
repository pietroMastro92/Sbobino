import type {
  ChatArtifactPayload,
  LanguageCode,
  SummarizeArtifactPayload,
} from "../types";

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
}): ChatArtifactPayload {
  return {
    id: params.id,
    prompt: params.prompt.trim(),
    include_timestamps: params.includeTimestamps,
    include_speakers: params.includeSpeakers,
  };
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
