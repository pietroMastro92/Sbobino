import { describe, expect, it, vi } from "vitest";

import {
  formatProvisioningAssetLabel,
  formatProvisioningFailureMessage,
  ProvisioningCancelledError,
  provisioningScopeForAssetKind,
  runProvisioningAndRefresh,
  shouldShowProvisioningStatus,
  shouldOfferLocalModelsCta,
} from "./provisioningUi";

describe("provisioningUi", () => {
  it("times out a run that never emits a terminal event", async () => {
    vi.useFakeTimers();
    const unlisten = vi.fn();
    const refresh = vi.fn().mockResolvedValue(undefined);
    const run = runProvisioningAndRefresh({
      starter: vi.fn().mockResolvedValue({ started: true }),
      subscribe: vi.fn().mockResolvedValue(unlisten),
      refresh,
      timeoutMs: 100,
      timeoutMessage: "timed out",
      cancelledMessage: "cancelled",
      failureMessage: "failed",
    });

    const rejection = expect(run).rejects.toThrow("timed out");
    await vi.advanceTimersByTimeAsync(100);
    await rejection;
    expect(unlisten).toHaveBeenCalledOnce();
    expect(refresh).toHaveBeenCalledOnce();
    vi.useRealTimers();
  });

  it("preserves the provisioning error when the refresh also fails", async () => {
    await expect(
      runProvisioningAndRefresh({
        starter: vi.fn().mockRejectedValue(new Error("install failed")),
        subscribe: vi.fn().mockResolvedValue(vi.fn()),
        refresh: vi.fn().mockRejectedValue(new Error("refresh failed")),
        timeoutMs: 100,
        timeoutMessage: "timed out",
        cancelledMessage: "cancelled",
        failureMessage: "failed",
      }),
    ).rejects.toThrow("install failed");
  });

  it("times out while subscription or startup is still pending", async () => {
    vi.useFakeTimers();
    for (const pendingStep of ["subscribe", "starter"] as const) {
      const never = new Promise<never>(() => undefined);
      const run = runProvisioningAndRefresh({
        starter:
          pendingStep === "starter"
            ? vi.fn(() => never)
            : vi.fn().mockResolvedValue({ started: true }),
        subscribe:
          pendingStep === "subscribe"
            ? vi.fn(() => never)
            : vi.fn().mockResolvedValue(vi.fn()),
        refresh: vi.fn().mockResolvedValue(undefined),
        timeoutMs: 100,
        timeoutMessage: "timed out",
        cancelledMessage: "cancelled",
        failureMessage: "failed",
      });
      const rejection = expect(run).rejects.toThrow("timed out");
      await vi.advanceTimersByTimeAsync(100);
      await rejection;
    }
    vi.useRealTimers();
  });

  it("releases a subscription that resolves after the lifecycle timeout", async () => {
    vi.useFakeTimers();
    const unlisten = vi.fn();
    const run = runProvisioningAndRefresh({
      starter: vi.fn().mockResolvedValue({ started: true }),
      subscribe: vi.fn(
        () =>
          new Promise<() => void>((resolve) => {
            setTimeout(() => resolve(unlisten), 200);
          }),
      ),
      refresh: vi.fn().mockResolvedValue(undefined),
      timeoutMs: 100,
      timeoutMessage: "timed out",
      cancelledMessage: "cancelled",
      failureMessage: "failed",
    });
    const rejection = expect(run).rejects.toThrow("timed out");
    await vi.advanceTimersByTimeAsync(100);
    await rejection;
    await vi.advanceTimersByTimeAsync(100);
    expect(unlisten).toHaveBeenCalledOnce();
    vi.useRealTimers();
  });

  it("captures an immediate terminal error without an unhandled rejection", async () => {
    vi.useFakeTimers();
    let onStatus: ((event: { state: string; message: string }) => void) | undefined;
    const run = runProvisioningAndRefresh({
      subscribe: vi.fn(async (listener) => {
        onStatus = listener;
        return vi.fn();
      }),
      starter: vi.fn(async () => {
        onStatus?.({ state: "error", message: "early native error" });
        await new Promise((resolve) => setTimeout(resolve, 100));
        return { started: true };
      }),
      refresh: vi.fn().mockResolvedValue(undefined),
      timeoutMs: 5_000,
      timeoutMessage: "timed out",
      cancelledMessage: "cancelled",
      failureMessage: "failed",
    });
    const rejection = expect(run).rejects.toThrow("early native error");
    await vi.advanceTimersByTimeAsync(100);
    await rejection;
    vi.useRealTimers();
  });

  it("bounds a refresh that never settles while preserving the primary error", async () => {
    vi.useFakeTimers();
    const run = runProvisioningAndRefresh({
      starter: vi.fn().mockRejectedValue(new Error("install failed")),
      subscribe: vi.fn().mockResolvedValue(vi.fn()),
      refresh: vi.fn(() => new Promise<never>(() => undefined)),
      timeoutMs: 5_000,
      refreshTimeoutMs: 100,
      timeoutMessage: "timed out",
      cancelledMessage: "cancelled",
      failureMessage: "failed",
    });
    const rejection = expect(run).rejects.toThrow("install failed");
    await vi.advanceTimersByTimeAsync(100);
    await rejection;
    vi.useRealTimers();
  });

  it("preserves cancellation as a distinct terminal outcome", async () => {
    let onStatus: ((event: { state: string; message: string }) => void) | undefined;
    const run = runProvisioningAndRefresh({
      subscribe: vi.fn(async (listener) => {
        onStatus = listener;
        return vi.fn();
      }),
      starter: vi.fn(async () => {
        onStatus?.({ state: "cancelled", message: "cancelled by user" });
        return { started: true };
      }),
      refresh: vi.fn().mockResolvedValue(undefined),
      timeoutMs: 100,
      timeoutMessage: "timed out",
      cancelledMessage: "cancelled",
      failureMessage: "failed",
    });
    await expect(run).rejects.toBeInstanceOf(ProvisioningCancelledError);
  });

  it("subscribes before starting and accepts an immediate completion", async () => {
    let onStatus: ((event: { state: string; message: string }) => void) | undefined;
    const order: string[] = [];
    await runProvisioningAndRefresh({
      subscribe: vi.fn(async (listener) => {
        order.push("subscribe");
        onStatus = listener;
        return vi.fn();
      }),
      starter: vi.fn(async () => {
        order.push("start");
        onStatus?.({ state: "completed", message: "ready" });
        return { started: true };
      }),
      refresh: vi.fn().mockResolvedValue(undefined),
      timeoutMs: 100,
      timeoutMessage: "timed out",
      cancelledMessage: "cancelled",
      failureMessage: "failed",
    });
    expect(order).toEqual(["subscribe", "start"]);
  });
  it("formats pyannote progress labels distinctly", () => {
    expect(
      formatProvisioningAssetLabel({
        current: 1,
        total: 1,
        asset: "speech-runtime-macos-aarch64.zip",
        asset_kind: "speech_runtime",
        stage: "installed",
        percentage: 100,
      }),
    ).toBe("Installing speech-runtime-macos-aarch64");

    expect(
      formatProvisioningAssetLabel({
        current: 1,
        total: 2,
        asset: "pyannote-runtime-macos-aarch64.zip",
        asset_kind: "pyannote_runtime",
        stage: "installed",
        percentage: 50,
      }),
    ).toBe("Installing pyannote-runtime-macos-aarch64");

    expect(
      formatProvisioningAssetLabel({
        current: 2,
        total: 2,
        asset: "ggml-base.bin",
        asset_kind: "whisper_model",
        stage: "downloaded",
        percentage: 100,
      }),
    ).toBe("Downloading ggml-base.bin");
  });

  it("offers the Local Models CTA only for pyannote setup errors", () => {
    expect(
      shouldOfferLocalModelsCta(
        "Pyannote diarization runtime is not installed. Install it from Settings > Local Models.",
      ),
    ).toBe(true);
    expect(
      shouldOfferLocalModelsCta(
        "Whisper CLI is not runnable at '/usr/local/bin/whisper-cli'.",
      ),
    ).toBe(false);
  });

  it("keeps runtime, model, and pyannote operation messages in their own panel", () => {
    expect(provisioningScopeForAssetKind("speech_runtime")).toBe("runtime");
    expect(provisioningScopeForAssetKind("whisper_model")).toBe("models");
    expect(provisioningScopeForAssetKind("parakeet_model")).toBe("models");
    expect(provisioningScopeForAssetKind("pyannote_runtime")).toBe("pyannote");
  });

  it("distinguishes a failed repair that preserves the previous installation", () => {
    expect(
      formatProvisioningFailureMessage("checksum mismatch", true),
    ).toBe("Repair failed; the previous installation is still available. checksum mismatch");
    expect(formatProvisioningFailureMessage("runtime missing", false)).toBe(
      "runtime missing",
    );
  });

  it("keeps start, error, completion, and cancellation messages visible in the owning panel", () => {
    for (const message of ["Installing...", "Failed", "Ready", "Cancelled"]) {
      expect(shouldShowProvisioningStatus("runtime", "local_models", message)).toBe(true);
      expect(shouldShowProvisioningStatus("models", "local_models", message)).toBe(true);
      expect(shouldShowProvisioningStatus("pyannote", "pyannote", message)).toBe(true);
      expect(shouldShowProvisioningStatus("pyannote", "local_models", message)).toBe(false);
      expect(shouldShowProvisioningStatus("runtime", "pyannote", message)).toBe(false);
    }
  });
});
