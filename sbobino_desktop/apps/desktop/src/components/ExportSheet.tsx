import { Braces, Captions, Check, Copy, Download, FileCode2, FileSpreadsheet, FileText, FileType, FileType2, List, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "../i18n";
import { copyTextToClipboard } from "../lib/clipboard";

export type ExportFormat = "txt" | "docx" | "html" | "pdf" | "json" | "srt" | "vtt" | "csv" | "md";
export type ExportStyle = "transcript" | "subtitles" | "segments";
export type ExportGrouping = "none" | "speaker_paragraphs";

export type ExportSegment = {
  time: string;
  line: string;
  startSeconds?: number | null;
  endSeconds?: number | null;
  speakerId?: string | null;
  speakerLabel?: string | null;
};

export type ExportOptions = {
  includeTimestamps: boolean;
  grouping: ExportGrouping;
  includeSpeakerNames?: boolean;
};

export type ExportRequest = {
  format: ExportFormat;
  style: ExportStyle;
  options: ExportOptions;
  segments: ExportSegment[];
  contentOverride?: string;
};

export type ExportPreview = {
  content: string;
  mode: "exact" | "document";
};

type ExportSheetProps = {
  open: boolean;
  transcriptText: string;
  segments: ExportSegment[];
  segmentsAlignedWithTranscript?: boolean;
  onClose: () => void;
  onPreview: (payload: ExportRequest) => Promise<ExportPreview>;
  onExport: (payload: ExportRequest) => Promise<boolean>;
};

type FormatItem = {
  value: ExportFormat;
  label: string;
  icon: JSX.Element;
  hint: string;
  badge?: string;
};

function getTranscriptFormats(t: (key: string, fallback?: string) => string): FormatItem[] {
  return [
    { value: "txt", label: ".txt", icon: <FileText size={16} />, hint: t("export.plainText", "Plain text") },
    { value: "docx", label: ".docx", icon: <FileType2 size={16} />, hint: t("export.wordDocument", "Word document") },
    { value: "html", label: ".html", icon: <FileCode2 size={16} />, hint: t("export.webPage", "Web page") },
    { value: "pdf", label: ".pdf", icon: <FileType size={16} />, hint: t("export.portableDocument", "Portable document") },
    { value: "md", label: ".md", icon: <FileText size={16} />, hint: t("export.markdown", "Markdown") },
  ];
}

function getSubtitlesFormats(t: (key: string, fallback?: string) => string): FormatItem[] {
  return [
    { value: "srt", label: ".srt", icon: <Captions size={16} />, hint: t("export.srtSubtitles", "SRT subtitles") },
    { value: "vtt", label: ".vtt", icon: <Captions size={16} />, hint: t("export.webVtt", "WebVTT") },
  ];
}

function getSegmentsFormats(t: (key: string, fallback?: string) => string): FormatItem[] {
  return [
    { value: "txt", label: ".txt", icon: <FileText size={16} />, hint: t("export.plainText", "Plain text") },
    { value: "csv", label: ".csv", icon: <FileSpreadsheet size={16} />, hint: t("export.csvSpreadsheet", "CSV spreadsheet") },
    { value: "docx", label: ".docx", icon: <FileType2 size={16} />, hint: t("export.wordDocument", "Word document") },
    { value: "html", label: ".html", icon: <FileCode2 size={16} />, hint: t("export.webPage", "Web page") },
    { value: "pdf", label: ".pdf", icon: <FileType size={16} />, hint: t("export.portableDocument", "Portable document") },
    { value: "md", label: ".md", icon: <FileText size={16} />, hint: t("export.markdown", "Markdown") },
    { value: "json", label: ".json", icon: <Braces size={16} />, hint: t("export.structuredData", "Structured data") },
  ];
}

function getFormatsForStyle(
  style: ExportStyle,
  t: (key: string, fallback?: string) => string,
): FormatItem[] {
  if (style === "subtitles") return getSubtitlesFormats(t);
  if (style === "segments") return getSegmentsFormats(t);
  return getTranscriptFormats(t);
}

type StyleItem = {
  value?: ExportStyle;
  label: string;
  icon: JSX.Element;
  subtitle?: string;
  badge?: string;
  disabled?: boolean;
};

function getStyleItems(
  t: (key: string, fallback?: string) => string,
  segmentsAlignedWithTranscript: boolean,
): StyleItem[] {
  return [
    {
      value: "transcript",
      label: t("export.transcript", "Transcript"),
      icon: <FileText size={16} />,
    },
    {
      value: "subtitles",
      label: t("export.subtitles", "Subtitles"),
      icon: <Captions size={16} />,
      subtitle: !segmentsAlignedWithTranscript
        ? t(
          "export.segmentedRequiresOriginal",
          "Available only for the original transcript to preserve timeline alignment.",
        )
        : undefined,
      disabled: !segmentsAlignedWithTranscript,
    },
    {
      value: "segments",
      label: t("export.segments", "Segments"),
      icon: <List size={16} />,
      subtitle: !segmentsAlignedWithTranscript
        ? t(
          "export.segmentedRequiresOriginal",
          "Available only for the original transcript to preserve timeline alignment.",
        )
        : undefined,
      disabled: !segmentsAlignedWithTranscript,
    },
  ];
}

export function ExportSheet({
  open,
  transcriptText,
  segments,
  segmentsAlignedWithTranscript = true,
  onClose,
  onPreview,
  onExport,
}: ExportSheetProps): JSX.Element | null {
  const [format, setFormat] = useState<ExportFormat>("txt");
  const [style, setStyle] = useState<ExportStyle>("transcript");
  const [includeTimestamps, setIncludeTimestamps] = useState(false);
  const [showSpeakerNames, setShowSpeakerNames] = useState(false);
  const [isExporting, setIsExporting] = useState(false);
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">("idle");
  const [previewState, setPreviewState] = useState<{
    status: "idle" | "loading" | "ready" | "error";
    content: string;
    mode: ExportPreview["mode"];
  }>({ status: "idle", content: "", mode: "exact" });
  const [exportError, setExportError] = useState<string | null>(null);
  const { t, language } = useTranslation();
  const prevStyleRef = useRef(style);
  const previewRequestIdRef = useRef(0);
  const exportSegments = useMemo(
    () => (segmentsAlignedWithTranscript ? segments : []),
    [segments, segmentsAlignedWithTranscript],
  );
  const speakerNamesAvailable = useMemo(
    () => exportSegments.some((segment) => Boolean(segment.speakerLabel?.trim())),
    [exportSegments],
  );

  const styleItems = useMemo(
    () => getStyleItems(t, segmentsAlignedWithTranscript),
    [language, segmentsAlignedWithTranscript],
  );
  const formatItems = useMemo(() => getFormatsForStyle(style, t), [style, language]);

  // Auto-reset format when style changes
  useEffect(() => {
    if (prevStyleRef.current !== style) {
      prevStyleRef.current = style;
      const available = getFormatsForStyle(style, t);
      if (available.length > 0 && !available.some((f) => f.value === format)) {
        setFormat(available[0].value);
      }
      // Subtitles always have timestamps on, segments default to on
      if (style === "subtitles") {
        setIncludeTimestamps(true);
      } else if (style === "segments") {
        setIncludeTimestamps(true);
      }
    }
  }, [style, format, t]);

  useEffect(() => {
    if (segmentsAlignedWithTranscript) return;
    if (style !== "transcript") {
      setStyle("transcript");
    }
    if (includeTimestamps) {
      setIncludeTimestamps(false);
    }
  }, [includeTimestamps, segmentsAlignedWithTranscript, style]);

  useEffect(() => {
    if (!speakerNamesAvailable && showSpeakerNames) {
      setShowSpeakerNames(false);
    }
  }, [showSpeakerNames, speakerNamesAvailable]);

  const exportRequest = useMemo<ExportRequest>(
    () => ({
      format,
      style,
      options: {
        includeTimestamps,
        grouping: "none",
        includeSpeakerNames: showSpeakerNames,
      },
      segments: exportSegments,
      contentOverride: transcriptText,
    }),
    [exportSegments, format, includeTimestamps, showSpeakerNames, style, transcriptText],
  );

  useEffect(() => {
    if (!open) {
      previewRequestIdRef.current += 1;
      setPreviewState({ status: "idle", content: "", mode: "exact" });
      return;
    }

    const requestId = previewRequestIdRef.current + 1;
    previewRequestIdRef.current = requestId;
    setPreviewState((previous) => ({ ...previous, status: "loading" }));
    setExportError(null);

    void onPreview(exportRequest).then(
      (result) => {
        if (previewRequestIdRef.current !== requestId) return;
        setPreviewState({
          status: "ready",
          content: result.content,
          mode: result.mode,
        });
      },
      () => {
        if (previewRequestIdRef.current !== requestId) return;
        setPreviewState({ status: "error", content: "", mode: "exact" });
      },
    );

    return () => {
      if (previewRequestIdRef.current === requestId) {
        previewRequestIdRef.current += 1;
      }
    };
  }, [exportRequest, onPreview, open]);

  const preview = (() => {
    if (previewState.status === "loading") {
      return t("export.previewLoading", "Preparing preview...");
    }
    if (previewState.status === "error") {
      return t("export.previewFailed", "Could not generate the export preview.");
    }
    const normalized = previewState.content.trim();
    if (!normalized) {
      return t("export.noContent", "No content available for export.");
    }
    return normalized;
  })();

  useEffect(() => {
    if (!open) return;
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === "Escape" && !isExporting) {
        event.preventDefault();
        onClose();
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [isExporting, onClose, open]);

  useEffect(() => {
    if (copyState === "idle") return;
    const timeoutId = window.setTimeout(() => {
      setCopyState("idle");
    }, 1600);
    return () => {
      window.clearTimeout(timeoutId);
    };
  }, [copyState]);

  if (!open) {
    return null;
  }

  async function onConfirm(): Promise<void> {
    if (previewState.status !== "ready") return;
    setIsExporting(true);
    setExportError(null);
    try {
      const didExport = await onExport(exportRequest);
      if (didExport) {
        onClose();
      }
    } catch {
      setExportError(t("export.exportFailed", "Export failed. Please try again."));
    } finally {
      setIsExporting(false);
    }
  }

  async function onCopyContent(): Promise<void> {
    if (previewState.status !== "ready") return;
    const didCopy = await copyTextToClipboard(previewState.content);
    setCopyState(didCopy ? "copied" : "failed");
  }

  const styleLabelCapitalized =
    style === "transcript"
      ? t("export.transcript", "Transcript")
      : style === "subtitles"
        ? t("export.subtitles", "Subtitles")
        : t("export.segments", "Segments");
  const copyButtonLabel =
    copyState === "copied"
      ? t("export.copied", "Copied")
      : copyState === "failed"
        ? t("export.copyFailed", "Copy failed")
        : t("export.copy", "Copy");
  const previewReady = previewState.status === "ready";

  return (
    <div className="sheet-overlay" onClick={isExporting ? undefined : onClose}>
      <section
        className="export-sheet"
        role="dialog"
        aria-modal="true"
        aria-labelledby="export-sheet-title"
        onClick={(event) => event.stopPropagation()}
      >
        <button
          className="export-close-button"
          aria-label={t("export.closePreview", "Close export preview")}
          onClick={onClose}
          disabled={isExporting}
        >
          <X size={14} />
        </button>

        <div className="export-preview">
          <header className="export-preview-head">
            <strong id="export-sheet-title">{t("export.preview", "Export Preview")}</strong>
            <div className="export-preview-head-actions">
              <div className="export-tags">
                <span>{styleLabelCapitalized}</span>
                <span>.{format}</span>
              </div>
            </div>
          </header>
          <div
            className="export-preview-body"
            aria-busy={previewState.status === "loading"}
            aria-live="polite"
          >
            <pre>{preview}</pre>
            {exportError ? (
              <p className="export-preview-error" role="alert">
                {exportError}
              </p>
            ) : null}
          </div>
        </div>

        <aside className="export-controls">
          <div className="export-controls-scroll">
            <h3>{t("export.style", "Style")}</h3>
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
                  disabled={!item.value || item.disabled || isExporting}
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

            {!segmentsAlignedWithTranscript ? (
              <p className="export-option-note">
                {t(
                  "export.segmentedRequiresOriginal",
                  "Available only for the original transcript to preserve timeline alignment.",
                )}
              </p>
            ) : null}

            <h3>{t("export.format", "Format")}</h3>
            <div className="export-format-grid">
              {formatItems.map((item) => (
                <button
                  key={item.value}
                  className={format === item.value ? "format-card active" : "format-card"}
                  onClick={() => setFormat(item.value)}
                  disabled={isExporting}
                >
                  <span className="format-card-top">
                    <span className="format-card-icon">{item.icon}</span>
                    {item.badge ? <span className="format-card-badge">{item.badge}</span> : null}
                  </span>
                  <strong>{item.label}</strong>
                </button>
              ))}
            </div>

            {/* ── Options ── */}
            <div className="inspector-block export-options-block">
              <h4>{t("export.options", "Options")}</h4>

              {style === "transcript" ? (
                <>
                  <label className="toggle-row">
                    <span>{t("export.showTimestamps", "Show Timestamps")}</span>
                    <input
                      type="checkbox"
                      checked={includeTimestamps}
                      onChange={(event) => setIncludeTimestamps(event.target.checked)}
                      disabled={!segmentsAlignedWithTranscript || isExporting}
                    />
                  </label>
                </>
              ) : null}

              {style === "subtitles" ? (
                <>
                  <label className="toggle-row">
                    <span>{t("export.showSpeakerNames", "Show Speaker Names")}</span>
                    <input
                      type="checkbox"
                      checked={showSpeakerNames}
                      onChange={(event) => setShowSpeakerNames(event.target.checked)}
                      disabled={!speakerNamesAvailable || isExporting}
                    />
                  </label>
                  <p className="export-option-note">
                    {t(
                      "export.speakerNote",
                      "You can only enable speaker names if you assign speakers in your transcript.",
                    )}
                  </p>
                </>
              ) : null}

              {style === "segments" ? (
                <>
                  <label className="toggle-row">
                    <span>{t("export.showSpeakerNames", "Show Speaker Names")}</span>
                    <input
                      type="checkbox"
                      checked={showSpeakerNames}
                      onChange={(event) => setShowSpeakerNames(event.target.checked)}
                      disabled={!speakerNamesAvailable || isExporting}
                    />
                  </label>
                  <p className="export-option-note">
                    {t(
                      "export.speakerNote",
                      "You can only enable speaker names if you assign speakers in your transcript.",
                    )}
                  </p>
                  <label className="toggle-row">
                    <span>{t("export.showTimestamps", "Show Timestamps")}</span>
                    <input
                      type="checkbox"
                      checked={includeTimestamps}
                      onChange={(event) => setIncludeTimestamps(event.target.checked)}
                      disabled={isExporting}
                    />
                  </label>
                </>
              ) : null}
            </div>
          </div>

          <div className="export-actions">
            <button
              className={`secondary-button export-copy-button ${copyState === "idle" ? "" : `is-${copyState}`}`}
              onClick={() => void onCopyContent()}
              disabled={isExporting || !previewReady}
              title={copyButtonLabel}
              aria-label={copyButtonLabel}
            >
              {copyState === "copied" ? <Check size={14} /> : copyState === "failed" ? <X size={14} /> : <Copy size={14} />}
              <span aria-live="polite">{copyButtonLabel}</span>
            </button>
            <button
              className="primary-button"
              onClick={() => void onConfirm()}
              disabled={isExporting || !previewReady}
            >
              <Download size={14} />
              {isExporting ? t("export.exporting", "Exporting...") : t("export.export", "Export")}
            </button>
          </div>
        </aside>
      </section>
    </div>
  );
}
