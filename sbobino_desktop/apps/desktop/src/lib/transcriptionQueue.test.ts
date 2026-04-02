import { describe, expect, it } from "vitest";

import {
  buildQueuedTranscriptionJob,
  buildQueuedTranscriptionJobId,
  isQueuedTranscriptionJobId,
  replaceQueuedTranscriptionJob,
} from "./transcriptionQueue";

describe("transcriptionQueue helpers", () => {
  it("marks placeholder jobs with a dedicated prefix", () => {
    const jobId = buildQueuedTranscriptionJobId(3);

    expect(jobId).toBe("queued-start:3");
    expect(isQueuedTranscriptionJobId(jobId)).toBe(true);
    expect(isQueuedTranscriptionJobId("real-job-id")).toBe(false);
  });

  it("replaces a queued placeholder once the backend returns a real job id", () => {
    const queuedJob = buildQueuedTranscriptionJob("queued-start:1", "Queued transcription job.");
    const startedJob = {
      ...queuedJob,
      job_id: "real-job-1",
      stage: "preparing_audio" as const,
      message: "Preparing audio",
      percentage: 10,
    };

    const updated = replaceQueuedTranscriptionJob(
      [queuedJob, buildQueuedTranscriptionJob("queued-start:2", "Queued transcription job.")],
      queuedJob.job_id,
      startedJob,
    );

    expect(updated).toEqual([
      startedJob,
      buildQueuedTranscriptionJob("queued-start:2", "Queued transcription job."),
    ]);
  });
});
