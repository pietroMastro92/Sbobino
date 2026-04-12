import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  readDismissedUpdateVersion,
  readLastSeenAppVersion,
  readSharedUpdateSnapshot,
  shouldShowUpdateBanner,
  writeDismissedUpdateVersion,
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
    writeLastSeenAppVersion("0.1.14");
    writeDismissedUpdateVersion("0.1.15");

    expect(readLastSeenAppVersion()).toBe("0.1.14");
    expect(readDismissedUpdateVersion()).toBe("0.1.15");
  });

  it("persists the shared updater snapshot", () => {
    writeSharedUpdateSnapshot({
      updateInfo: {
        has_update: true,
        current_version: "0.1.14",
        latest_version: "0.1.15",
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
        current_version: "0.1.14",
        latest_version: "0.1.15",
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
          current_version: "0.1.14",
          latest_version: "0.1.15",
          download_url: null,
        },
        false,
        false,
        "0.1.15",
      ),
    ).toBe(false);

    expect(
      shouldShowUpdateBanner(
        {
          has_update: true,
          current_version: "0.1.14",
          latest_version: "0.1.16",
          download_url: null,
        },
        false,
        false,
        "0.1.15",
      ),
    ).toBe(true);
  });
});
