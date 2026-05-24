import { describe, expect, it } from "vitest";

import { formatImproveTextError } from "./lib/improveTextError";

const translate = (key: string, fallback: string) =>
  key === "error.improveConfigureProvider"
    ? "Improve text failed: configure an AI provider in Settings > AI Services."
    : fallback;

const formatUiError = (key: string, fallback: string, error: unknown) => {
  const message =
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof error.message === "string"
      ? error.message
      : "";
  return message ? `${fallback}: ${message}` : fallback;
};

const formatAppErrorCode = (error: unknown) =>
  typeof error === "object" &&
  error !== null &&
  "code" in error &&
  typeof error.code === "string"
    ? error.code
    : null;

describe("App improve text errors", () => {
  it("shows post-processing details from optimize failures", () => {
    expect(
      formatImproveTextError(
        {
          code: "post_processing",
          message: "Exceeded model context window size while optimizing the transcript.",
        },
        translate,
        formatUiError,
        formatAppErrorCode,
      ),
    ).toContain("Exceeded model context window size while optimizing the transcript.");
  });

  it("keeps the provider configuration CTA for missing providers", () => {
    expect(
      formatImproveTextError(
        {
          code: "missing_ai_provider",
          message: "No usable AI provider is configured.",
        },
        translate,
        formatUiError,
        formatAppErrorCode,
      ),
    ).toBe(
      "Improve text failed: configure an AI provider in Settings > AI Services.",
    );
  });
});
