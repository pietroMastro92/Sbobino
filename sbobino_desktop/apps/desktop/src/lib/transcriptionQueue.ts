import type { JobProgress } from "../types";

export const QUEUED_TRANSCRIPTION_JOB_PREFIX = "queued-start:";

export function buildQueuedTranscriptionJobId(sequence: number): string {
  return `${QUEUED_TRANSCRIPTION_JOB_PREFIX}${sequence}`;
}

export function isQueuedTranscriptionJobId(jobId: string): boolean {
  return jobId.startsWith(QUEUED_TRANSCRIPTION_JOB_PREFIX);
}

export function buildQueuedTranscriptionJob(jobId: string, message: string): JobProgress {
  return {
    job_id: jobId,
    stage: "queued",
    message,
    percentage: 0,
    current_seconds: 0,
    total_seconds: null,
  };
}

export function replaceQueuedTranscriptionJob(
  items: JobProgress[],
  queuedJobId: string,
  startedJob: JobProgress,
): JobProgress[] {
  return items.map((item) => (item.job_id === queuedJobId ? startedJob : item));
}
