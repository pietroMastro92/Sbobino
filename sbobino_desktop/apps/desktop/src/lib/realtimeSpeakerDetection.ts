export type RealtimeSpeakerDetectionTracking = {
  requested: boolean;
  pendingJobId: string | null;
};

export function registerRealtimeSpeakerDetectionBeforeStop(
  tracking: RealtimeSpeakerDetectionTracking,
  params: { saveResult: boolean; activeJobId: string | null },
): RealtimeSpeakerDetectionTracking {
  if (!params.saveResult || !tracking.requested || !params.activeJobId) {
    return { ...tracking, pendingJobId: null };
  }

  return { ...tracking, pendingJobId: params.activeJobId };
}

export function consumeRealtimeSavedSpeakerDetection(
  tracking: RealtimeSpeakerDetectionTracking,
  savedJobId: string,
): { tracking: RealtimeSpeakerDetectionTracking; matchedPendingJob: boolean } {
  if (tracking.pendingJobId !== savedJobId) {
    return { tracking, matchedPendingJob: false };
  }

  return {
    tracking: { requested: false, pendingJobId: null },
    matchedPendingJob: true,
  };
}

export function finalizeQueuedRealtimeSpeakerDetectionStop(
  tracking: RealtimeSpeakerDetectionTracking,
  params: { saveResult: boolean; jobId: string | null },
): RealtimeSpeakerDetectionTracking {
  if (!params.saveResult || !tracking.requested || !params.jobId) {
    return { requested: false, pendingJobId: null };
  }

  return { ...tracking, pendingJobId: params.jobId };
}

export function rollbackRealtimeSpeakerDetectionStop(
  tracking: RealtimeSpeakerDetectionTracking,
  stoppingJobId: string | null,
): RealtimeSpeakerDetectionTracking {
  if (!stoppingJobId || tracking.pendingJobId !== stoppingJobId) {
    return tracking;
  }

  return { ...tracking, pendingJobId: null };
}
