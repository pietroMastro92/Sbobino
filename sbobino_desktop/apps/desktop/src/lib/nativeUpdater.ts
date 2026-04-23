export const DEFAULT_NATIVE_UPDATE_INSTALL_TIMEOUT_MS = 5 * 60 * 1000;
export const MIN_NATIVE_UPDATE_INSTALL_TIMEOUT_MS = 30 * 1000;
export const MAX_NATIVE_UPDATE_INSTALL_TIMEOUT_MS = 30 * 60 * 1000;
export const NATIVE_UPDATE_TIMEOUT_CODE = "native_updater_timeout";

type NativeUpdateTimeoutError = Error & {
  code?: string;
};

export function resolveNativeUpdateInstallTimeoutMs(
  rawValue?: string | null,
): number {
  const parsed = Number.parseInt(String(rawValue ?? "").trim(), 10);
  if (!Number.isFinite(parsed) || parsed < MIN_NATIVE_UPDATE_INSTALL_TIMEOUT_MS) {
    return DEFAULT_NATIVE_UPDATE_INSTALL_TIMEOUT_MS;
  }
  return Math.min(parsed, MAX_NATIVE_UPDATE_INSTALL_TIMEOUT_MS);
}

export function createNativeUpdateTimeoutError(
  timeoutMs: number,
  stage: string,
  lastEvent: string | null,
): NativeUpdateTimeoutError {
  const error = new Error(
    `Native updater timed out after ${Math.round(timeoutMs / 1000)}s during ${stage} (last_event=${lastEvent ?? "none"})`,
  ) as NativeUpdateTimeoutError;
  error.code = NATIVE_UPDATE_TIMEOUT_CODE;
  return error;
}

export function isNativeUpdateTimeoutError(
  value: unknown,
): value is NativeUpdateTimeoutError {
  return (
    value instanceof Error &&
    (value as NativeUpdateTimeoutError).code === NATIVE_UPDATE_TIMEOUT_CODE
  );
}
