import { describe, expect, it } from "vitest";

import { formatProvisioningAssetLabel, shouldOfferLocalModelsCta } from "./provisioningUi";

describe("provisioningUi", () => {
  it("formats pyannote progress labels distinctly", () => {
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
});
