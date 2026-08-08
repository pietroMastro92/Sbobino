export type TranslationFn = (key: string, fallback: string) => string;
export type UiErrorFormatter = (
  key: string,
  fallback: string,
  error: unknown,
) => string;
export type AppErrorCodeFormatter = (error: unknown) => string | null;

export function formatImproveTextError(
  error: unknown,
  translate: TranslationFn,
  formatUiError: UiErrorFormatter,
  formatAppErrorCode: AppErrorCodeFormatter,
): string {
  const code = formatAppErrorCode(error);
  if (code === "missing_ai_provider" || code === "missing_api_key") {
    return translate(
      "error.improveConfigureProvider",
      "Improve text failed: configure an AI provider in Settings > AI Services.",
    );
  }

  return formatUiError("error.improveFailed", "Improve text failed", error);
}
