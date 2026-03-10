import { describe, expect, it } from "vitest";

import {
  buildChatArtifactPayload,
  buildSummaryArtifactPayload,
  shouldAutostartSummary,
} from "./artifactAi";

describe("artifactAi helpers", () => {
  it("builds summary payloads from inspector controls", () => {
    expect(buildSummaryArtifactPayload({
      id: "artifact-1",
      language: "it",
      includeTimestamps: true,
      includeSpeakers: false,
      sections: true,
      bulletPoints: true,
      actionItems: false,
      keyPointsOnly: true,
      customPrompt: "  Focus on decisions only.  ",
    })).toEqual({
      id: "artifact-1",
      language: "it",
      include_timestamps: true,
      include_speakers: false,
      sections: true,
      bullet_points: true,
      action_items: false,
      key_points_only: true,
      custom_prompt: "Focus on decisions only.",
    });
  });

  it("normalizes empty custom summary prompts to null", () => {
    expect(buildSummaryArtifactPayload({
      id: "artifact-1",
      language: "en",
      includeTimestamps: false,
      includeSpeakers: true,
      sections: false,
      bulletPoints: false,
      actionItems: true,
      keyPointsOnly: false,
      customPrompt: "   ",
    }).custom_prompt).toBeNull();
  });

  it("builds chat payloads that preserve context toggles", () => {
    expect(buildChatArtifactPayload({
      id: "artifact-2",
      prompt: "  What were the next steps?  ",
      includeTimestamps: false,
      includeSpeakers: true,
    })).toEqual({
      id: "artifact-2",
      prompt: "What were the next steps?",
      include_timestamps: false,
      include_speakers: true,
    });
  });

  it("autostarts only once for empty summaries on ready artifacts", () => {
    expect(shouldAutostartSummary({
      enabled: true,
      artifactId: "artifact-3",
      persistedSummary: "",
      draftSummary: "",
      hasActiveJob: false,
      isGeneratingSummary: false,
      triggeredArtifactIds: new Set<string>(),
    })).toBe(true);

    expect(shouldAutostartSummary({
      enabled: true,
      artifactId: "artifact-3",
      persistedSummary: "",
      draftSummary: "",
      hasActiveJob: false,
      isGeneratingSummary: false,
      triggeredArtifactIds: new Set(["artifact-3"]),
    })).toBe(false);

    expect(shouldAutostartSummary({
      enabled: true,
      artifactId: "artifact-3",
      persistedSummary: "Existing summary",
      draftSummary: "",
      hasActiveJob: false,
      isGeneratingSummary: false,
      triggeredArtifactIds: new Set<string>(),
    })).toBe(false);
  });
});
