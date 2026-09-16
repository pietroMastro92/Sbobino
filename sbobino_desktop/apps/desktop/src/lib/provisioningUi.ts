import type { ProvisioningProgressEvent } from "../types";
import { t } from "../i18n";

export type ProvisioningScope = "runtime" | "models" | "pyannote";

type ProvisioningStatusEvent = {
  state: string;
  message: string;
};

export class ProvisioningCancelledError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ProvisioningCancelledError";
  }
}

export async function runProvisioningAndRefresh<T>(options: {
  starter: () => Promise<{ started: boolean }>;
  subscribe: (
    onStatus: (event: ProvisioningStatusEvent) => void,
  ) => Promise<() => void>;
  refresh: () => Promise<T>;
  timeoutMs: number;
  refreshTimeoutMs?: number;
  timeoutMessage: string;
  cancelledMessage: string;
  failureMessage: string;
  waitForExistingRun?: boolean;
}): Promise<T> {
  let unlisten: (() => void) | undefined;
  let timeoutId: ReturnType<typeof setTimeout> | undefined;
  let refreshTimeoutId: ReturnType<typeof setTimeout> | undefined;
  let primaryError: unknown;
  let hasPrimaryError = false;
  let finished = false;
  let refreshValue!: T;

  try {
    let resolveCompletion!: () => void;
    let rejectCompletion!: (error: Error) => void;
    const completion = new Promise<void>((resolve, reject) => {
      resolveCompletion = resolve;
      rejectCompletion = reject;
    });
    const completionOutcome = completion.then(
      () => ({ kind: "completed" as const }),
      (error: unknown) => ({ kind: "completion-error" as const, error }),
    );
    const timeout = new Promise<never>((_, reject) => {
      timeoutId = setTimeout(
        () => reject(new Error(options.timeoutMessage)),
        options.timeoutMs,
      );
    });

    await Promise.race([
      options
        .subscribe((event) => {
          if (event.state === "completed") {
            resolveCompletion();
          } else if (event.state === "cancelled") {
            rejectCompletion(
              new ProvisioningCancelledError(
                event.message || options.cancelledMessage,
              ),
            );
          } else if (event.state === "error") {
            rejectCompletion(new Error(event.message || options.failureMessage));
          }
        })
        .then((listener) => {
          if (finished) listener();
          else unlisten = listener;
        }),
      timeout,
    ]);

    const starterOutcome = options.starter().then(
      (result) => ({ kind: "started" as const, result }),
      (error: unknown) => ({ kind: "starter-error" as const, error }),
    );
    const startResult = await Promise.race([starterOutcome, timeout]);
    if (startResult.kind === "starter-error") throw startResult.error;
    if (!startResult.result.started && !options.waitForExistingRun) {
      resolveCompletion();
    }
    const completionResult = await Promise.race([completionOutcome, timeout]);
    if (completionResult.kind === "completion-error") throw completionResult.error;
  } catch (error) {
    primaryError = error;
    hasPrimaryError = true;
  } finally {
    finished = true;
    if (timeoutId !== undefined) clearTimeout(timeoutId);
    try {
      unlisten?.();
    } catch (unlistenError) {
      if (!hasPrimaryError) {
        primaryError = unlistenError;
        hasPrimaryError = true;
      }
    }
    const refreshOutcome = Promise.resolve().then(options.refresh).then(
      (value) => ({ kind: "refreshed" as const, value }),
      (error: unknown) => ({ kind: "refresh-error" as const, error }),
    );
    const refreshTimeout = new Promise<{ kind: "refresh-timeout" }>((resolve) => {
      refreshTimeoutId = setTimeout(
        () => resolve({ kind: "refresh-timeout" }),
        options.refreshTimeoutMs ?? options.timeoutMs,
      );
    });
    const result = await Promise.race([refreshOutcome, refreshTimeout]);
    if (refreshTimeoutId !== undefined) clearTimeout(refreshTimeoutId);
    if (!hasPrimaryError && result.kind !== "refreshed") {
      primaryError =
        result.kind === "refresh-error"
          ? result.error
          : new Error(options.timeoutMessage);
      hasPrimaryError = true;
    } else if (result.kind === "refreshed") {
      refreshValue = result.value;
    }
  }

  if (hasPrimaryError) throw primaryError;
  return refreshValue;
}

export function provisioningScopeForAssetKind(
  assetKind: ProvisioningProgressEvent["asset_kind"],
): ProvisioningScope {
  if (assetKind === "speech_runtime") return "runtime";
  if (assetKind === "pyannote_runtime" || assetKind === "pyannote_model") {
    return "pyannote";
  }
  return "models";
}

export function formatProvisioningFailureMessage(
  message: string,
  previousInstallationAvailable: boolean,
): string {
  if (!previousInstallationAvailable) return message;
  return `${t(
    "provisioning.previousInstallationAvailable",
    "Repair failed; the previous installation is still available.",
  )} ${message}`.trim();
}

export function shouldShowProvisioningStatus(
  scope: ProvisioningScope | null,
  panel: "local_models" | "pyannote",
  message: string,
): boolean {
  if (!message || !scope) return false;
  return panel === "pyannote"
    ? scope === "pyannote"
    : scope === "runtime" || scope === "models";
}

export function formatProvisioningAssetLabel(progress: ProvisioningProgressEvent): string {
  const asset = progress.asset.replace(/\.zip$/i, "");
  if (progress.asset_kind === "speech_runtime") {
    return t("provisioning.installingAsset", "Installing {asset}", { asset });
  }
  if (progress.asset_kind === "pyannote_runtime") {
    return t("provisioning.installingAsset", "Installing {asset}", { asset });
  }
  if (progress.asset_kind === "pyannote_model") {
    return t("provisioning.installingAsset", "Installing {asset}", { asset });
  }
  if (progress.asset_kind === "whisper_encoder") {
    return t("provisioning.downloadingAsset", "Downloading {asset}", { asset });
  }
  if (progress.asset_kind === "parakeet_model") {
    return t("provisioning.downloadingAsset", "Downloading {asset}", { asset });
  }
  return t("provisioning.downloadingAsset", "Downloading {asset}", { asset });
}

export function shouldOfferLocalModelsCta(error: string | null | undefined): boolean {
  if (!error) return false;
  return error.toLowerCase().includes("pyannote");
}
