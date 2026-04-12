import type { UpdateCheckResponse } from "../types";

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
  if (installing) {
    return true;
  }
  if (checking && !updateInfo?.has_update) {
    return false;
  }
  if (!updateInfo?.has_update || !updateInfo.latest_version) {
    return false;
  }
  return dismissedVersion !== updateInfo.latest_version;
}
