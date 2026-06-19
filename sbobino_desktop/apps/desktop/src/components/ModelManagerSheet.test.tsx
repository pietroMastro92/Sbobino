import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ModelManagerSheet } from "./ModelManagerSheet";

afterEach(() => {
  cleanup();
});

describe("ModelManagerSheet", () => {
  it("shows missing count and triggers actions", () => {
    const onDownloadModel = vi.fn().mockResolvedValue(undefined);
    const onDownloadAll = vi.fn().mockResolvedValue(undefined);
    const onRefresh = vi.fn().mockResolvedValue(undefined);
    const onCancel = vi.fn().mockResolvedValue(undefined);
    const onClose = vi.fn();

    render(
      <ModelManagerSheet
        open
        modelsDir="/tmp/models"
        models={[
          {
            key: "tiny",
            label: "Tiny",
            model_file: "ggml-tiny.bin",
            installed: false,
            coreml_installed: false,
            engine: "whisper_cpp",
            experimental: false,
          },
        ]}
        running={false}
        progress={null}
        statusMessage=""
        onDownloadModel={onDownloadModel}
        onDownloadAll={onDownloadAll}
        onRefresh={onRefresh}
        onCancel={onCancel}
        onClose={onClose}
      />,
    );

    expect(screen.getByText("1 model(s) missing")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: /download missing/i }));
    expect(onDownloadAll).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByTitle("Refresh"));
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });

  it("shows parakeet models without coreml state or experimental badge", () => {
    const onDownloadModel = vi.fn().mockResolvedValue(undefined);

    render(
      <ModelManagerSheet
        open
        modelsDir="/tmp/parakeet-models"
        models={[
          {
            key: "tdt06b_v3_f16",
            label: "TDT 0.6B v3 F16",
            model_file: "tdt-0.6b-v3-f16.gguf",
            installed: false,
            coreml_installed: false,
            engine: "parakeet_cpp",
            experimental: false,
          },
        ]}
        running={false}
        progress={null}
        statusMessage=""
        onDownloadModel={onDownloadModel}
        onDownloadAll={vi.fn().mockResolvedValue(undefined)}
        onRefresh={vi.fn().mockResolvedValue(undefined)}
        onCancel={vi.fn().mockResolvedValue(undefined)}
        onClose={vi.fn()}
      />,
    );

    expect(screen.getByText("TDT 0.6B v3 F16")).toBeInTheDocument();
    expect(screen.queryByText("Experimental")).not.toBeInTheDocument();
    expect(screen.queryByText(/CoreML/i)).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /^download$/i }));
    expect(onDownloadModel).toHaveBeenCalledWith("tdt06b_v3_f16");
  });
});
