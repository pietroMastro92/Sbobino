import type { TranscriptArtifact } from "../types";

export type ArtifactDiarizationUiState =
  | {
      kind: "speakers_detected";
      speakerCount: number;
      speakerLabels: string[];
      error: null;
    }
  | {
      kind: "failed";
      speakerCount: 0;
      speakerLabels: [];
      error: string | null;
    }
  | {
      kind: "no_speakers_detected";
      speakerCount: 0;
      speakerLabels: [];
      error: null;
    }
  | {
      kind: "not_requested";
      speakerCount: 0;
      speakerLabels: [];
      error: null;
    }
  | {
      kind: "processing" | "interrupted";
      speakerCount: 0;
      speakerLabels: [];
      error: null;
    }
  | null;

function normalizeText(value: string | null | undefined): string | null {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

export function normalizeJobFailureMessage(
  message: string | null | undefined,
  fallback = "Transcription failed.",
): string {
  return normalizeText(message) ?? fallback;
}

export function getArtifactDiarizationUiState(
  artifact: TranscriptArtifact | null | undefined,
  speakerLabels: string[],
): ArtifactDiarizationUiState {
  if (!artifact) {
    return null;
  }

  const uniqueSpeakerLabels = Array.from(
    new Set(
      speakerLabels
        .map((value) => normalizeText(value))
        .filter((value): value is string => Boolean(value)),
    ),
  );

  if (uniqueSpeakerLabels.length > 0) {
    return {
      kind: "speakers_detected",
      speakerCount: uniqueSpeakerLabels.length,
      speakerLabels: uniqueSpeakerLabels,
      error: null,
    };
  }

  const diarizationStatus = normalizeText(
    artifact.metadata?.speaker_diarization_status,
  )?.toLowerCase();
  const diarizationError = normalizeText(artifact.metadata?.speaker_diarization_error);

  if (diarizationStatus === "failed") {
    return {
      kind: "failed",
      speakerCount: 0,
      speakerLabels: [],
      error: diarizationError,
    };
  }

  if (diarizationStatus === "completed") {
    return {
      kind: "no_speakers_detected",
      speakerCount: 0,
      speakerLabels: [],
      error: null,
    };
  }

  if (["queued", "running", "diarizing"].includes(diarizationStatus ?? "")) {
    return { kind: "processing", speakerCount: 0, speakerLabels: [], error: null };
  }

  if (diarizationStatus === "interrupted") {
    return { kind: "interrupted", speakerCount: 0, speakerLabels: [], error: null };
  }

  return {
    kind: "not_requested",
    speakerCount: 0,
    speakerLabels: [],
    error: null,
  };
}
