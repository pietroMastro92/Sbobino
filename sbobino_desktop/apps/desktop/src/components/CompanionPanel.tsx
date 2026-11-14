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

type PairingStateName = "active" | "inactive_paired" | "inactive_unpaired" | "unknown";

type PairingState = {
  state: PairingStateName;
  pairing: Pairing | null;
};

type ConfirmAction = "stop" | "revoke";

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

function normalizePairing(value: unknown): Pairing | null {
  if (!isRecord(value)) return null;
  if (
    typeof value.version !== "number" ||
    typeof value.endpoint !== "string" ||
    typeof value.token !== "string" ||
    typeof value.certificate !== "string" ||
    typeof value.desktop_name !== "string" ||
    typeof value.desktop_version !== "string"
  ) {
    return null;
  }
  return {
    version: value.version,
    endpoint: value.endpoint,
    token: value.token,
    certificate: value.certificate,
    desktop_name: value.desktop_name,
    desktop_version: value.desktop_version,
  };
}

function normalizePairingState(value: unknown): PairingState | null {
  if (!isRecord(value)) return null;
  if (value.state === "active") {
    const pairing = normalizePairing(value.pairing);
    // An active state without a complete pairing payload cannot be rendered
    // safely; the panel reports an unknown state instead of guessing.
    return pairing ? { state: "active", pairing } : null;
  }
  if (value.state === "inactive_paired" || value.state === "inactive_unpaired" || value.state === "unknown") {
    // Inactive states never expose certificate, token or key material.
    return { state: value.state, pairing: null };
  }
  return null;
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
  const [pairingState, setPairingState] = useState<PairingStateName>("inactive_unpaired");
  const [pairing, setPairing] = useState<Pairing | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [confirmAction, setConfirmAction] = useState<ConfirmAction | null>(null);
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

  const clearSessionUi = useCallback((): void => {
    setCopyFeedback("");
    setAiStatus(null);
    setAiError("");
    setAiAction(null);
  }, []);

  const applyPairingState = useCallback(
    (next: PairingState, requestSession: number): void => {
      if (!isCurrentSession(requestSession)) return;
      setPairing(next.pairing);
      setPairingState(next.state);
      setConfirmAction(null);
      if (!next.pairing) clearSessionUi();
    },
    [clearSessionUi, isCurrentSession],
  );

  const loadPairingState = useCallback(
    async (requestSession: number): Promise<void> => {
      try {
        const rawState = await invoke<unknown>("companion_pairing_state");
        const nextState = normalizePairingState(rawState);
        if (!nextState) throw new Error("invalid companion pairing state");
        applyPairingState(nextState, requestSession);
      } catch {
        // A read failure is reported as unknown: paired and unpaired are
        // opposite claims and neither may be guessed. Nothing is started.
        if (isCurrentSession(requestSession)) {
          setPairing(null);
          setPairingState("unknown");
          setConfirmAction(null);
          clearSessionUi();
        }
      }
    },
    [applyPairingState, clearSessionUi, isCurrentSession],
  );

  useEffect(() => {
    void loadPairingState(sessionRef.current);
  }, [loadPairingState]);

  useEffect(() => {
    mountedRef.current = true;
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

  function beginAction(): number {
    // Every new user action invalidates the responses still in flight, so a
    // late status, QR, AI, start, stop or revoke reply can never overwrite the
    // state produced by the latest action.
    const requestSession = sessionRef.current + 1;
    sessionRef.current = requestSession;
    setBusy(true);
    setError("");
    setConfirmAction(null);
    return requestSession;
  }

  async function startConnection(): Promise<void> {
    const requestSession = beginAction();
    try {
      const rawPairing = await invoke<unknown>("start_companion", { lan: true });
      const nextPairing = normalizePairing(rawPairing);
      if (!nextPairing) throw new Error("invalid companion pairing");
      if (isCurrentSession(requestSession)) {
        setPairing(nextPairing);
        setPairingState("active");
      }
    } catch {
      if (isCurrentSession(requestSession)) {
        setError(t("companion.error.connection", "Connection unavailable. Check the local network and try again."));
        // Reload the authoritative state instead of leaving the panel on a
        // guess about what the failed command actually did.
        await loadPairingState(requestSession);
      }
    } finally {
      if (isCurrentSession(requestSession)) setBusy(false);
    }
  }

  async function stopConnection(): Promise<void> {
    const requestSession = beginAction();
    try {
      await invoke("stop_companion");
      if (isCurrentSession(requestSession)) {
        // The listener is closed but the saved pairing stays; the reload below
        // confirms it from metadata instead of assuming it.
        setPairing(null);
        setPairingState("inactive_paired");
        clearSessionUi();
      }
      await loadPairingState(requestSession);
    } catch {
      if (isCurrentSession(requestSession)) {
        setError(t("companion.error.connection", "Connection unavailable. Check the local network and try again."));
        await loadPairingState(requestSession);
      }
    } finally {
      if (isCurrentSession(requestSession)) setBusy(false);
    }
  }

  async function revokePairing(): Promise<void> {
    const requestSession = beginAction();
    try {
      const rawState = await invoke<unknown>("revoke_companion_pairing");
      const nextState = normalizePairingState(rawState);
      if (!nextState) throw new Error("invalid companion pairing state");
      applyPairingState(nextState, requestSession);
    } catch {
      if (isCurrentSession(requestSession)) {
        // Never claim a successful revocation: report the unconfirmed outcome
        // and reload the real state (which may still be stopped-with-pairing).
        setError(t("companion.error.revoke", "Revocation not confirmed. Check the connection state and try again."));
        await loadPairingState(requestSession);
      }
    } finally {
      if (isCurrentSession(requestSession)) setBusy(false);
    }
  }

  function checkAgain(): void {
    const requestSession = sessionRef.current + 1;
    sessionRef.current = requestSession;
    setError("");
    void loadPairingState(requestSession);
  }

  async function copyPairing(): Promise<void> {
    if (!pairing) return;
    const requestSession = sessionRef.current;
    try {
      await navigator.clipboard.writeText(pairingPayload);
      if (isCurrentSession(requestSession)) {
        setCopyFeedback(t("companion.copy.success", "Code copied. Paste it into the iOS app settings."));
      }
    } catch {
      const field = document.getElementById("companion-pairing") as HTMLTextAreaElement | null;
      field?.focus();
      field?.select();
      if (isCurrentSession(requestSession)) {
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

  const isActive = pairingState === "active" && pairing !== null;
  const canRevoke = isActive || pairingState === "inactive_paired";
  const primaryLabel = busy
    ? t("companion.busy", "Please wait…")
    : isActive
      ? t("companion.stop", "Stop connection")
      : pairingState === "inactive_paired"
        ? t("companion.resume", "Resume connection")
        : pairingState === "unknown"
          ? t("companion.retry", "Check again")
          : t("companion.enable", "Enable connection");

  const statusTitle = isActive
    ? t("companion.active", "Connection active")
    : pairingState === "inactive_paired"
      ? t("companion.stopped", "Connection stopped")
      : pairingState === "unknown"
        ? t("companion.state.unknownTitle", "Connection state unavailable")
        : t("companion.inactive", "Connect Sbobbino iOS");
  const statusDetail = isActive
    ? pairing.endpoint
    : pairingState === "inactive_paired"
      ? t("companion.state.paired", "The pairing is saved. Resume the connection to receive new recordings; nothing was started automatically.")
      : pairingState === "unknown"
        ? t("companion.state.unknown", "The saved pairing could not be read. Nothing was started. Check again before enabling the connection.")
        : t("companion.networkDescription", "Devices must be on the same network. The connection uses an encrypted session.");

  const selectedAiProvider =
    aiStatus?.options.some(option => option.id === aiStatus.selection) ? aiStatus.selection ?? "" : "";
  const aiBusyLabel = aiAction === "select"
    ? t("companion.ai.saving", "Saving…")
    : t("companion.ai.probing", "Checking availability…");

  function primaryAction(): void {
    if (busy) return;
    if (isActive) setConfirmAction("stop");
    else if (pairingState === "unknown") checkAgain();
    else void startConnection();
  }

  return (
    <section className="settings-panel">
      <header>
        <h3>{t("companion.title", "iPhone and iPad")}</h3>
        <p>{t("companion.description", "Process longer recordings on the computer. Results are also saved in the desktop archive.")}</p>
      </header>
      <div className="settings-row">
        <div>
          <strong>{statusTitle}</strong>
          <small>{statusDetail}</small>
        </div>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
          <button type="button" onClick={primaryAction} disabled={busy}>
            {primaryLabel}
          </button>
          {canRevoke && (
            <button
              type="button"
              onClick={() => setConfirmAction("revoke")}
              disabled={busy || confirmAction !== null}
            >
              {t("companion.revokePairing", "Revoke pairing")}
            </button>
          )}
        </div>
      </div>
      {isActive && pairing && (
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
      {isActive && (
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
      {confirmAction && (
        <div role="alert" className="settings-row settings-row-block">
          <p>
            {confirmAction === "stop"
              ? t("companion.confirm.stop", "Stop the connection? Remote transcriptions in progress are interrupted and their result cannot be recovered afterwards. The saved pairing and the local archive stay intact.")
              : t("companion.confirm.revoke", "Revoke the pairing? Remote transcriptions in progress are interrupted and the iOS app must be paired again with a new code. The local archive stays intact.")}
          </p>
          <button
            type="button"
            disabled={busy}
            onClick={() => void (confirmAction === "stop" ? stopConnection() : revokePairing())}
          >
            {confirmAction === "stop"
              ? t("companion.confirm.stopButton", "Stop and interrupt jobs")
              : t("companion.confirm.revokeButton", "Revoke and require new pairing")}
          </button>
          <button type="button" disabled={busy} onClick={() => setConfirmAction(null)}>
            {t("companion.confirm.cancel", "Cancel")}
          </button>
        </div>
      )}
      {error && <p role="alert">{error}</p>}
    </section>
  );
}
