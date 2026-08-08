import { describe, expect, it } from "vitest";

import {
  consumeRealtimeSavedSpeakerDetection,
  finalizeQueuedRealtimeSpeakerDetectionStop,
  registerRealtimeSpeakerDetectionBeforeStop,
  rollbackRealtimeSpeakerDetectionStop,
  type RealtimeSpeakerDetectionTracking,
} from "./realtimeSpeakerDetection";

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

describe("realtime speaker-detection stop tracking", () => {
  it("keeps the known job pending when realtime://saved arrives before stop resolves", async () => {
    let tracking: RealtimeSpeakerDetectionTracking = {
      requested: true,
      pendingJobId: null,
    };
    const stop = deferred<{ jobId: string }>();
    let pendingAtStopInvocation: string | null = null;
    let diarizationStarts = 0;

    tracking = registerRealtimeSpeakerDetectionBeforeStop(tracking, {
      saveResult: true,
      activeJobId: "live-job-1",
    });
    const awaitingStop = (async () => {
      pendingAtStopInvocation = tracking.pendingJobId;
      const result = await stop.promise;
      tracking = finalizeQueuedRealtimeSpeakerDetectionStop(tracking, {
        saveResult: true,
        jobId: result.jobId,
      });
    })();

    expect(pendingAtStopInvocation).toBe("live-job-1");
    const saved = consumeRealtimeSavedSpeakerDetection(tracking, "live-job-1");
    expect(saved.matchedPendingJob).toBe(true);
    tracking = saved.tracking;
    diarizationStarts += 1;

    stop.resolve({ jobId: "live-job-1" });
    await awaitingStop;

    expect(diarizationStarts).toBe(1);
    expect(tracking).toEqual({ requested: false, pendingJobId: null });
  });

  it("rolls back only the matching pending job when stop fails", async () => {
    let tracking = registerRealtimeSpeakerDetectionBeforeStop(
      { requested: true, pendingJobId: null },
      { saveResult: true, activeJobId: "live-job-2" },
    );
    const stop = deferred<void>();
    stop.reject(new Error("stop failed"));
    await expect(stop.promise).rejects.toThrow("stop failed");

    tracking = rollbackRealtimeSpeakerDetectionStop(tracking, "live-job-2");

    expect(tracking).toEqual({ requested: true, pendingJobId: null });
  });

  it("does not leave a pending job for disabled detection or a cancelled save", () => {
    expect(
      registerRealtimeSpeakerDetectionBeforeStop(
        { requested: false, pendingJobId: "stale-job" },
        { saveResult: true, activeJobId: "live-job-3" },
      ),
    ).toEqual({ requested: false, pendingJobId: null });
    expect(
      finalizeQueuedRealtimeSpeakerDetectionStop(
        { requested: true, pendingJobId: "live-job-3" },
        { saveResult: false, jobId: "live-job-3" },
      ),
    ).toEqual({ requested: false, pendingJobId: null });
  });
});
