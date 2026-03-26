import type { ProvisioningProgressEvent } from "../types";
import { t } from "../i18n";

export function formatProvisioningAssetLabel(progress: ProvisioningProgressEvent): string {
  const asset = progress.asset.replace(/\.zip$/i, "");
  if (progress.asset_kind === "pyannote_runtime") {
    return t("provisioning.installingAsset", "Installing {asset}", { asset });
  }
  if (progress.asset_kind === "pyannote_model") {
    return t("provisioning.installingAsset", "Installing {asset}", { asset });
  }
  if (progress.asset_kind === "whisper_encoder") {
    return t("provisioning.downloadingAsset", "Downloading {asset}", { asset });
  }
  return t("provisioning.downloadingAsset", "Downloading {asset}", { asset });
}

export function shouldOfferLocalModelsCta(error: string | null | undefined): boolean {
  if (!error) return false;
  return error.toLowerCase().includes("pyannote");
}
