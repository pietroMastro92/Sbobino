import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ModelManagerSheet } from "./ModelManagerSheet";

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
});
