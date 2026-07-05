import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ExportSheet, type ExportRequest } from "./ExportSheet";

afterEach(() => {
  cleanup();
});

function renderExportSheet(onExport = vi.fn().mockResolvedValue(true)): void {
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
      onExport={onExport}
    />,
  );
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

  it("submits the same formatted content shown in the preview", async () => {
    const onExport = vi.fn().mockResolvedValue(true);
    renderExportSheet(onExport);

    fireEvent.click(screen.getAllByRole("button", { name: /Segments/i })[0]);
    fireEvent.click(screen.getByRole("checkbox", { name: /Show Speaker Names/i }));
    const dialogs = screen.getAllByRole("dialog");
    const currentDialog = dialogs[dialogs.length - 1];
    fireEvent.click(within(currentDialog).getByRole("button", { name: /^Export$/i }));

    await waitFor(() => expect(onExport).toHaveBeenCalledTimes(1));
    const payload = onExport.mock.calls[0][0] as ExportRequest;
    expect(payload.contentOverride).toContain("[00:00] Alice: Hello world");
  });
});
