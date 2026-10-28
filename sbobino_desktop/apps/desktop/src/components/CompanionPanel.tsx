import { useCallback, useEffect, useRef, useState } from "react";
import QRCode from "qrcode";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "../i18n";

type Pairing = {
  version: number;
  endpoint: string;
  token: string;
  certificate: string;
  desktop_name: string;
  desktop_version: string;
};

type CompanionAiProvider = "foundation_apple" | "ollama";

type CompanionAiOption = {
  id: string;
  provider: CompanionAiProvider;
  label: string;
  model: string;
};

type CompanionAiHealth = {
  available: boolean;
  provider: string | null;
  model: string | null;
  unavailable_reason: string | null;
  limits: Record<string, unknown>;
};

type CompanionAiStatus = {
  selection: string | null;
  options: CompanionAiOption[];
  health: CompanionAiHealth;
};

type AiAction = "probe" | "select" | null;

const QR_WIDTH = 384;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isCompanionAiProvider(value: unknown): value is CompanionAiProvider {
  return value === "foundation_apple" || value === "ollama";
}

function normalizeCompanionAiStatus(value: unknown): CompanionAiStatus | null {
  if (!isRecord(value)) return null;

  const rawOptions = Array.isArray(value.options) ? value.options : [];
  const options = rawOptions.filter((option): option is CompanionAiOption => {
    if (!isRecord(option)) return false;
    const provider = option.provider;
    const idMatchesProvider =
      (provider === "foundation_apple" && option.id === "foundation_apple") ||
      (provider === "ollama" && typeof option.id === "string" && option.id.startsWith("remote:"));
    return (
      typeof option.id === "string" &&
      isCompanionAiProvider(option.provider) &&
      idMatchesProvider &&
      typeof option.label === "string" &&
      typeof option.model === "string"
    );
  });
  const rawHealth = isRecord(value.health) ? value.health : {};
  const rawLimits = isRecord(rawHealth.limits) ? rawHealth.limits : {};

  return {
    selection: typeof value.selection === "string" ? value.selection : null,
    options,
    health: {
      available: rawHealth.available === true,
      provider: typeof rawHealth.provider === "string" ? rawHealth.provider : null,
      model: typeof rawHealth.model === "string" ? rawHealth.model : null,
      unavailable_reason:
        typeof rawHealth.unavailable_reason === "string" ? rawHealth.unavailable_reason : null,
      limits: rawLimits,
    },
  };
}

export function CompanionPanel(): JSX.Element {
  const { t } = useTranslation();
  const [pairing, setPairing] = useState<Pairing | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [confirmStop, setConfirmStop] = useState(false);
  const [copyFeedback, setCopyFeedback] = useState("");
  const [qrDataUrl, setQrDataUrl] = useState("");
  const [qrError, setQrError] = useState("");
  const [aiStatus, setAiStatus] = useState<CompanionAiStatus | null>(null);
  const [aiError, setAiError] = useState("");
  const [aiAction, setAiAction] = useState<AiAction>(null);
  const mountedRef = useRef(true);
  const sessionRef = useRef(0);

  const pairingPayload = pairing ? JSON.stringify(pairing) : "";

  const isCurrentSession = useCallback((session: number): boolean => {
    return mountedRef.current && sessionRef.current === session;
  }, []);

  useEffect(() => {
    const requestSession = sessionRef.current;
    let active = true;
    void invoke<Pairing | null>("companion_status")
      .then(value => {
        if (active && isCurrentSession(requestSession)) setPairing(value);
      })
      .catch(() => {
        // The panel keeps its explicit activation flow available when status is unavailable.
      });
    return () => {
      active = false;
    };
  }, [isCurrentSession]);

  useEffect(() => {
    return () => {
      mountedRef.current = false;
      sessionRef.current += 1;
    };
  }, []);

  useEffect(() => {
    const requestSession = sessionRef.current;
    let active = true;

    if (!pairingPayload) {
      setQrDataUrl("");
      setQrError("");
      return () => {
        active = false;
      };
    }

    setQrDataUrl("");
    setQrError("");
    void QRCode.toDataURL(pairingPayload, {
      errorCorrectionLevel: "M" as const,
      margin: 2,
      width: QR_WIDTH,
    })
      .then(dataUrl => {
        if (active && isCurrentSession(requestSession)) setQrDataUrl(dataUrl);
      })
      .catch(() => {
        if (active && isCurrentSession(requestSession)) {
          setQrError(t("companion.qr.unavailable", "QR unavailable. Use Copy code as an alternative."));
        }
      });

    return () => {
      active = false;
    };
  }, [isCurrentSession, pairingPayload]);

  const loadAiStatus = useCallback(
    async (requestSession: number): Promise<void> => {
      if (!isCurrentSession(requestSession)) return;
      setAiAction("probe");
      setAiError("");
      try {
        const rawStatus = await invoke<unknown>("companion_ai_status");
        const nextStatus = normalizeCompanionAiStatus(rawStatus);
        if (!nextStatus) throw new Error("invalid companion AI status");
        if (isCurrentSession(requestSession)) setAiStatus(nextStatus);
      } catch {
        if (isCurrentSession(requestSession)) {
          setAiError(t("companion.error.backendUnavailable", "Backend unavailable. Check the connection and try again."));
        }
      } finally {
        if (isCurrentSession(requestSession)) setAiAction(null);
      }
    },
    [isCurrentSession],
  );

  useEffect(() => {
    if (!pairingPayload) {
      setAiStatus(null);
      setAiError("");
      setAiAction(null);
      return;
    }
    void loadAiStatus(sessionRef.current);
  }, [loadAiStatus, pairingPayload]);

  async function toggle(): Promise<void> {
    const requestSession = sessionRef.current + 1;
    sessionRef.current = requestSession;
    setBusy(true);
    setError("");
    try {
      if (pairing) {
        await invoke("stop_companion");
        if (isCurrentSession(requestSession)) {
          setPairing(null);
          setConfirmStop(false);
          setCopyFeedback("");
        }
      } else {
        const nextPairing = await invoke<Pairing>("start_companion", { lan: true });
        if (isCurrentSession(requestSession)) setPairing(nextPairing);
      }
    } catch {
      if (isCurrentSession(requestSession)) {
        setError(t("companion.error.connection", "Connection unavailable. Check the local network and try again."));
      }
    } finally {
      if (isCurrentSession(requestSession)) setBusy(false);
    }
  }

  async function copyPairing(): Promise<void> {
    if (!pairing) return;
    try {
      await navigator.clipboard.writeText(pairingPayload);
      if (mountedRef.current) {
        setCopyFeedback(t("companion.copy.success", "Code copied. Paste it into the iOS app settings."));
      }
    } catch {
      const field = document.getElementById("companion-pairing") as HTMLTextAreaElement | null;
      field?.focus();
      field?.select();
      if (mountedRef.current) {
        setCopyFeedback(t("companion.copy.fallback", "Code selected: use the computer's Copy command."));
      }
    }
  }

  async function selectAiProvider(selection: string): Promise<void> {
    if (!pairing || !aiStatus) return;
    const requestSession = sessionRef.current;
    if (!isCurrentSession(requestSession)) return;
    setAiAction("select");
    setAiError("");
    try {
      const rawStatus = await invoke<unknown>("select_companion_ai_provider", {
        selection: selection || null,
      });
      const nextStatus = normalizeCompanionAiStatus(rawStatus);
      if (!nextStatus) throw new Error("invalid companion AI status");
      if (isCurrentSession(requestSession)) setAiStatus(nextStatus);
    } catch {
      if (isCurrentSession(requestSession)) {
        setAiError(t("companion.error.aiSelection", "Backend unavailable. The local AI selection was not saved."));
      }
    } finally {
      if (isCurrentSession(requestSession)) setAiAction(null);
    }
  }

  const selectedAiProvider =
    aiStatus?.options.some(option => option.id === aiStatus.selection) ? aiStatus.selection ?? "" : "";
  const aiBusyLabel = aiAction === "select"
    ? t("companion.ai.saving", "Saving…")
    : t("companion.ai.probing", "Checking availability…");

  return (
    <section className="settings-panel">
      <header>
        <h3>{t("companion.title", "iPhone and iPad")}</h3>
        <p>{t("companion.description", "Process longer recordings on the computer. Results are also saved in the desktop archive.")}</p>
      </header>
      <div className="settings-row">
        <div>
          <strong>{pairing ? t("companion.active", "Connection active") : t("companion.inactive", "Connect Sbobbino iOS")}</strong>
          <small>{pairing ? pairing.endpoint : t("companion.networkDescription", "Devices must be on the same network. The connection uses an encrypted session.")}</small>
        </div>
        <button
          type="button"
          onClick={() => {
            if (pairing) setConfirmStop(true);
            else void toggle();
          }}
          disabled={busy}
        >
          {busy
            ? t("companion.busy", "Please wait…")
            : pairing
              ? t("companion.revoke", "Disable")
              : t("companion.enable", "Enable connection")}
        </button>
      </div>
      {pairing && (
        <div className="settings-row settings-row-block">
          <label htmlFor="companion-pairing">{t("companion.pairing.label", "Pairing code")}</label>
          <textarea
            id="companion-pairing"
            rows={4}
            readOnly
            spellCheck={false}
            value={pairingPayload}
            onFocus={event => event.currentTarget.select()}
          />
          <button type="button" onClick={() => void copyPairing()}>
            {t("companion.copy.button", "Copy code")}
          </button>
          <small>
            {t("companion.pairing.help", "In Sbobbino iOS, open Settings → Connect Sbobbino Desktop. The code authorizes access: share it only with your devices.")}
          </small>
          <div
            aria-live="polite"
            role="status"
            style={{ display: "grid", gap: 8, justifyItems: "start", width: "100%" }}
          >
            {qrDataUrl ? (
              <img
                src={qrDataUrl}
                alt={t("companion.qr.alt", "Sbobbino iOS pairing QR code")}
                style={{ display: "block", height: "auto", maxWidth: QR_WIDTH, width: "100%" }}
              />
            ) : qrError ? (
              <span role="alert">{qrError}</span>
            ) : (
              <span>{t("companion.qr.preparing", "Preparing QR code…")}</span>
            )}
          </div>
          <p role="status">{copyFeedback}</p>
        </div>
      )}
      {pairing && (
        <div className="settings-row settings-row-block">
          <div>
            <strong>{t("companion.ai.title", "Local AI for remote transcriptions")}</strong>
            <small>{t("companion.ai.description", "No provider is selected automatically. Explicitly choose a local provider to enable it.")}</small>
          </div>
          {aiStatus ? (
            <>
              <label htmlFor="companion-ai-provider">{t("companion.ai.provider", "Local AI provider")}</label>
              <select
                id="companion-ai-provider"
                value={selectedAiProvider}
                disabled={aiAction !== null || aiStatus.options.length === 0}
                onChange={event => void selectAiProvider(event.currentTarget.value)}
              >
                <option value="">{t("companion.ai.disabled", "Disabled — no provider")}</option>
                {aiStatus.options.map(option => (
                  <option key={option.id} value={option.id}>
                    {option.label} ({option.model})
                  </option>
                ))}
              </select>
              <small>
                {aiStatus.health.available
                  ? t("companion.ai.available", "Available: {provider}", {
                      provider: `${aiStatus.health.provider ?? t("companion.ai.localProvider", "local provider")}${aiStatus.health.model ? ` · ${aiStatus.health.model}` : ""}`,
                    })
                  : aiStatus.health.unavailable_reason ?? t("companion.ai.unavailable", "Local AI provider unavailable.")}
              </small>
              {aiAction && <p role="status">{aiBusyLabel}</p>}
            </>
          ) : (
            <p role="status">{aiAction === "probe" ? aiBusyLabel : t("companion.ai.notVerified", "Local AI status not verified.")}</p>
          )}
          <button
            type="button"
            onClick={() => void loadAiStatus(sessionRef.current)}
            disabled={aiAction !== null}
          >
            {aiAction === "probe"
              ? t("companion.ai.probingButton", "Checking…")
              : t("companion.ai.probe", "Check availability")}
          </button>
          {aiError && <p role="alert">{aiError}</p>}
        </div>
      )}
      {confirmStop && (
        <div role="alert" className="settings-row settings-row-block">
          <p>{t("companion.confirm.revoke", "Disable the connection? The code will be revoked and remote transcriptions still in progress will be cancelled.")}</p>
          <button type="button" disabled={busy} onClick={() => void toggle()}>
            {t("companion.confirm.revokeButton", "Disable and cancel remote jobs")}
          </button>
          <button type="button" disabled={busy} onClick={() => setConfirmStop(false)}>
            {t("companion.confirm.keep", "Keep connection")}
          </button>
        </div>
      )}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
