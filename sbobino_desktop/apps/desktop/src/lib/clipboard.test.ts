import { afterEach, describe, expect, it, vi } from "vitest";
import { copyTextToClipboard } from "./clipboard";

describe("copyTextToClipboard", () => {
  const originalClipboard = navigator.clipboard;
  const originalExecCommand = document.execCommand;

  afterEach(() => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: originalClipboard,
    });
    document.execCommand = originalExecCommand;
  });

  it("uses navigator.clipboard when available", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);

    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });

    await expect(copyTextToClipboard("hello export")).resolves.toBe(true);
    expect(writeText).toHaveBeenCalledWith("hello export");
  });

  it("falls back to document.execCommand when navigator.clipboard fails", async () => {
    const writeText = vi.fn().mockRejectedValue(new Error("clipboard unavailable"));
    const execCommand = vi.fn().mockReturnValue(true);

    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    document.execCommand = execCommand;

    await expect(copyTextToClipboard("fallback copy")).resolves.toBe(true);
    expect(writeText).toHaveBeenCalledWith("fallback copy");
    expect(execCommand).toHaveBeenCalledWith("copy");
  });

  it("returns false when both clipboard strategies fail", async () => {
    const writeText = vi.fn().mockRejectedValue(new Error("clipboard unavailable"));
    const execCommand = vi.fn().mockImplementation(() => {
      throw new Error("copy failed");
    });

    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    document.execCommand = execCommand;

    await expect(copyTextToClipboard("no copy")).resolves.toBe(false);
  });
});
