import { describe, expect, it } from "vitest";

import {
  createNativeUpdateTimeoutError,
  DEFAULT_NATIVE_UPDATE_INSTALL_TIMEOUT_MS,
  isNativeUpdateTimeoutError,
  MAX_NATIVE_UPDATE_INSTALL_TIMEOUT_MS,
  resolveNativeUpdateInstallTimeoutMs,
} from "./nativeUpdater";

describe("nativeUpdater helpers", () => {
  it("falls back to the default timeout for empty or too-small values", () => {
    expect(resolveNativeUpdateInstallTimeoutMs(undefined)).toBe(
      DEFAULT_NATIVE_UPDATE_INSTALL_TIMEOUT_MS,
    );
    expect(resolveNativeUpdateInstallTimeoutMs("0")).toBe(
      DEFAULT_NATIVE_UPDATE_INSTALL_TIMEOUT_MS,
    );
    expect(resolveNativeUpdateInstallTimeoutMs("15000")).toBe(
      DEFAULT_NATIVE_UPDATE_INSTALL_TIMEOUT_MS,
    );
  });

  it("caps oversized timeout values", () => {
    expect(resolveNativeUpdateInstallTimeoutMs(String(MAX_NATIVE_UPDATE_INSTALL_TIMEOUT_MS * 2))).toBe(
      MAX_NATIVE_UPDATE_INSTALL_TIMEOUT_MS,
    );
  });

  it("marks updater watchdog errors with a stable code", () => {
    const error = createNativeUpdateTimeoutError(
      DEFAULT_NATIVE_UPDATE_INSTALL_TIMEOUT_MS,
      "installing",
      "Finished",
    );

    expect(isNativeUpdateTimeoutError(error)).toBe(true);
    expect(isNativeUpdateTimeoutError(new Error("generic"))).toBe(false);
  });
});
