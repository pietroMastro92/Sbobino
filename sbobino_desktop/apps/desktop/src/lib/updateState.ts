import type {
  PyannoteBackgroundActionTrigger,
  UpdateCheckResponse,
} from "../types";

export type SharedUpdateSource = "native" | "github" | null;

export type SharedUpdateSnapshot = {
  updateInfo: UpdateCheckResponse | null;
  updateSource: SharedUpdateSource;
  statusMessage: string | null;
  checking: boolean;
  installing: boolean;
  downloadPercent: number | null;
  syncedAt: number;
};

export const LAST_SEEN_APP_VERSION_STORAGE_KEY = "sbobino.update.lastSeenAppVersion";
export const DISMISSED_UPDATE_VERSION_STORAGE_KEY = "sbobino.update.dismissedVersion";
export const SHARED_UPDATE_STATE_STORAGE_KEY = "sbobino.update.sharedState";
export const LAST_PYANNOTE_AUTO_ACTION_STORAGE_KEY =
  "sbobino.update.lastPyannoteAutoAction";
export const PYANNOTE_AUTO_ACTION_MARKER_TTL_MS = 12 * 60 * 60 * 1000;

export type PyannoteAutoActionOutcome = "pending" | "succeeded" | "failed";

export type PyannoteAutoActionMarker = {
  appVersion: string;
  trigger: PyannoteBackgroundActionTrigger;
  reasonCode: string;
  expiresAt: number;
  outcome: PyannoteAutoActionOutcome;
};

function readStorageValue(key: string): string | null {
  if (typeof window === "undefined") {
    return null;
  }
  return window.localStorage.getItem(key);
}

function writeStorageValue(key: string, value: string | null): void {
  if (typeof window === "undefined") {
    return;
  }
  if (value === null) {
    window.localStorage.removeItem(key);
    return;
  }
  window.localStorage.setItem(key, value);
}

export function readLastSeenAppVersion(): string | null {
  return readStorageValue(LAST_SEEN_APP_VERSION_STORAGE_KEY);
}

export function writeLastSeenAppVersion(version: string | null): void {
  writeStorageValue(LAST_SEEN_APP_VERSION_STORAGE_KEY, version?.trim() || null);
}

export function readLastPyannoteAutoActionMarker(): PyannoteAutoActionMarker | null {
  const raw = readStorageValue(LAST_PYANNOTE_AUTO_ACTION_STORAGE_KEY);
  if (!raw) {
    return null;
  }

  try {
    const parsed = JSON.parse(raw) as Partial<PyannoteAutoActionMarker>;
    if (
      typeof parsed.appVersion !== "string" ||
      typeof parsed.trigger !== "string" ||
      typeof parsed.reasonCode !== "string" ||
      typeof parsed.expiresAt !== "number" ||
      !Number.isFinite(parsed.expiresAt) ||
      (parsed.outcome !== "pending" &&
        parsed.outcome !== "succeeded" &&
        parsed.outcome !== "failed")
    ) {
      return null;
    }
    return {
      appVersion: parsed.appVersion.trim(),
      trigger: parsed.trigger as PyannoteBackgroundActionTrigger,
      reasonCode: parsed.reasonCode.trim(),
      expiresAt: parsed.expiresAt,
      outcome: parsed.outcome,
    };
  } catch {
    return null;
  }
}

export function writeLastPyannoteAutoActionMarker(
  marker: PyannoteAutoActionMarker | null,
): void {
  if (!marker) {
    writeStorageValue(LAST_PYANNOTE_AUTO_ACTION_STORAGE_KEY, null);
    return;
  }

  writeStorageValue(
    LAST_PYANNOTE_AUTO_ACTION_STORAGE_KEY,
    JSON.stringify({
      appVersion: marker.appVersion.trim(),
      trigger: marker.trigger,
      reasonCode: marker.reasonCode.trim(),
      expiresAt: marker.expiresAt,
      outcome: marker.outcome,
    }),
  );
}

export function matchesPyannoteAutoActionMarker(
  marker: PyannoteAutoActionMarker | null,
  candidate: PyannoteAutoActionMarker,
): boolean {
  if (!marker) {
    return false;
  }
  if (marker.outcome !== "pending") {
    return false;
  }
  if (marker.expiresAt <= Date.now()) {
    return false;
  }
  return (
    marker.appVersion === candidate.appVersion &&
    marker.trigger === candidate.trigger &&
    marker.reasonCode === candidate.reasonCode
  );
}

export function readDismissedUpdateVersion(): string | null {
  return readStorageValue(DISMISSED_UPDATE_VERSION_STORAGE_KEY);
}

export function writeDismissedUpdateVersion(version: string | null): void {
  writeStorageValue(DISMISSED_UPDATE_VERSION_STORAGE_KEY, version?.trim() || null);
}

export function readSharedUpdateSnapshot(): SharedUpdateSnapshot | null {
  const raw = readStorageValue(SHARED_UPDATE_STATE_STORAGE_KEY);
  if (!raw) {
    return null;
  }

  try {
    return JSON.parse(raw) as SharedUpdateSnapshot;
  } catch {
    return null;
  }
}

export function writeSharedUpdateSnapshot(snapshot: SharedUpdateSnapshot): void {
  writeStorageValue(SHARED_UPDATE_STATE_STORAGE_KEY, JSON.stringify(snapshot));
}

export function shouldShowUpdateBanner(
  updateInfo: UpdateCheckResponse | null,
  installing: boolean,
  checking: boolean,
  dismissedVersion: string | null,
): boolean {
  // Banner is purely informational. It only shows when a newer version is
  // actually available and the user has not dismissed it. The `installing`
  // flag alone must never re-open the banner — historically a stale
  // `installing: true` snapshot persisted in localStorage would leave the
  // banner stuck even when the running app already matched the latest
  // release.
  if (!updateInfo?.has_update || !updateInfo.latest_version) {
    return false;
  }
  if (checking && !updateInfo.has_update) {
    return false;
  }
  if (dismissedVersion === updateInfo.latest_version) {
    // Keep the banner visible only while the user is actively driving an
    // install they kicked off before dismissing.
    return installing;
  }
  return true;
}
