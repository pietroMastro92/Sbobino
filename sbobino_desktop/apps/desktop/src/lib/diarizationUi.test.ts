import { describe, expect, it } from "vitest";

import type { TranscriptArtifact } from "../types";
import {
  getArtifactDiarizationUiState,
  normalizeJobFailureMessage,
} from "./diarizationUi";

function artifact(metadata: Record<string, string> = {}): TranscriptArtifact {
  return {
    id: "artifact-1",
    job_id: "job-1",
    title: "Example",
    kind: "file",
    source_label: "/tmp/example.wav",
    source_origin: "imported",
    audio_available: true,
    audio_backfill_status: "imported",
    revision: 1,
    raw_transcript: "raw",
    optimized_transcript: "",
    summary: "",
    faqs: "",
    metadata,
    parent_artifact_id: null,
    processing_engine: "whisper_cpp",
    processing_model: "base",
    processing_language: "en",
    audio_duration_seconds: 12,
    audio_byte_size: 2048,
    created_at: "2026-01-01T00:00:00Z",
    updated_at: "2026-01-01T00:00:00Z",
  };
}

describe("normalizeJobFailureMessage", () => {
  it("preserves backend failure text when present", () => {
    expect(
      normalizeJobFailureMessage("input file not found: /tmp/missing.wav"),
    ).toBe("input file not found: /tmp/missing.wav");
  });

  it("falls back for empty failure text", () => {
    expect(normalizeJobFailureMessage("   ", "Fallback")).toBe("Fallback");
  });
});

describe("getArtifactDiarizationUiState", () => {
  it("detects labeled speakers from timeline-derived labels", () => {
    expect(
      getArtifactDiarizationUiState(artifact(), ["Speaker 2", "Speaker 1", "Speaker 2"]),
    ).toEqual({
      kind: "speakers_detected",
      speakerCount: 2,
      speakerLabels: ["Speaker 2", "Speaker 1"],
      error: null,
    });
  });

  it("reports failed diarization from artifact metadata", () => {
    expect(
      getArtifactDiarizationUiState(
        artifact({
          speaker_diarization_status: "failed",
          speaker_diarization_error: "pyannote helper failed",
        }),
        [],
      ),
    ).toEqual({
      kind: "failed",
      speakerCount: 0,
      speakerLabels: [],
      error: "pyannote helper failed",
    });
  });

  it("reports missing speaker labels as not requested when no metadata exists", () => {
    expect(getArtifactDiarizationUiState(artifact(), [])).toEqual({
      kind: "not_requested",
      speakerCount: 0,
      speakerLabels: [],
      error: null,
    });
  });

  it("reports queued and running diarization as processing", () => {
    for (const status of ["queued", "running", "diarizing"]) {
      expect(getArtifactDiarizationUiState(artifact({ speaker_diarization_status: status }), [])).toEqual({
        kind: "processing", speakerCount: 0, speakerLabels: [], error: null,
      });
    }
  });

  it("reports interrupted background diarization", () => {
    expect(getArtifactDiarizationUiState(artifact({ speaker_diarization_status: "interrupted" }), [])).toEqual({
      kind: "interrupted", speakerCount: 0, speakerLabels: [], error: null,
    });
  });
});
