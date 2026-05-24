import type { JobProgress, JobStage } from "../types";

export const QUEUED_TRANSCRIPTION_JOB_PREFIX = "queued-start:";

export function buildQueuedTranscriptionJobId(sequence: number): string {
  return `${QUEUED_TRANSCRIPTION_JOB_PREFIX}${sequence}`;
}

export function isQueuedTranscriptionJobId(jobId: string): boolean {
  return jobId.startsWith(QUEUED_TRANSCRIPTION_JOB_PREFIX);
}

export function buildQueuedTranscriptionJob(
  jobId: string,
  message: string,
): JobProgress {
  return {
    job_id: jobId,
    stage: "queued",
    message,
    percentage: 0,
    current_seconds: 0,
    total_seconds: null,
  };
}

export function buildQueuedTranscriptionJobs(
  jobIds: string[],
  message: string,
): JobProgress[] {
  return jobIds.map((jobId) => buildQueuedTranscriptionJob(jobId, message));
}

export function replaceQueuedTranscriptionJob(
  items: JobProgress[],
  queuedJobId: string,
  startedJob: JobProgress,
): JobProgress[] {
  return items.map((item) => (item.job_id === queuedJobId ? startedJob : item));
}

export function upsertQueueItem(
  items: JobProgress[],
  incoming: JobProgress,
): JobProgress[] {
  const existing = items.find((entry) => entry.job_id === incoming.job_id);
  if (!existing) {
    return [...items, incoming];
  }
  return items.map((entry) =>
    entry.job_id === incoming.job_id ? incoming : entry,
  );
}

export function isTerminalJobStage(stage: JobStage): boolean {
  return stage === "completed" || stage === "cancelled" || stage === "failed";
}

export function clearFinishedQueueItems(items: JobProgress[]): JobProgress[] {
  return items.filter((item) => !isTerminalJobStage(item.stage));
}

export type QueueSessionCounts = {
  waiting: number;
  running: number;
  completed: number;
  failed: number;
  cancelled: number;
};

export function summarizeQueueItems(items: JobProgress[]): QueueSessionCounts {
  return items.reduce<QueueSessionCounts>(
    (counts, item) => {
      if (item.stage === "queued") {
        counts.waiting += 1;
      } else if (item.stage === "completed") {
        counts.completed += 1;
      } else if (item.stage === "failed") {
        counts.failed += 1;
      } else if (item.stage === "cancelled") {
        counts.cancelled += 1;
      } else {
        counts.running += 1;
      }
      return counts;
    },
    { waiting: 0, running: 0, completed: 0, failed: 0, cancelled: 0 },
  );
}

export function markQueueItemTerminal(
  items: JobProgress[],
  jobId: string,
  stage: Extract<JobStage, "completed" | "failed" | "cancelled">,
  message: string,
): JobProgress[] {
  const existing = items.find((entry) => entry.job_id === jobId);
  return upsertQueueItem(items, {
    ...(existing ?? {
      job_id: jobId,
      current_seconds: null,
      total_seconds: null,
      percentage: 0,
    }),
    job_id: jobId,
    stage,
    message,
    percentage: 100,
  });
}

export function shouldQueueTranscriptionStart({
  activeJobId,
  isStarting,
  startInFlight,
}: {
  activeJobId: string | null;
  isStarting: boolean;
  startInFlight: boolean;
}): boolean {
  return Boolean(activeJobId || isStarting || startInFlight);
}

export function shouldFocusStartedTranscription({
  queuedPromotion,
  preserveCurrentArtifact,
}: {
  queuedPromotion: boolean;
  preserveCurrentArtifact: boolean;
}): boolean {
  return !queuedPromotion && !preserveCurrentArtifact;
}

export function shouldPreserveCurrentArtifactOnStart({
  hasActiveArtifact,
  section,
  isTrimmedAudioTranscription,
}: {
  hasActiveArtifact: boolean;
  section: string;
  isTrimmedAudioTranscription: boolean;
}): boolean {
  return hasActiveArtifact && section === "detail" && !isTrimmedAudioTranscription;
}
