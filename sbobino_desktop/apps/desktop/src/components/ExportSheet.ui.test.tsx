import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import {
  ExportSheet,
  type ExportPreview,
  type ExportRequest,
} from "./ExportSheet";

afterEach(() => {
  cleanup();
});

function renderExportSheet(options?: {
  open?: boolean;
  onClose?: () => void;
  onPreview?: (payload: ExportRequest) => Promise<ExportPreview>;
  onExport?: (payload: ExportRequest) => Promise<boolean>;
}): ReturnType<typeof render> {
  return render(
    <ExportSheet
      open={options?.open ?? true}
      transcriptText="Hello world"
      segments={[
        {
          time: "00:00",
          line: "Hello world",
          startSeconds: 0.25,
          endSeconds: 2.75,
          speakerLabel: "Alice",
        },
      ]}
      onClose={options?.onClose ?? vi.fn()}
      onPreview={
        options?.onPreview ??
        vi.fn().mockResolvedValue({ content: "Backend preview", mode: "exact" })
      }
      onExport={options?.onExport ?? vi.fn().mockResolvedValue(true)}
    />,
  );
}

describe("ExportSheet", () => {
  it("does not render orphan transcript options", () => {
    renderExportSheet();

    expect(screen.queryByText("Grouping")).not.toBeInTheDocument();
    expect(screen.queryByText("Speaker paragraphs")).not.toBeInTheDocument();
  });

  it("does not render orphan subtitles/segments options", () => {
    renderExportSheet();

    fireEvent.click(screen.getAllByRole("button", { name: /Subtitles/i })[0]);
    expect(screen.queryByText("Favorited Segments Only")).not.toBeInTheDocument();
    expect(screen.queryByText("Allow multiple lines")).not.toBeInTheDocument();
    expect(screen.queryByText("Use Original File Name")).not.toBeInTheDocument();

    fireEvent.click(screen.getAllByRole("button", { name: /Segments/i })[0]);
    expect(screen.queryByText("Favorited Segments Only")).not.toBeInTheDocument();
    expect(screen.queryByText("Allow multiple lines")).not.toBeInTheDocument();
    expect(screen.queryByText("Use Original File Name")).not.toBeInTheDocument();
  });

  it("shows the canonical backend preview and submits the raw transcript", async () => {
    const onPreview = vi.fn().mockResolvedValue({
      content: "Transcript of Meeting\n\nTranscript\nHello world",
      mode: "document",
    });
    const onExport = vi.fn().mockResolvedValue(true);
    renderExportSheet({ onPreview, onExport });

    expect(
      await screen.findByText(/Transcript of Meeting/, { selector: "pre" }),
    ).toBeInTheDocument();
    const dialog = screen.getByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: /^Export$/i }));

    await waitFor(() => expect(onExport).toHaveBeenCalledTimes(1));
    const payload = onExport.mock.calls[0][0] as ExportRequest;
    expect(payload.contentOverride).toBe("Hello world");
    expect(payload.segments[0]).toMatchObject({
      startSeconds: 0.25,
      endSeconds: 2.75,
    });
  });

  it("ignores an older preview response after the format changes", async () => {
    let resolveTxt!: (preview: ExportPreview) => void;
    let resolvePdf!: (preview: ExportPreview) => void;
    const txtPreview = new Promise<ExportPreview>((resolve) => {
      resolveTxt = resolve;
    });
    const pdfPreview = new Promise<ExportPreview>((resolve) => {
      resolvePdf = resolve;
    });
    const onPreview = vi.fn((payload: ExportRequest) =>
      payload.format === "pdf" ? pdfPreview : txtPreview,
    );
    renderExportSheet({ onPreview });

    expect(screen.getByRole("button", { name: /^Export$/i })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: /\.pdf/i }));
    await act(async () => {
      resolvePdf({ content: "Newest PDF preview", mode: "document" });
    });
    expect(await screen.findByText("Newest PDF preview")).toBeInTheDocument();

    await act(async () => {
      resolveTxt({ content: "Stale TXT preview", mode: "exact" });
    });
    expect(screen.queryByText("Stale TXT preview")).not.toBeInTheDocument();
    expect(screen.getByText("Newest PDF preview")).toBeInTheDocument();
  });

  it("keeps the sheet open and restores controls after an unexpected rejection", async () => {
    const onExport = vi.fn().mockRejectedValue(new Error("disk unavailable"));
    renderExportSheet({ onExport });

    const exportButton = screen.getByRole("button", { name: /^Export$/i });
    await waitFor(() => expect(exportButton).toBeEnabled());
    fireEvent.click(exportButton);

    expect(await screen.findByRole("alert")).toHaveTextContent(/Export failed/i);
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    await waitFor(() => expect(exportButton).toBeEnabled());
  });

  it("disables copy and export when preview generation fails", async () => {
    renderExportSheet({
      onPreview: vi.fn().mockRejectedValue(new Error("preview unavailable")),
    });

    expect(
      await screen.findByText("Could not generate the export preview."),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^Copy$/i })).toBeDisabled();
    expect(screen.getByRole("button", { name: /^Export$/i })).toBeDisabled();
  });

  it("focuses the dialog and cycles only through enabled controls", () => {
    renderExportSheet();

    const dialog = screen.getByRole("dialog");
    const closeButton = screen.getByRole("button", { name: /Close export preview/i });
    const timestamps = screen.getByRole("checkbox", { name: /Show Timestamps/i });

    expect(closeButton).toHaveFocus();

    fireEvent.keyDown(dialog, { key: "Tab", shiftKey: true });
    expect(timestamps).toHaveFocus();

    fireEvent.keyDown(dialog, { key: "Tab" });
    expect(closeButton).toHaveFocus();

    fireEvent.keyDown(dialog, { key: "Tab" });
    expect(screen.getByRole("button", { name: /^Transcript$/i })).toHaveFocus();
  });

  it("restores the opener after Escape closes the sheet", () => {
    const opener = document.createElement("button");
    opener.type = "button";
    opener.textContent = "Open export";
    document.body.appendChild(opener);
    opener.focus();
    const onClose = vi.fn();
    const onPreview = vi.fn().mockResolvedValue({ content: "Backend preview", mode: "exact" });
    const view = renderExportSheet({ onClose, onPreview });

    fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });

    expect(onClose).toHaveBeenCalledTimes(1);
    view.rerender(
      <ExportSheet
        open={false}
        transcriptText="Hello world"
        segments={[]}
        onClose={onClose}
        onPreview={onPreview}
        onExport={vi.fn().mockResolvedValue(true)}
      />,
    );
    expect(opener).toHaveFocus();
    opener.remove();
  });

  it("keeps Escape inert while exporting and focuses the busy dialog fallback", async () => {
    let resolveExport!: (didExport: boolean) => void;
    const onExport = vi.fn(
      () => new Promise<boolean>((resolve) => {
        resolveExport = resolve;
      }),
    );
    const onClose = vi.fn();
    renderExportSheet({ onClose, onExport });

    const exportButton = screen.getByRole("button", { name: /^Export$/i });
    await waitFor(() => expect(exportButton).toBeEnabled());
    fireEvent.click(exportButton);
    await waitFor(() => expect(exportButton).toBeDisabled());

    const dialog = screen.getByRole("dialog");
    expect(dialog).toHaveAttribute("tabindex", "-1");
    expect(dialog).toHaveFocus();

    fireEvent.keyDown(dialog, { key: "Escape" });
    expect(onClose).not.toHaveBeenCalled();

    await act(async () => {
      resolveExport(false);
    });
  });
});
