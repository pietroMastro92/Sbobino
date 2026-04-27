import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  matchesPyannoteAutoActionMarker,
  readDismissedUpdateVersion,
  readLastPyannoteAutoActionMarker,
  readLastSeenAppVersion,
  readSharedUpdateSnapshot,
  shouldShowUpdateBanner,
  writeDismissedUpdateVersion,
  writeLastPyannoteAutoActionMarker,
  writeLastSeenAppVersion,
  writeSharedUpdateSnapshot,
} from "./updateState";

describe("updateState", () => {
  beforeEach(() => {
    const storage = new Map<string, string>();
    Object.defineProperty(window, "localStorage", {
      configurable: true,
      value: {
        getItem: (key: string) => storage.get(key) ?? null,
        setItem: (key: string, value: string) => {
          storage.set(key, value);
        },
        removeItem: (key: string) => {
          storage.delete(key);
        },
        clear: () => {
          storage.clear();
        },
      },
    });
  });

  afterEach(() => {
    window.localStorage.clear();
  });

  it("persists version markers in local storage", () => {
    writeLastSeenAppVersion("0.1.16");
    writeDismissedUpdateVersion("0.1.16");
    writeLastPyannoteAutoActionMarker({
      appVersion: "0.1.16",
      trigger: "post_update",
      reasonCode: "pyannote_version_mismatch",
      expiresAt: Date.now() + 60_000,
      outcome: "pending",
    });

    expect(readLastSeenAppVersion()).toBe("0.1.16");
    expect(readDismissedUpdateVersion()).toBe("0.1.16");
    expect(readLastPyannoteAutoActionMarker()).toEqual({
      appVersion: "0.1.16",
      trigger: "post_update",
      reasonCode: "pyannote_version_mismatch",
      expiresAt: expect.any(Number),
      outcome: "pending",
    });
  });

  it("compares pyannote auto-action markers by version, trigger, and reason", () => {
    const current = {
      appVersion: "0.1.16",
      trigger: "post_update" as const,
      reasonCode: "pyannote_version_mismatch",
      expiresAt: Date.now() + 60_000,
      outcome: "pending" as const,
    };

    expect(matchesPyannoteAutoActionMarker(current, current)).toBe(true);
    expect(
      matchesPyannoteAutoActionMarker(current, {
        ...current,
        reasonCode: "pyannote_checksum_invalid",
      }),
    ).toBe(false);
    expect(
      matchesPyannoteAutoActionMarker(current, {
        ...current,
        trigger: "startup",
      }),
    ).toBe(false);
    expect(
      matchesPyannoteAutoActionMarker(current, {
        ...current,
        appVersion: "0.1.17",
      }),
    ).toBe(false);
  });

  it("does not match expired or failed pyannote auto-action markers", () => {
    const candidate = {
      appVersion: "0.1.38",
      trigger: "startup" as const,
      reasonCode: "pyannote_repair_required",
      expiresAt: Date.now() + 60_000,
      outcome: "pending" as const,
    };

    expect(
      matchesPyannoteAutoActionMarker(
        {
          ...candidate,
          expiresAt: Date.now() - 1,
        },
        candidate,
      ),
    ).toBe(false);
    expect(
      matchesPyannoteAutoActionMarker(
        {
          ...candidate,
          outcome: "failed",
        },
        candidate,
      ),
    ).toBe(false);
  });

  it("persists the shared updater snapshot", () => {
    writeSharedUpdateSnapshot({
      updateInfo: {
        has_update: true,
        current_version: "0.1.16",
        latest_version: "0.1.16",
        download_url: null,
      },
      updateSource: "native",
      statusMessage: "Downloading update...",
      checking: false,
      installing: true,
      downloadPercent: 42,
      syncedAt: 123,
    });

    expect(readSharedUpdateSnapshot()).toEqual({
      updateInfo: {
        has_update: true,
        current_version: "0.1.16",
        latest_version: "0.1.16",
        download_url: null,
      },
      updateSource: "native",
      statusMessage: "Downloading update...",
      checking: false,
      installing: true,
      downloadPercent: 42,
      syncedAt: 123,
    });
  });

  it("hides a dismissed banner until a newer version appears", () => {
    expect(
      shouldShowUpdateBanner(
        {
          has_update: true,
          current_version: "0.1.16",
          latest_version: "0.1.16",
          download_url: null,
        },
        false,
        false,
        "0.1.16",
      ),
    ).toBe(false);

    expect(
      shouldShowUpdateBanner(
        {
          has_update: true,
          current_version: "0.1.16",
          latest_version: "0.1.17",
          download_url: null,
        },
        false,
        false,
        "0.1.16",
      ),
    ).toBe(true);
  });

  it("does not resurrect the banner from a stale installing flag", () => {
    // Regression: a stale shared snapshot used to leave the banner stuck on
    // "Installing update" even when the running app already matched the
    // latest release (no update available).
    expect(
      shouldShowUpdateBanner(
        {
          has_update: false,
          current_version: "0.1.31",
          latest_version: "0.1.31",
          download_url: null,
        },
        true,
        false,
        null,
      ),
    ).toBe(false);
  });

  it("keeps the banner while a user-triggered install runs even if dismissed", () => {
    // Once the user kicked off the install we keep the progress banner
    // visible so they know the app is about to restart, but only as long as
    // an actual update is in flight.
    expect(
      shouldShowUpdateBanner(
        {
          has_update: true,
          current_version: "0.1.16",
          latest_version: "0.1.17",
          download_url: null,
        },
        true,
        false,
        "0.1.17",
      ),
    ).toBe(true);
  });
});
