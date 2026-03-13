import type { ProvisioningProgressEvent } from "../types";

export function formatProvisioningAssetLabel(progress: ProvisioningProgressEvent): string {
  const asset = progress.asset.replace(/\.zip$/i, "");
  if (progress.asset_kind === "pyannote_runtime") return `Installing ${asset}`;
  if (progress.asset_kind === "pyannote_model") return `Installing ${asset}`;
  if (progress.asset_kind === "whisper_encoder") return `Downloading ${asset}`;
  return `Downloading ${asset}`;
}

export function shouldOfferLocalModelsCta(error: string | null | undefined): boolean {
  if (!error) return false;
  return error.toLowerCase().includes("pyannote");
}
