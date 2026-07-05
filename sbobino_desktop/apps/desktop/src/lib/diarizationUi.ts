import type { TranscriptArtifact } from "../types";

export type ArtifactDiarizationUiState =
  | {
      kind: "running";
      speakerCount: 0;
      speakerLabels: [];
      error: null;
      progress: number | null;
    }
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
      kind: "cancelled";
      speakerCount: 0;
      speakerLabels: [];
      error: null;
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

  const diarizationStatus = normalizeText(
    artifact.metadata?.speaker_diarization_status,
  )?.toLowerCase();
  const diarizationError = normalizeText(artifact.metadata?.speaker_diarization_error);
  const diarizationProgress = Number(
    normalizeText(artifact.metadata?.speaker_diarization_progress) ?? "",
  );

  if (diarizationStatus === "running") {
    return {
      kind: "running",
      speakerCount: 0,
      speakerLabels: [],
      error: null,
      progress: Number.isFinite(diarizationProgress)
        ? Math.max(0, Math.min(100, diarizationProgress))
        : null,
    };
  }

  if (diarizationStatus === "cancelled") {
    return {
      kind: "cancelled",
      speakerCount: 0,
      speakerLabels: [],
      error: null,
    };
  }

  if (diarizationStatus === "failed") {
    return {
      kind: "failed",
      speakerCount: 0,
      speakerLabels: [],
      error: diarizationError,
    };
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

  if (diarizationStatus === "completed") {
    return {
      kind: "no_speakers_detected",
      speakerCount: 0,
      speakerLabels: [],
      error: null,
    };
  }

  return {
    kind: "not_requested",
    speakerCount: 0,
    speakerLabels: [],
    error: null,
  };
}
