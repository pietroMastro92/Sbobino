import { useState } from "react";

import { useTranslation } from "../i18n";
import type {
  PersonalizationEntry,
  PersonalizationEntryKind,
  PersonalizationSettings,
} from "../types";

type EntryDraft = {
  kind: PersonalizationEntryKind;
  source_text: string;
  replacement_text: string;
  language_code: string;
};

const EMPTY_DRAFT: EntryDraft = {
  kind: "vocabulary",
  source_text: "",
  replacement_text: "",
  language_code: "",
};

export function PersonalizationSettingsPanel({
  settings,
  entries,
  busy,
  onSettingsChange,
  onUpsert,
  onDelete,
  onClear,
}: {
  settings: PersonalizationSettings;
  entries: PersonalizationEntry[];
  busy: boolean;
  onSettingsChange: (settings: PersonalizationSettings) => Promise<void>;
  onUpsert: (entry: PersonalizationEntry) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
  onClear: () => Promise<void>;
}): JSX.Element {
  const { t } = useTranslation();
  const [draft, setDraft] = useState<EntryDraft>(EMPTY_DRAFT);
  const [editingId, setEditingId] = useState<string | null>(null);

  async function addEntry(): Promise<void> {
    const sourceText = draft.source_text.trim();
    const replacementText = draft.replacement_text.trim();
    if (!sourceText || (draft.kind === "correction" && !replacementText)) return;
    const now = new Date().toISOString();
    const existing = entries.find((entry) => entry.id === editingId);
    await onUpsert({
      id: existing?.id
        ?? globalThis.crypto?.randomUUID?.()
        ?? `personalization-${Date.now()}`,
      kind: draft.kind,
      source_text: sourceText,
      replacement_text: draft.kind === "correction" ? replacementText : null,
      language_code: draft.language_code.trim() || null,
      enabled: existing?.enabled ?? true,
      hit_count: existing?.hit_count ?? 0,
      created_at: existing?.created_at ?? now,
      updated_at: now,
    });
    setDraft(EMPTY_DRAFT);
    setEditingId(null);
  }

  return (
    <div className="settings-stack personalization-settings">
      <section className="settings-panel">
        <header>
          <h3>{t("settings.personalization.title", "Personalization")}</h3>
          <p>
            {t(
              "settings.personalization.desc",
              "Keep vocabulary and correction memory encrypted on this device.",
            )}
          </p>
        </header>

        <div className="settings-row">
          <div>
            <strong>{t("settings.personalization.enabled", "Use personalization")}</strong>
            <small>
              {t(
                "settings.personalization.enabledDesc",
                "Disabling this prevents saved entries from affecting new transcripts.",
              )}
            </small>
          </div>
          <input
            type="checkbox"
            checked={settings.enabled}
            disabled={busy}
            onChange={(event) => void onSettingsChange({
              ...settings,
              enabled: event.target.checked,
            })}
          />
        </div>

        <div className="settings-row">
          <div>
            <strong>
              {t("settings.personalization.autoApply", "Auto-apply exact corrections")}
            </strong>
            <small>
              {t(
                "settings.personalization.autoApplyDesc",
                "Off by default. Only exact, whole-word remembered corrections are applied.",
              )}
            </small>
          </div>
          <input
            type="checkbox"
            checked={settings.auto_apply_safe_corrections}
            disabled={busy || !settings.enabled}
            onChange={(event) => void onSettingsChange({
              ...settings,
              auto_apply_safe_corrections: event.target.checked,
            })}
          />
        </div>
      </section>

      <section className="settings-panel">
        <header>
          <h3>{t("settings.personalization.entries", "Vocabulary and corrections")}</h3>
          <p>
            {t(
              "settings.personalization.entriesDesc",
              "Vocabulary biases supported local engines; corrections remain suggestions unless automatic application is enabled.",
            )}
          </p>
        </header>

        <div className="personalization-entry-form">
          <select
            aria-label={t("settings.personalization.kind", "Entry type")}
            value={draft.kind}
            disabled={busy}
            onChange={(event) => setDraft((current) => ({
              ...current,
              kind: event.target.value as PersonalizationEntryKind,
            }))}
          >
            <option value="vocabulary">
              {t("settings.personalization.vocabulary", "Vocabulary")}
            </option>
            <option value="correction">
              {t("settings.personalization.correction", "Correction")}
            </option>
          </select>
          <input
            aria-label={t("settings.personalization.sourceText", "Word or original text")}
            placeholder={t("settings.personalization.sourcePlaceholder", "e.g. Sbobino")}
            value={draft.source_text}
            disabled={busy}
            onChange={(event) => setDraft((current) => ({
              ...current,
              source_text: event.target.value,
            }))}
          />
          {draft.kind === "correction" ? (
            <input
              aria-label={t("settings.personalization.replacementText", "Replacement text")}
              placeholder={t("settings.personalization.replacementPlaceholder", "Correct spelling")}
              value={draft.replacement_text}
              disabled={busy}
              onChange={(event) => setDraft((current) => ({
                ...current,
                replacement_text: event.target.value,
              }))}
            />
          ) : null}
          <input
            aria-label={t("settings.personalization.language", "Language (optional)")}
            placeholder={t("settings.personalization.languagePlaceholder", "e.g. it")}
            value={draft.language_code}
            disabled={busy}
            onChange={(event) => setDraft((current) => ({
              ...current,
              language_code: event.target.value,
            }))}
          />
          <button
            type="button"
            className="primary-button"
            disabled={
              busy
              || !draft.source_text.trim()
              || (draft.kind === "correction" && !draft.replacement_text.trim())
            }
            onClick={() => void addEntry()}
          >
            {editingId
              ? t("settings.personalization.saveChanges", "Save changes")
              : t("settings.personalization.add", "Add entry")}
          </button>
          {editingId ? (
            <button
              type="button"
              className="secondary-button"
              disabled={busy}
              onClick={() => {
                setEditingId(null);
                setDraft(EMPTY_DRAFT);
              }}
            >
              {t("common.cancel", "Cancel")}
            </button>
          ) : null}
        </div>

        {entries.length === 0 ? (
          <div className="settings-empty-state">
            {t("settings.personalization.empty", "No saved personalization entries.")}
          </div>
        ) : (
          <div className="personalization-entry-list">
            {entries.map((entry) => (
              <div className="personalization-entry" key={entry.id}>
                <div>
                  <strong>{entry.source_text}</strong>
                  {entry.replacement_text ? <span> → {entry.replacement_text}</span> : null}
                  <small>
                    {entry.kind === "vocabulary"
                      ? t("settings.personalization.vocabulary", "Vocabulary")
                      : t("settings.personalization.correction", "Correction")}
                    {entry.language_code ? ` · ${entry.language_code}` : ""}
                    {entry.hit_count > 0 ? ` · ${entry.hit_count} hits` : ""}
                  </small>
                </div>
                <div className="personalization-entry-actions">
                  <button
                    type="button"
                    className="icon-button"
                    disabled={busy}
                    onClick={() => {
                      setEditingId(entry.id);
                      setDraft({
                        kind: entry.kind,
                        source_text: entry.source_text,
                        replacement_text: entry.replacement_text ?? "",
                        language_code: entry.language_code ?? "",
                      });
                    }}
                    aria-label={t("settings.personalization.edit", "Edit entry")}
                  >
                    ✎
                  </button>
                  <label>
                    <input
                      type="checkbox"
                      checked={entry.enabled}
                      disabled={busy}
                      aria-label={t("settings.personalization.entryEnabled", "Entry enabled")}
                      onChange={(event) => void onUpsert({
                        ...entry,
                        enabled: event.target.checked,
                        updated_at: new Date().toISOString(),
                      })}
                    />
                  </label>
                  <button
                    type="button"
                    className="icon-button danger"
                    disabled={busy}
                    onClick={() => void onDelete(entry.id)}
                    aria-label={t("settings.personalization.delete", "Delete entry")}
                  >
                    ×
                  </button>
                </div>
              </div>
            ))}
          </div>
        )}

        {entries.length > 0 ? (
          <button
            type="button"
            className="secondary-button danger"
            disabled={busy}
            onClick={() => void onClear()}
          >
            {t("settings.personalization.clear", "Delete all personalization data")}
          </button>
        ) : null}
      </section>
    </div>
  );
}
