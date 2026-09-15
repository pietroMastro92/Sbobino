import type { ProvisioningProgressEvent } from "../types";
import { t } from "../i18n";

export type ProvisioningScope = "runtime" | "models" | "pyannote";

type ProvisioningStatusEvent = {
  state: string;
  message: string;
};

export async function runProvisioningAndRefresh(options: {
  starter: () => Promise<{ started: boolean }>;
  subscribe: (
    onStatus: (event: ProvisioningStatusEvent) => void,
  ) => Promise<() => void>;
  refresh: () => Promise<unknown>;
  timeoutMs: number;
  timeoutMessage: string;
  cancelledMessage: string;
  failureMessage: string;
  waitForExistingRun?: boolean;
}): Promise<void> {
  let unlisten: (() => void) | undefined;
  let timeoutId: ReturnType<typeof setTimeout> | undefined;
  let primaryError: unknown;

  try {
    let resolveCompletion!: () => void;
    let rejectCompletion!: (error: Error) => void;
    const completion = new Promise<void>((resolve, reject) => {
      resolveCompletion = resolve;
      rejectCompletion = reject;
    });

    unlisten = await options.subscribe((event) => {
      if (event.state === "completed") {
        resolveCompletion();
      } else if (event.state === "cancelled") {
        rejectCompletion(new Error(event.message || options.cancelledMessage));
      } else if (event.state === "error") {
        rejectCompletion(new Error(event.message || options.failureMessage));
      }
    });

    const result = await options.starter();
    if (!result.started && !options.waitForExistingRun) {
      resolveCompletion();
    }

    const timeout = new Promise<never>((_, reject) => {
      timeoutId = setTimeout(
        () => reject(new Error(options.timeoutMessage)),
        options.timeoutMs,
      );
    });
    await Promise.race([completion, timeout]);
  } catch (error) {
    primaryError = error;
  } finally {
    if (timeoutId !== undefined) clearTimeout(timeoutId);
    unlisten?.();
    try {
      await options.refresh();
    } catch (refreshError) {
      primaryError ??= refreshError;
    }
  }

  if (primaryError) throw primaryError;
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
