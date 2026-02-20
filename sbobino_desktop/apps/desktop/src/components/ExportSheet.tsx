import { Braces, Captions, Copy, Download, FileCode2, FileText, FileType, FileType2, List, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";

export type ExportFormat = "txt" | "docx" | "html" | "pdf" | "json";
export type ExportStyle = "transcript" | "subtitles" | "segments";
export type ExportGrouping = "none" | "speaker_paragraphs";

export type ExportSegment = {
  time: string;
  line: string;
};

export type ExportOptions = {
  includeTimestamps: boolean;
  grouping: ExportGrouping;
};

export type ExportRequest = {
  format: ExportFormat;
  style: ExportStyle;
  options: ExportOptions;
  segments: ExportSegment[];
  contentOverride?: string;
};

type ExportSheetProps = {
  open: boolean;
  transcriptText: string;
  segments: ExportSegment[];
  onClose: () => void;
  onExport: (payload: ExportRequest) => Promise<void>;
};

const formatItems: Array<{
  value: ExportFormat;
  label: string;
  icon: JSX.Element;
  hint: string;
  badge?: string;
}> = [
  {
    value: "txt",
    label: ".txt",
    icon: <FileText size={16} />,
    hint: "Plain text",
  },
  {
    value: "docx",
    label: ".docx",
    icon: <FileType2 size={16} />,
    hint: "Word document",
  },
  {
    value: "html",
    label: ".html",
    icon: <FileCode2 size={16} />,
    hint: "Web page",
  },
  {
    value: "pdf",
    label: ".pdf",
    icon: <FileType size={16} />,
    hint: "Portable document",
  },
  {
    value: "json",
    label: ".json",
    icon: <Braces size={16} />,
    hint: "Structured data",
  },
];

const styleItems: Array<{
  value?: ExportStyle;
  label: string;
  icon: JSX.Element;
  subtitle?: string;
  badge?: string;
}> = [
  {
    value: "transcript",
    label: "Transcript",
    icon: <FileText size={16} />,
  },
  {
    value: "subtitles",
    label: "Subtitles",
    icon: <Captions size={16} />,
  },
  {
    value: "segments",
    label: "Segments",
    icon: <List size={16} />,
  },
  {
    label: "Whisper",
    icon: <FileText size={16} />,
    subtitle: "Coming soon",
    badge: "PRO",
  },
  {
    label: "Dote",
    icon: <Captions size={16} />,
    subtitle: "Coming soon",
    badge: "PRO",
  },
];

function parseMmSsToSeconds(value: string): number {
  const [mmRaw, ssRaw] = value.split(":");
  const mm = Number(mmRaw);
  const ss = Number(ssRaw);
  if (Number.isNaN(mm) || Number.isNaN(ss)) {
    return 0;
  }
  return mm * 60 + ss;
}

function formatSrtTime(seconds: number): string {
  const hh = String(Math.floor(seconds / 3600)).padStart(2, "0");
  const mm = String(Math.floor((seconds % 3600) / 60)).padStart(2, "0");
  const ss = String(seconds % 60).padStart(2, "0");
  return `${hh}:${mm}:${ss},000`;
}

function buildExportContent(params: {
  transcriptText: string;
  segments: ExportSegment[];
  style: ExportStyle;
  includeTimestamps: boolean;
}): string {
  const { transcriptText, segments, style, includeTimestamps } = params;
  const normalizedTranscript = transcriptText.trim();

  if (style === "subtitles") {
    if (segments.length === 0) {
      return normalizedTranscript;
    }
    return segments
      .map((segment, index) => {
        const startSeconds = parseMmSsToSeconds(segment.time);
        const endSeconds = startSeconds + 4;
        return `${index + 1}\n${formatSrtTime(startSeconds)} --> ${formatSrtTime(endSeconds)}\n${segment.line.trim()}`;
      })
      .join("\n\n");
  }

  if (style === "segments") {
    if (segments.length === 0) {
      return normalizedTranscript;
    }
    return segments
      .map((segment) =>
        includeTimestamps ? `[${segment.time}] ${segment.line.trim()}` : segment.line.trim(),
      )
      .join("\n");
  }

  if (!includeTimestamps || segments.length === 0) {
    return normalizedTranscript;
  }

  return segments.map((segment) => `[${segment.time}] ${segment.line.trim()}`).join("\n");
}

export function ExportSheet({
  open,
  transcriptText,
  segments,
  onClose,
  onExport,
}: ExportSheetProps): JSX.Element | null {
  const [format, setFormat] = useState<ExportFormat>("txt");
  const [style, setStyle] = useState<ExportStyle>("transcript");
  const [includeTimestamps, setIncludeTimestamps] = useState(false);
  const [grouping, setGrouping] = useState<ExportGrouping>("none");
  const [isExporting, setIsExporting] = useState(false);

  const exportContent = useMemo(() => {
    return buildExportContent({
      transcriptText,
      segments,
      style,
      includeTimestamps,
    });
  }, [includeTimestamps, segments, style, transcriptText]);

  const preview = useMemo(() => {
    const normalized = exportContent.trim();
    if (!normalized) {
      return "No content available for export.";
    }
    return normalized;
  }, [exportContent]);

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [onClose, open]);

  if (!open) {
    return null;
  }

  async function onConfirm(): Promise<void> {
    setIsExporting(true);
    try {
      await onExport({
        format,
        style,
        options: {
          includeTimestamps,
          grouping,
        },
        segments,
        contentOverride: exportContent,
      });
      onClose();
    } finally {
      setIsExporting(false);
    }
  }

  async function onCopyContent(): Promise<void> {
    try {
      await navigator.clipboard.writeText(exportContent);
    } catch {
      // Keep the export sheet responsive even if clipboard fails.
    }
  }

  return (
    <div className="sheet-overlay" onClick={onClose}>
      <section
        className="export-sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby="export-sheet-title"
        onClick={(event) => event.stopPropagation()}
      >
        <button
          className="export-close-button"
          aria-label="Close export preview"
          onClick={onClose}
          disabled={isExporting}
        >
          <X size={14} />
        </button>

        <div className="export-preview">
          <header className="export-preview-head">
            <strong id="export-sheet-title">Export Preview</strong>
            <div className="export-tags">
              <span>{style}</span>
              <span>{format}</span>
            </div>
          </header>
          <pre>{preview}</pre>
        </div>

        <aside className="export-controls">
          <div className="export-controls-scroll">
            <h3>Style</h3>
            <div className="export-style-grid">
              {styleItems.map((item) => (
                <button
                  key={item.label}
                  className={style === item.value ? "format-card active" : "format-card"}
                  onClick={() => {
                    if (item.value) {
                      setStyle(item.value);
                    }
                  }}
                  disabled={!item.value}
                >
                  <span className="format-card-top">
                    <span className="format-card-icon">{item.icon}</span>
                    {item.badge ? <span className="format-card-badge">{item.badge}</span> : null}
                  </span>
                  <strong>{item.label}</strong>
                  {item.subtitle ? <small>{item.subtitle}</small> : null}
                </button>
              ))}
            </div>

            <h3>Format</h3>
            <div className="export-format-grid">
              {formatItems.map((item) => (
                <button
                  key={item.value}
                  className={format === item.value ? "format-card active" : "format-card"}
                  onClick={() => setFormat(item.value)}
                >
                  <span className="format-card-top">
                    <span className="format-card-icon">{item.icon}</span>
                    {item.badge ? <span className="format-card-badge">{item.badge}</span> : null}
                  </span>
                  <strong>{item.label}</strong>
                  <small>{item.hint}</small>
                </button>
              ))}
            </div>

            <div className="inspector-block export-options-block">
              <h4>Options</h4>
              <div className="property-line">
                <span>Grouping</span>
                <select
                  value={grouping}
                  onChange={(event) => setGrouping(event.target.value as ExportGrouping)}
                >
                  <option value="none">None</option>
                  <option value="speaker_paragraphs" disabled>
                    Speaker paragraphs
                  </option>
                </select>
              </div>
              <label className="toggle-row">
                <span>Show Timestamps</span>
                <input
                  type="checkbox"
                  checked={includeTimestamps}
                  onChange={(event) => setIncludeTimestamps(event.target.checked)}
                  disabled={style === "subtitles"}
                />
              </label>
            </div>
          </div>

          <div className="export-actions">
            <button className="secondary-button" onClick={onClose} disabled={isExporting}>
              Close
            </button>
            <button
              className="secondary-button"
              onClick={() => void onCopyContent()}
              disabled={isExporting}
            >
              <Copy size={14} />
              Copy
            </button>
            <button className="primary-button" onClick={() => void onConfirm()} disabled={isExporting}>
              <Download size={14} />
              {isExporting ? "Exporting..." : "Export"}
            </button>
          </div>
        </aside>
      </section>
    </div>
  );
}
