import { Braces, Captions, Copy, Download, FileCode2, FileSpreadsheet, FileText, FileType, FileType2, List, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "../i18n";

export type ExportFormat = "txt" | "docx" | "html" | "pdf" | "json" | "srt" | "vtt" | "csv" | "md";
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
  segmentsAlignedWithTranscript?: boolean;
  title?: string;
  summary?: string;
  faqs?: string;
  onClose: () => void;
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

function formatVttTime(seconds: number): string {
  const hh = String(Math.floor(seconds / 3600)).padStart(2, "0");
  const mm = String(Math.floor((seconds % 3600) / 60)).padStart(2, "0");
  const ss = String(seconds % 60).padStart(2, "0");
  return `${hh}:${mm}:${ss}.000`;
}

function buildPreviewContent(params: {
  transcriptText: string;
  segments: ExportSegment[];
  style: ExportStyle;
  format: ExportFormat;
  includeTimestamps: boolean;
  language: "en" | "it" | "es" | "de";
  title: string;
  summary?: string;
  faqs?: string;
}): string {
  const {
    transcriptText,
    segments,
    style,
    format,
    includeTimestamps,
    language,
    title,
    summary = "",
    faqs = "",
  } = params;
  const normalizedTranscript = transcriptText.trim();

  const localizedDocumentTitle = (rawTitle: string): string => {
    const fallbackTitle = (() => {
      switch (language) {
        case "it":
          return "Trascrizione";
        case "es":
          return "Transcripción";
        case "de":
          return "Transkript";
        default:
          return "Transcript";
      }
    })();
    const baseTitle = rawTitle.trim() || title.trim() || fallbackTitle;
    switch (language) {
      case "it":
        return `Trascrizione di ${baseTitle}`;
      case "es":
        return `Transcripción de ${baseTitle}`;
      case "de":
        return `Transkript von ${baseTitle}`;
      default:
        return `Transcript of ${baseTitle}`;
    }
  };

  const localizedCsvHeader = (): string => {
    switch (language) {
      case "it":
        return "Timestamp inizio;Timestamp fine;Trascrizione";
      case "es":
        return "Marca de tiempo inicial;Marca de tiempo final;Transcripción";
      case "de":
        return "Start-Zeitstempel;End-Zeitstempel;Transkript";
      default:
        return "Start Timestamp;End Timestamp;Transcript";
    }
  };

  const localizedPrimarySectionTitle = (): string => {
    if (style === "segments") {
      switch (language) {
        case "it":
          return "Segmenti";
        case "es":
          return "Segmentos";
        case "de":
          return "Segmente";
        default:
          return "Segments";
      }
    }
    switch (language) {
      case "it":
        return "Trascrizione";
      case "es":
        return "Transcripción";
      case "de":
        return "Transkript";
      default:
        return "Transcript";
    }
  };

  const localizedSummaryTitle = (): string => {
    switch (language) {
      case "it":
        return "Riassunto";
      case "es":
        return "Resumen";
      case "de":
        return "Zusammenfassung";
      default:
        return "Summary";
    }
  };

  const localizedFaqTitle = (): string => {
    switch (language) {
      case "it":
        return "Domande frequenti";
      case "es":
        return "Preguntas frecuentes";
      case "de":
        return "Haeufige Fragen";
      default:
        return "FAQs";
    }
  };

  const buildDocumentPreview = (body: string): string => {
    const sections = [
      `${localizedPrimarySectionTitle()}\n${body.trim()}`,
      summary.trim() ? `${localizedSummaryTitle()}\n${summary.trim()}` : "",
      faqs.trim() ? `${localizedFaqTitle()}\n${faqs.trim()}` : "",
    ].filter(Boolean);

    return [localizedDocumentTitle(title), ...sections].join("\n\n");
  };

  // ── Subtitles ──
  if (style === "subtitles") {
    if (format === "srt") {
      if (segments.length === 0) return normalizedTranscript;
      return segments
        .map((segment, index) => {
          const startSeconds = parseMmSsToSeconds(segment.time);
          const endSeconds = startSeconds + 11;
          return `${index + 1}\n${formatSrtTime(startSeconds)} --> ${formatSrtTime(endSeconds)}\n${segment.line.trim()}`;
        })
        .join("\n\n");
    }
    if (format === "vtt") {
      if (segments.length === 0) return `WEBVTT\n\n${normalizedTranscript}`;
      const cues = segments
        .map((segment) => {
          const startSeconds = parseMmSsToSeconds(segment.time);
          const endSeconds = startSeconds + 11;
          return `${formatVttTime(startSeconds)} --> ${formatVttTime(endSeconds)}\n${segment.line.trim()}`;
        })
        .join("\n\n");
      return `WEBVTT\n\n${cues}`;
    }
    if (format === "md") {
      if (segments.length === 0) return normalizedTranscript;
      return segments
        .map((segment) => `${segment.line.trim()}\n${segment.time}`)
        .join("\n\n");
    }
    return normalizedTranscript;
  }

  // ── Segments ──
  if (style === "segments") {
    if (format === "json") {
      if (segments.length === 0) {
        return JSON.stringify([{ text: normalizedTranscript }], null, 2);
      }
      const arr = segments.map((segment) => {
        const startSeconds = parseMmSsToSeconds(segment.time);
        const endStr = String(startSeconds + 11).padStart(2, "0");
        return {
          text: segment.line.trim(),
          timestamp: `${segment.time}-00:${endStr}`,
        };
      });
      return JSON.stringify(arr, null, 2);
    }

    if (format === "csv") {
      const header = localizedCsvHeader();
      if (segments.length === 0) return `${header}\n00:00;00:00;"${normalizedTranscript}"`;
      const rows = segments.map((segment) => {
        const startSeconds = parseMmSsToSeconds(segment.time);
        const endSeconds = startSeconds + 11;
        const endMm = String(Math.floor(endSeconds / 60)).padStart(2, "0");
        const endSs = String(endSeconds % 60).padStart(2, "0");
        return `${segment.time};${endMm}:${endSs};"${segment.line.trim()}"`;
      });
      return `${header}\n${rows.join("\n")}`;
    }

    if (format === "html" || format === "pdf" || format === "md") {
      if (segments.length === 0) {
        return buildDocumentPreview(normalizedTranscript);
      }
      const body = segments
        .map((segment) =>
          includeTimestamps ? `[${segment.time}] ${segment.line.trim()}` : segment.line.trim(),
        )
        .join("\n");
      return buildDocumentPreview(body);
    }

    // txt, docx
    if (segments.length === 0) return buildDocumentPreview(normalizedTranscript);
    const body = segments
      .map((segment) => `[${segment.time}] ${segment.line.trim()}`)
      .join("\n");
    return buildDocumentPreview(body);
  }

  // ── Transcript ──
  if (format === "json") {
    return JSON.stringify([{ text: normalizedTranscript }], null, 2);
  }

  if (format === "html" || format === "pdf") {
    if (!includeTimestamps || segments.length === 0) {
      return buildDocumentPreview(normalizedTranscript);
    }
    return buildDocumentPreview(
      segments.map((segment) => `[${segment.time}] ${segment.line.trim()}`).join("\n"),
    );
  }

  // txt, docx, md
  if (!includeTimestamps || segments.length === 0) {
    return buildDocumentPreview(normalizedTranscript);
  }
  return buildDocumentPreview(
    segments.map((segment) => `[${segment.time}] ${segment.line.trim()}`).join("\n"),
  );
}

export function ExportSheet({
  open,
  transcriptText,
  segments,
  segmentsAlignedWithTranscript = true,
  title = "",
  summary = "",
  faqs = "",
  onClose,
  onExport,
}: ExportSheetProps): JSX.Element | null {
  const [format, setFormat] = useState<ExportFormat>("txt");
  const [style, setStyle] = useState<ExportStyle>("transcript");
  const [includeTimestamps, setIncludeTimestamps] = useState(false);
  const [grouping, setGrouping] = useState<ExportGrouping>("none");
  const [showSpeakerNames, setShowSpeakerNames] = useState(false);
  const [favoritedOnly, setFavoritedOnly] = useState(false);
  const [allowMultipleLines, setAllowMultipleLines] = useState(false);
  const [useOriginalFileName, setUseOriginalFileName] = useState(false);
  const [isExporting, setIsExporting] = useState(false);
  const { t, language } = useTranslation();
  const prevStyleRef = useRef(style);
  const exportSegments = useMemo(
    () => (segmentsAlignedWithTranscript ? segments : []),
    [segments, segmentsAlignedWithTranscript],
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

  const exportContent = useMemo(() => {
    return buildPreviewContent({
      transcriptText,
      segments: exportSegments,
      style,
      format,
      includeTimestamps,
      language,
      title,
      summary,
      faqs,
    });
  }, [exportSegments, faqs, includeTimestamps, language, style, format, summary, transcriptText, title]);

  const preview = useMemo(() => {
    const normalized = exportContent.trim();
    if (!normalized) {
      return t("export.noContent", "No content available for export.");
    }
    return normalized;
  }, [exportContent, t]);

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
      const didExport = await onExport({
        format,
        style,
        options: {
          includeTimestamps,
          grouping,
        },
        segments: exportSegments,
        contentOverride: transcriptText,
      });
      if (didExport) {
        onClose();
      }
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

  const styleLabelCapitalized =
    style === "transcript"
      ? t("export.transcript", "Transcript")
      : style === "subtitles"
        ? t("export.subtitles", "Subtitles")
        : t("export.segments", "Segments");

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
          <div className="export-preview-body">
            <pre>{preview}</pre>
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
                  disabled={!item.value || item.disabled}
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
                  <div className="property-line">
                    <span>{t("export.grouping", "Grouping")}</span>
                    <select
                      value={grouping}
                      onChange={(event) => setGrouping(event.target.value as ExportGrouping)}
                    >
                      <option value="none">{t("export.groupingNone", "None")}</option>
                      <option value="speaker_paragraphs" disabled>
                        {t("export.speakerParagraphs", "Speaker paragraphs")}
                      </option>
                    </select>
                  </div>
                  <label className="toggle-row">
                    <span>{t("export.showTimestamps", "Show Timestamps")}</span>
                    <input
                      type="checkbox"
                      checked={includeTimestamps}
                      onChange={(event) => setIncludeTimestamps(event.target.checked)}
                      disabled={!segmentsAlignedWithTranscript}
                    />
                  </label>
                  <p className="export-option-note">
                    {t(
                      "export.speakerNote",
                      "The Speaker paragraphs option is unavailable unless speakers are assigned to your transcript.",
                    )}
                  </p>
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
                    />
                  </label>
                  <label className="toggle-row">
                    <span>{t("export.favoritedOnly", "Favorited Segments Only")}</span>
                    <input
                      type="checkbox"
                      checked={favoritedOnly}
                      onChange={(event) => setFavoritedOnly(event.target.checked)}
                    />
                  </label>
                  <p className="export-option-note">
                    {t(
                      "export.speakerNote",
                      "You can only enable speaker names if you assign speakers in your transcript.",
                    )}
                  </p>
                  <label className="toggle-row">
                    <span>{t("export.allowMultipleLines", "Allow multiple lines")}</span>
                    <input
                      type="checkbox"
                      checked={allowMultipleLines}
                      onChange={(event) => setAllowMultipleLines(event.target.checked)}
                    />
                  </label>
                  <label className="toggle-row">
                    <span>{t("export.useOriginalFileName", "Use Original File Name")}</span>
                    <input
                      type="checkbox"
                      checked={useOriginalFileName}
                      onChange={(event) => setUseOriginalFileName(event.target.checked)}
                    />
                  </label>
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
                    />
                  </label>
                  <label className="toggle-row">
                    <span>{t("export.favoritedOnly", "Favorited Segments Only")}</span>
                    <input
                      type="checkbox"
                      checked={favoritedOnly}
                      onChange={(event) => setFavoritedOnly(event.target.checked)}
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
                    />
                  </label>
                  <label className="toggle-row">
                    <span>{t("export.allowMultipleLines", "Allow multiple lines")}</span>
                    <input
                      type="checkbox"
                      checked={allowMultipleLines}
                      onChange={(event) => setAllowMultipleLines(event.target.checked)}
                    />
                  </label>
                  <label className="toggle-row">
                    <span>{t("export.useOriginalFileName", "Use Original File Name")}</span>
                    <input
                      type="checkbox"
                      checked={useOriginalFileName}
                      onChange={(event) => setUseOriginalFileName(event.target.checked)}
                    />
                  </label>
                </>
              ) : null}
            </div>
          </div>

          <div className="export-actions">
            <button
              className="secondary-button icon-only"
              onClick={() => void onCopyContent()}
              disabled={isExporting}
              title={t("export.copy", "Copy")}
            >
              <Copy size={14} />
            </button>
            <button className="primary-button" onClick={() => void onConfirm()} disabled={isExporting}>
              <Download size={14} />
              {isExporting ? t("export.exporting", "Exporting...") : t("export.export", "Export")}
            </button>
          </div>
        </aside>
      </section>
    </div>
  );
}
