import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ExportSheet } from "./ExportSheet";

afterEach(() => {
  cleanup();
});

function renderExportSheet(): void {
  render(
    <ExportSheet
      open
      transcriptText="Hello world"
      segments={[
        { time: "00:00", line: "Hello world", speakerLabel: "Alice" },
      ]}
      title="Meeting"
      summary=""
      faqs=""
      onClose={vi.fn()}
      onExport={vi.fn().mockResolvedValue(true)}
    />,
  );
}

function renderExportSheetWithExportSpy(
  onExport = vi.fn().mockResolvedValue(true),
): ReturnType<typeof vi.fn> {
  render(
    <ExportSheet
      open
      transcriptText="This is the edited transcript paragraph."
      segments={[
        { time: "00:00", line: "Raw segment one.", speakerLabel: "Alice" },
        { time: "00:11", line: "Raw segment two.", speakerLabel: "Bob" },
      ]}
      title="Meeting"
      summary="Short summary"
      faqs=""
      onClose={vi.fn()}
      onExport={onExport}
    />,
  );
  return onExport;
}

describe("ExportSheet options cleanup", () => {
  it("does not render orphan transcript options", () => {
    renderExportSheet();

    expect(screen.queryByText("Grouping")).not.toBeInTheDocument();
    expect(screen.queryByText("Speaker paragraphs")).not.toBeInTheDocument();
  });

  it("does not render orphan subtitles/segments options", () => {
    renderExportSheet();

    fireEvent.click(screen.getAllByRole("button", { name: /Subtitles/i })[0]);
    expect(
      screen.queryByText("Favorited Segments Only"),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("Allow multiple lines")).not.toBeInTheDocument();
    expect(screen.queryByText("Use Original File Name")).not.toBeInTheDocument();

    fireEvent.click(screen.getAllByRole("button", { name: /Segments/i })[0]);
    expect(
      screen.queryByText("Favorited Segments Only"),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("Allow multiple lines")).not.toBeInTheDocument();
    expect(screen.queryByText("Use Original File Name")).not.toBeInTheDocument();
  });

  it("sends the rendered document preview for PDF exports without replacing the transcript draft", async () => {
    const onExport = renderExportSheetWithExportSpy();

    fireEvent.click(screen.getByRole("button", { name: /\.pdf/i }));
    fireEvent.click(screen.getByRole("button", { name: /^Export$/i }));

    await waitFor(() => expect(onExport).toHaveBeenCalledTimes(1));
    const payload = onExport.mock.calls[0][0];
    expect(payload.contentOverride).toBe("This is the edited transcript paragraph.");
    expect(payload.renderedContentOverride).toContain("Transcript of Meeting");
    expect(payload.renderedContentOverride).toContain("Transcript\nThis is the edited transcript paragraph.");
    expect(payload.renderedContentOverride).toContain("Summary\nShort summary");
    expect(payload.renderedContentOverride).not.toContain("Raw segment one.");
  });

  it("sends the rendered document preview for TXT transcript exports", async () => {
    const onExport = renderExportSheetWithExportSpy();

    fireEvent.click(screen.getByRole("button", { name: /^Export$/i }));

    await waitFor(() => expect(onExport).toHaveBeenCalledTimes(1));
    const payload = onExport.mock.calls[0][0];
    expect(payload.format).toBe("txt");
    expect(payload.style).toBe("transcript");
    expect(payload.renderedContentOverride).toContain("This is the edited transcript paragraph.");
    expect(payload.renderedContentOverride).not.toContain("Raw segment one.");
  });
});
