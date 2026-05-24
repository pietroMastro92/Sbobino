import { describe, expect, it } from "vitest";

import {
  buildQueuedTranscriptionJob,
  buildQueuedTranscriptionJobId,
  buildQueuedTranscriptionJobs,
  clearFinishedQueueItems,
  isQueuedTranscriptionJobId,
  markQueueItemTerminal,
  replaceQueuedTranscriptionJob,
  shouldFocusStartedTranscription,
  shouldPreserveCurrentArtifactOnStart,
  shouldQueueTranscriptionStart,
  summarizeQueueItems,
  upsertQueueItem,
} from "./transcriptionQueue";

describe("transcriptionQueue helpers", () => {
  it("marks placeholder jobs with a dedicated prefix", () => {
    const jobId = buildQueuedTranscriptionJobId(3);

    expect(jobId).toBe("queued-start:3");
    expect(isQueuedTranscriptionJobId(jobId)).toBe(true);
    expect(isQueuedTranscriptionJobId("real-job-id")).toBe(false);
  });

  it("replaces a queued placeholder once the backend returns a real job id", () => {
    const queuedJob = buildQueuedTranscriptionJob(
      "queued-start:1",
      "Queued transcription job.",
    );
    const startedJob = {
      ...queuedJob,
      job_id: "real-job-1",
      stage: "preparing_audio" as const,
      message: "Preparing audio",
      percentage: 10,
    };

    const updated = replaceQueuedTranscriptionJob(
      [
        queuedJob,
        buildQueuedTranscriptionJob(
          "queued-start:2",
          "Queued transcription job.",
        ),
      ],
      queuedJob.job_id,
      startedJob,
    );

    expect(updated).toEqual([
      startedJob,
      buildQueuedTranscriptionJob(
        "queued-start:2",
        "Queued transcription job.",
      ),
    ]);
  });

  it("builds batch placeholders in FIFO order", () => {
    expect(
      buildQueuedTranscriptionJobs(
        ["queued-start:1", "queued-start:2", "queued-start:3"],
        "Queued transcription job.",
      ).map((item) => item.job_id),
    ).toEqual(["queued-start:1", "queued-start:2", "queued-start:3"]);
  });

  it("appends new jobs in FIFO order and replaces existing jobs in place", () => {
    const first = buildQueuedTranscriptionJob("queued-start:1", "Queued A");
    const second = buildQueuedTranscriptionJob("queued-start:2", "Queued B");
    const updatedFirst = {
      ...first,
      stage: "preparing_audio" as const,
      message: "Preparing A",
      percentage: 12,
    };

    expect(upsertQueueItem(upsertQueueItem([], first), second)).toEqual([
      first,
      second,
    ]);
    expect(upsertQueueItem([first, second], updatedFirst)).toEqual([
      updatedFirst,
      second,
    ]);
  });

  it("queues starts while a backend start request is synchronously in flight", () => {
    expect(
      shouldQueueTranscriptionStart({
        activeJobId: null,
        isStarting: false,
        startInFlight: true,
      }),
    ).toBe(true);
    expect(
      shouldQueueTranscriptionStart({
        activeJobId: "job-1",
        isStarting: false,
        startInFlight: false,
      }),
    ).toBe(true);
    expect(
      shouldQueueTranscriptionStart({
        activeJobId: null,
        isStarting: false,
        startInFlight: false,
      }),
    ).toBe(false);
  });

  it("focuses manual starts but keeps queued promotions in the background", () => {
    expect(
      shouldFocusStartedTranscription({
        queuedPromotion: false,
        preserveCurrentArtifact: false,
      }),
    ).toBe(true);
    expect(
      shouldFocusStartedTranscription({
        queuedPromotion: true,
        preserveCurrentArtifact: false,
      }),
    ).toBe(false);
    expect(
      shouldFocusStartedTranscription({
        queuedPromotion: false,
        preserveCurrentArtifact: true,
      }),
    ).toBe(false);
  });

  it("does not preserve the current artifact when starting a trimmed child transcription", () => {
    expect(
      shouldPreserveCurrentArtifactOnStart({
        hasActiveArtifact: true,
        section: "detail",
        isTrimmedAudioTranscription: false,
      }),
    ).toBe(true);
    expect(
      shouldPreserveCurrentArtifactOnStart({
        hasActiveArtifact: true,
        section: "detail",
        isTrimmedAudioTranscription: true,
      }),
    ).toBe(false);
  });

  it("keeps active and waiting jobs when clearing finished items", () => {
    const queued = buildQueuedTranscriptionJob("queued-start:1", "Queued");
    const running = {
      ...buildQueuedTranscriptionJob("job-1", "Running"),
      stage: "transcribing" as const,
      percentage: 24,
    };
    const completed = {
      ...buildQueuedTranscriptionJob("job-2", "Done"),
      stage: "completed" as const,
      percentage: 100,
    };
    const failed = {
      ...buildQueuedTranscriptionJob("job-3", "Failed"),
      stage: "failed" as const,
      percentage: 100,
    };

    expect(clearFinishedQueueItems([queued, running, completed, failed])).toEqual([
      queued,
      running,
    ]);
  });

  it("summarizes waiting, running, completed, failed, and cancelled jobs", () => {
    expect(
      summarizeQueueItems([
        buildQueuedTranscriptionJob("queued-start:1", "Queued"),
        {
          ...buildQueuedTranscriptionJob("job-1", "Running"),
          stage: "transcribing" as const,
        },
        {
          ...buildQueuedTranscriptionJob("job-2", "Done"),
          stage: "completed" as const,
        },
        {
          ...buildQueuedTranscriptionJob("job-3", "Failed"),
          stage: "failed" as const,
        },
        {
          ...buildQueuedTranscriptionJob("job-4", "Cancelled"),
          stage: "cancelled" as const,
        },
      ]),
    ).toEqual({
      waiting: 1,
      running: 1,
      completed: 1,
      failed: 1,
      cancelled: 1,
    });
  });

  it("marks completed jobs as visible terminal entries", () => {
    const running = {
      ...buildQueuedTranscriptionJob("job-1", "Running"),
      stage: "transcribing" as const,
      percentage: 42,
    };

    expect(
      markQueueItemTerminal([running], "job-1", "completed", "Completed."),
    ).toEqual([
      {
        ...running,
        stage: "completed",
        message: "Completed.",
        percentage: 100,
      },
    ]);
  });
});
