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
    });

    expect(readLastSeenAppVersion()).toBe("0.1.16");
    expect(readDismissedUpdateVersion()).toBe("0.1.16");
    expect(readLastPyannoteAutoActionMarker()).toEqual({
      appVersion: "0.1.16",
      trigger: "post_update",
      reasonCode: "pyannote_version_mismatch",
    });
  });

  it("compares pyannote auto-action markers by version, trigger, and reason", () => {
    const current = {
      appVersion: "0.1.16",
      trigger: "post_update" as const,
      reasonCode: "pyannote_version_mismatch",
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
});
