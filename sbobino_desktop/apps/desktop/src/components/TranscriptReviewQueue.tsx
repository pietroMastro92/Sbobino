import { useEffect, useMemo, useState } from "react";

import { useTranslation } from "../i18n";
import type { TranscriptReviewItem } from "../lib/transcriptReview";

export type TranscriptReviewAction = "confirmed" | "corrected" | "ignored";

export function TranscriptReviewQueue({
  items,
  decisions,
  busy,
  onReview,
}: {
  items: TranscriptReviewItem[];
  decisions: Record<string, TranscriptReviewAction>;
  busy: boolean;
  onReview: (
    item: TranscriptReviewItem,
    action: TranscriptReviewAction,
    replacementText: string,
    rememberCorrection: boolean,
  ) => Promise<void>;
}): JSX.Element | null {
  const { t } = useTranslation();
  const pendingItems = useMemo(
    () => items.filter((item) => !decisions[item.id]),
    [decisions, items],
  );
  const [index, setIndex] = useState(0);
  const [editing, setEditing] = useState(false);
  const [replacementText, setReplacementText] = useState("");
  const [rememberCorrection, setRememberCorrection] = useState(false);

  useEffect(() => {
    setIndex((current) => Math.min(current, Math.max(0, pendingItems.length - 1)));
  }, [pendingItems.length]);

  const item = pendingItems[index];
  useEffect(() => {
    setEditing(false);
    setReplacementText(item?.originalText ?? "");
    setRememberCorrection(false);
  }, [item?.id, item?.originalText]);

  if (items.length === 0) return null;

  if (!item) {
    return (
      <div className="inspector-block transcript-review-card is-complete">
        <strong>{t("review.completeTitle", "Review complete")}</strong>
        <small>
          {t("review.completeDesc", "Every suggested transcript point has been reviewed.")}
        </small>
      </div>
    );
  }

  const confidenceLabel = item.confidence === null
    ? t("review.confidenceUnavailable", "Confidence unavailable")
    : t("review.confidencePercent", "{percent}% confidence", {
      percent: Math.round(item.confidence * 100),
    });

  async function submit(
    action: TranscriptReviewAction,
    replacement = "",
    remember = false,
  ): Promise<void> {
    await onReview(item, action, replacement, remember);
    setEditing(false);
  }

  return (
    <div className="inspector-block transcript-review-card">
      <div className="transcript-review-head">
        <div>
          <strong>{t("review.title", "Review queue")}</strong>
          <small>
            {t("review.progress", "{current} of {total} remaining", {
              current: index + 1,
              total: pendingItems.length,
            })}
          </small>
        </div>
        <span className={item.confidence === null ? "status-chip warning" : "status-chip"}>
          {confidenceLabel}
        </span>
      </div>

      <blockquote className="transcript-review-context">{item.contextText}</blockquote>
      <div className="transcript-review-target">{item.originalText}</div>

      {editing ? (
        <div className="transcript-review-editor">
          <label>
            <span>{t("review.replacement", "Correction")}</span>
            <input
              autoFocus
              value={replacementText}
              onChange={(event) => setReplacementText(event.target.value)}
              disabled={busy}
            />
          </label>
          <label className="transcript-review-remember">
            <input
              type="checkbox"
              checked={rememberCorrection}
              onChange={(event) => setRememberCorrection(event.target.checked)}
              disabled={busy}
            />
            <span>{t("review.remember", "Remember this correction")}</span>
          </label>
          <div className="transcript-review-actions">
            <button
              type="button"
              className="secondary-button"
              onClick={() => setEditing(false)}
              disabled={busy}
            >
              {t("common.cancel", "Cancel")}
            </button>
            <button
              type="button"
              className="primary-button"
              onClick={() => void submit("corrected", replacementText, rememberCorrection)}
              disabled={busy || !replacementText.trim() || replacementText.trim() === item.originalText}
            >
              {busy ? t("common.saving", "Saving...") : t("review.apply", "Apply correction")}
            </button>
          </div>
        </div>
      ) : (
        <div className="transcript-review-actions">
          <button
            type="button"
            className="secondary-button"
            onClick={() => void submit("ignored")}
            disabled={busy}
          >
            {t("review.ignore", "Ignore")}
          </button>
          <button
            type="button"
            className="secondary-button"
            onClick={() => setEditing(true)}
            disabled={busy}
          >
            {t("review.correct", "Correct")}
          </button>
          <button
            type="button"
            className="primary-button"
            onClick={() => void submit("confirmed")}
            disabled={busy}
          >
            {busy ? t("common.saving", "Saving...") : t("review.confirm", "Confirm")}
          </button>
        </div>
      )}

      {pendingItems.length > 1 ? (
        <div className="transcript-review-nav">
          <button
            type="button"
            className="icon-button"
            onClick={() => setIndex((current) => Math.max(0, current - 1))}
            disabled={index === 0 || busy}
            aria-label={t("review.previous", "Previous review point")}
          >
            ‹
          </button>
          <button
            type="button"
            className="icon-button"
            onClick={() => setIndex((current) => Math.min(pendingItems.length - 1, current + 1))}
            disabled={index >= pendingItems.length - 1 || busy}
            aria-label={t("review.next", "Next review point")}
          >
            ›
          </button>
        </div>
      ) : null}
    </div>
  );
}
