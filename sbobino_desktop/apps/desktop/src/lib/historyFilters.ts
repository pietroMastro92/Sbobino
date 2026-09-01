import type { TranscriptArtifact } from "../types";

export const AUTO_IMPORT_TAGS_METADATA_KEY = "auto_import_tags_v1";

export type HistorySourceKind =
  | "automatic"
  | "manual"
  | "realtime"
  | "unassigned";

/** `source:<stable-id>` selects a specific watched source. */
export type HistorySourceFilter = "all" | HistorySourceKind | `source:${string}`;

export type HistoryStatusKind =
  | "completed"
  | "processing"
  | "failed"
  | "cancelled"
  | "unknown";

export type HistoryStatusFilter = "all" | HistoryStatusKind;

export type HistoryGeneratedOutputKind =
  | "study"
  | "meeting"
  | "summary"
  | "faqs";

export type HistoryGeneratedOutputFilter =
  | "all"
  | HistoryGeneratedOutputKind
  | "none";

export type HistoryQualityKind = "good" | "warning" | "failed" | "unknown";

export type HistoryFilterOptions = {
  source?: HistorySourceFilter;
  status?: HistoryStatusFilter;
  output?: HistoryGeneratedOutputFilter;
  /** Empty/undefined means that no tag filter is applied. */
  tag?: string | null;
};

export function sourceFilterForId(sourceId: string): HistorySourceFilter {
  return `source:${sourceId.trim()}`;
}

type HistoryArtifact = Pick<
  TranscriptArtifact,
  | "kind"
  | "source_origin"
  | "raw_transcript"
  | "optimized_transcript"
  | "summary"
  | "faqs"
  | "metadata"
>;

function normalizedMetadataValue(
  artifact: Pick<HistoryArtifact, "metadata">,
  key: string,
): string {
  return artifact.metadata?.[key]?.trim().toLowerCase() ?? "";
}

function normalizedTag(value: string): string {
  return value.trim();
}

/** Normalize labels for display while comparing duplicate labels case-insensitively. */
export function normalizeHistoryTags(values: readonly string[]): string[] {
  const result: string[] = [];
  const seen = new Set<string>();
  for (const raw of values) {
    const value = normalizedTag(raw);
    if (!value) continue;
    const key = value.toLocaleLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    result.push(value);
  }
  return result;
}

/**
 * Read the versioned automatic-import tags payload.
 *
 * V1 stores a JSON array. The object forms are accepted as a defensive
 * compatibility path for early development builds and malformed/partially
 * migrated metadata; unknown values intentionally produce no tags.
 */
export function historyTags(
  artifact: Pick<HistoryArtifact, "metadata">,
): string[] {
  const raw = artifact.metadata?.[AUTO_IMPORT_TAGS_METADATA_KEY];
  if (!raw?.trim()) return [];

  try {
    const parsed: unknown = JSON.parse(raw);
    if (Array.isArray(parsed)) {
      return normalizeHistoryTags(
        parsed.filter((value): value is string => typeof value === "string"),
      );
    }
    if (typeof parsed === "object" && parsed !== null) {
      const tags = (parsed as { tags?: unknown }).tags;
      if (Array.isArray(tags)) {
        return normalizeHistoryTags(
          tags.filter((value): value is string => typeof value === "string"),
        );
      }
    }
  } catch {
    // Metadata is user-local and older versions may have omitted/corrupted it.
  }
  return [];
}

/** Classify artifacts by their durable source identity metadata. */
export function historySourceKind(
  artifact: Pick<HistoryArtifact, "kind" | "source_origin" | "metadata">,
): HistorySourceKind {
  const metadata = artifact.metadata ?? {};
  const isRealtime =
    artifact.kind === "realtime" ||
    artifact.source_origin === "realtime" ||
    normalizedMetadataValue(artifact, "kind") === "realtime";
  if (isRealtime) return "realtime";

  const automaticSourceId = metadata.auto_import_source_id?.trim();
  const automaticSourcePath = metadata.auto_import_source_path?.trim();
  const hasAutomaticTags = historyTags(artifact).length > 0;
  if (automaticSourceId || automaticSourcePath || hasAutomaticTags) {
    return "automatic";
  }

  if (artifact.source_origin === "imported" || artifact.source_origin === "trimmed") {
    return "manual";
  }
  return "unassigned";
}

/** Return the stable watched-source identity, when the artifact came from one. */
export function historySourceId(
  artifact: Pick<HistoryArtifact, "metadata">,
): string | null {
  const sourceId = artifact.metadata?.auto_import_source_id?.trim();
  return sourceId || null;
}

const FAILURE_STATUSES = new Set(["failed", "failure", "error", "errored"]);
const CANCELLED_STATUSES = new Set(["cancelled", "canceled", "aborted"]);
const PROCESSING_STATUSES = new Set([
  "queued",
  "pending",
  "processing",
  "running",
  "transcribing",
  "optimizing",
  "summarizing",
]);
const COMPLETED_STATUSES = new Set([
  "completed",
  "complete",
  "success",
  "succeeded",
  "ready",
  "done",
]);

const STATUS_METADATA_KEYS = [
  "processing_status",
  "artifact_status",
  "job_status",
  "status",
  "state",
] as const;

function classifyStatus(value: string): HistoryStatusKind | null {
  if (FAILURE_STATUSES.has(value)) return "failed";
  if (CANCELLED_STATUSES.has(value)) return "cancelled";
  if (PROCESSING_STATUSES.has(value)) return "processing";
  if (COMPLETED_STATUSES.has(value)) return "completed";
  return null;
}

/**
 * Derive a stable status from persisted metadata, with compatibility fallback
 * for historical artifacts that predate an explicit processing status.
 */
export function historyStatus(
  artifact: Pick<HistoryArtifact, "metadata" | "raw_transcript" | "optimized_transcript">,
): HistoryStatusKind {
  for (const key of STATUS_METADATA_KEYS) {
    const status = classifyStatus(normalizedMetadataValue(artifact, key));
    if (status) return status;
  }

  if (artifact.raw_transcript.trim() || artifact.optimized_transcript.trim()) {
    return "completed";
  }
  return "unknown";
}

const QUALITY_FAILURE_KEYS = [
  "speaker_diarization_status",
  "auto_post_summary_status",
  "auto_post_faqs_status",
  "auto_post_preset_output_status",
] as const;

/** Derive quality state without conflating a failed optional post-process with a missing artifact. */
export function historyQuality(
  artifact: Pick<HistoryArtifact, "metadata" | "raw_transcript" | "optimized_transcript">,
): HistoryQualityKind {
  if (!artifact.raw_transcript.trim() && !artifact.optimized_transcript.trim()) {
    return "unknown";
  }

  let hasWarning = false;
  for (const key of QUALITY_FAILURE_KEYS) {
    const status = normalizedMetadataValue(artifact, key);
    if (FAILURE_STATUSES.has(status)) return "failed";
    if (status === "warning" || status === "degraded" || status === "disabled") {
      hasWarning = true;
    }
  }
  return hasWarning ? "warning" : "good";
}

/** Return generated output categories, preserving a deterministic display order. */
export function historyGeneratedOutputs(
  artifact: Pick<HistoryArtifact, "metadata" | "summary" | "faqs">,
): HistoryGeneratedOutputKind[] {
  const outputs: HistoryGeneratedOutputKind[] = [];
  const metadata = artifact.metadata ?? {};
  if (metadata.study_pack_v1?.trim()) outputs.push("study");
  if (metadata.meeting_intelligence_v1?.trim()) outputs.push("meeting");
  if (artifact.summary.trim()) outputs.push("summary");
  if (artifact.faqs.trim()) outputs.push("faqs");
  return outputs;
}

function matchesOutput(
  artifact: Pick<HistoryArtifact, "metadata" | "summary" | "faqs">,
  output: HistoryGeneratedOutputFilter | undefined,
): boolean {
  if (!output || output === "all") return true;
  const outputs = historyGeneratedOutputs(artifact);
  return output === "none" ? outputs.length === 0 : outputs.includes(output);
}

/** Filter history records without mutating the input list or artifacts. */
export function filterHistoryArtifacts<T extends HistoryArtifact>(
  artifacts: readonly T[],
  options: HistoryFilterOptions = {},
): T[] {
  const source = options.source ?? "all";
  const status = options.status ?? "all";
  const tag = options.tag?.trim().toLocaleLowerCase();

  return artifacts.filter((artifact) => {
    if (source !== "all") {
      if (source.startsWith("source:")) {
        const sourceId = source.slice("source:".length).trim();
        if (!sourceId || historySourceId(artifact) !== sourceId) return false;
      } else if (historySourceKind(artifact) !== source) {
        return false;
      }
    }
    if (status !== "all" && historyStatus(artifact) !== status) return false;
    if (!matchesOutput(artifact, options.output)) return false;
    if (tag && !historyTags(artifact).some((value) => value.toLocaleLowerCase() === tag)) {
      return false;
    }
    return true;
  });
}

// Verbose aliases make call sites self-documenting and preserve a small, stable
// API if the UI later renames the filter controls.
export const getHistorySourceKind = historySourceKind;
export const getHistorySourceId = historySourceId;
export const getHistoryStatus = historyStatus;
export const getHistoryTags = historyTags;
export const getHistoryGeneratedOutputs = historyGeneratedOutputs;
