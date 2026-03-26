import React, {
  type CSSProperties,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";

import type { ConfidenceTranscriptDocument } from "../lib/whisperConfidence";
import { useTranslation } from "../i18n";

const TOOLTIP_MARGIN = 14;
const TOOLTIP_GAP = 14;
const TOOLTIP_CLOSE_MS = 440;

type TooltipPhase = "opening" | "open" | "closing";

type ActiveTooltip = {
  color: string;
  nonce: number;
  phase: TooltipPhase;
  target: HTMLSpanElement;
  text: string;
};

type TooltipLayout = {
  arrowLeft: number;
  left: number;
  top: number;
};

function clamp(value: number, min: number, max: number): number {
  if (max < min) {
    return min;
  }
  return Math.min(Math.max(value, min), max);
}

export function ConfidenceTranscript({
  document: transcriptDocument,
  fontSize,
}: {
  document: ConfidenceTranscriptDocument;
  fontSize: number;
}): JSX.Element {
  const { t } = useTranslation();
  const bubbleRef = useRef<HTMLDivElement | null>(null);
  const [activeTooltip, setActiveTooltip] = useState<ActiveTooltip | null>(null);
  const [tooltipLayout, setTooltipLayout] = useState<TooltipLayout | null>(null);
  const tooltipNonceRef = useRef(0);
  const closeTimerRef = useRef<number | null>(null);

  const clearCloseTimer = useCallback(() => {
    if (closeTimerRef.current !== null) {
      window.clearTimeout(closeTimerRef.current);
      closeTimerRef.current = null;
    }
  }, []);

  const updateTooltipLayout = useCallback(() => {
    if (!activeTooltip || !bubbleRef.current) {
      setTooltipLayout(null);
      return;
    }
    if (!activeTooltip.target.isConnected) {
      if (activeTooltip.phase !== "closing") {
        setTooltipLayout(null);
      }
      return;
    }

    const bubbleRect = bubbleRef.current.getBoundingClientRect();
    const anchorRect = activeTooltip.target.getBoundingClientRect();
    const viewportWidth = window.innerWidth;
    const viewportHeight = window.innerHeight;
    const anchorCenterX = anchorRect.left + (anchorRect.width / 2);

    const nextLeft = clamp(
      anchorCenterX - (bubbleRect.width / 2),
      TOOLTIP_MARGIN,
      viewportWidth - bubbleRect.width - TOOLTIP_MARGIN,
    );
    const nextTop = clamp(
      anchorRect.top - bubbleRect.height - TOOLTIP_GAP,
      TOOLTIP_MARGIN,
      viewportHeight - bubbleRect.height - TOOLTIP_MARGIN,
    );
    const nextArrowLeft = clamp(anchorCenterX - nextLeft, 20, bubbleRect.width - 20);

    setTooltipLayout((previous) => {
      if (
        previous
        && Math.abs(previous.left - nextLeft) < 0.5
        && Math.abs(previous.top - nextTop) < 0.5
        && Math.abs(previous.arrowLeft - nextArrowLeft) < 0.5
      ) {
        return previous;
      }
      return {
        arrowLeft: nextArrowLeft,
        left: nextLeft,
        top: nextTop,
      };
    });
  }, [activeTooltip]);

  useLayoutEffect(() => {
    updateTooltipLayout();
  }, [activeTooltip, updateTooltipLayout]);

  useEffect(() => {
    if (!activeTooltip) {
      return;
    }

    const refresh = () => updateTooltipLayout();
    window.addEventListener("resize", refresh);
    document.addEventListener("scroll", refresh, true);

    return () => {
      window.removeEventListener("resize", refresh);
      document.removeEventListener("scroll", refresh, true);
    };
  }, [activeTooltip, updateTooltipLayout]);

  useEffect(() => () => clearCloseTimer(), [clearCloseTimer]);

  const showTooltip = useCallback((target: HTMLSpanElement, text: string, color: string) => {
    clearCloseTimer();
    tooltipNonceRef.current += 1;
    setTooltipLayout(null);
    setActiveTooltip({
      color,
      nonce: tooltipNonceRef.current,
      phase: "opening",
      target,
      text,
    });
  }, [clearCloseTimer]);

  const hideTooltip = useCallback((target?: EventTarget | null) => {
    clearCloseTimer();
    setActiveTooltip((current) => {
      if (!current) {
        return current;
      }
      if (target && current.target !== target) {
        return current;
      }
      if (current.phase === "closing") {
        return current;
      }
      return {
        ...current,
        phase: "closing",
      };
    });
    closeTimerRef.current = window.setTimeout(() => {
      setActiveTooltip((current) => (current?.phase === "closing" ? null : current));
      setTooltipLayout((current) => (current ? null : current));
      closeTimerRef.current = null;
    }, TOOLTIP_CLOSE_MS);
  }, [clearCloseTimer]);

  return (
    <>
      <div
        className="detail-editor confidence-transcript"
        style={{ fontSize: `${fontSize}px` }}
        role="document"
        aria-label={t("confidence.transcriptAria", "Confidence-colored transcript")}
      >
        {transcriptDocument.fragments.map((fragment, index) => {
          if (!fragment.color || fragment.confidence === null || !fragment.tooltip) {
            return <span key={`${index}-${fragment.text.length}`}>{fragment.text}</span>;
          }

          const tooltipText = fragment.tooltip;
          const tooltipColor = fragment.color;

          return (
            <span
              key={`${index}-${fragment.text.length}`}
              className="confidence-word"
              style={{
                color: tooltipColor,
                "--confidence-color": tooltipColor,
              } as CSSProperties}
              tabIndex={0}
              aria-label={tooltipText}
              onMouseEnter={(event) =>
                showTooltip(event.currentTarget, tooltipText, tooltipColor)}
              onMouseLeave={(event) => hideTooltip(event.currentTarget)}
              onFocus={(event) =>
                showTooltip(event.currentTarget, tooltipText, tooltipColor)}
              onBlur={(event) => hideTooltip(event.currentTarget)}
            >
              {fragment.text}
            </span>
          );
        })}
      </div>

      {activeTooltip
        ? createPortal(
          <div className="confidence-tooltip-layer" aria-hidden="true">
            <div
              key={activeTooltip.nonce}
              className={
                `confidence-tooltip-shell${
                  tooltipLayout && activeTooltip.phase !== "closing" ? " is-visible" : ""
                }${activeTooltip.phase === "closing" ? " is-closing" : ""}`
              }
              style={{
                left: `${tooltipLayout?.left ?? -9999}px`,
                top: `${tooltipLayout?.top ?? -9999}px`,
                opacity: tooltipLayout ? 1 : 0,
                "--confidence-color": activeTooltip.color,
                "--tooltip-arrow-left": `${tooltipLayout?.arrowLeft ?? 20}px`,
              } as CSSProperties}
            >
              <div ref={bubbleRef} className="confidence-tooltip-bubble">
                <span className="confidence-tooltip-text">{activeTooltip.text}</span>
              </div>
              <span className="confidence-tooltip-arrow" />
            </div>
          </div>,
          globalThis.document.body,
        )
        : null}
    </>
  );
}
