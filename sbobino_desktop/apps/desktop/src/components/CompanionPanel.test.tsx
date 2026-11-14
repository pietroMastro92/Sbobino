import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { StrictMode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CompanionPanel } from "./CompanionPanel";
import { changeLanguage } from "../i18n";

const invoke = vi.hoisted(() => vi.fn());
const qrToDataUrl = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("qrcode", () => ({
  default: { toDataURL: qrToDataUrl },
  toDataURL: qrToDataUrl,
}));

const pairing = {
  version: 1,
  endpoint: "https://192.0.2.1:8443",
  token: "synthetic-token-must-not-render",
  certificate: "synthetic-certificate-must-not-render",
  desktop_name: "Sbobino Desktop",
  desktop_version: "2.0.32",
};

const activeState = { state: "active", pairing };
const stoppedSavedState = { state: "inactive_paired", pairing: null };
const unpairedState = { state: "inactive_unpaired", pairing: null };
const unknownState = { state: "unknown", pairing: null };

const localAiStatus = {
  selection: null,
  options: [
    {
      id: "foundation_apple",
      provider: "foundation_apple",
      label: "Apple Intelligence locale",
      model: "Foundation",
    },
    {
      id: "remote:ollama",
      provider: "ollama",
      label: "Ollama locale",
      model: "llama3.1",
    },
    {
      id: "remote:cloud",
      provider: "cloud",
      label: "Cloud provider (must not render)",
      model: "cloud-model",
    },
  ],
  health: {
    available: true,
    provider: "ollama",
    model: "llama3.1",
    unavailable_reason: null,
    limits: {},
  },
};

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(nextResolve => {
    resolve = nextResolve;
  });
  return { promise, resolve };
}

function qrImage(): HTMLElement | null {
  return screen.queryByRole("img", { name: "Sbobbino iOS pairing QR code" });
}

function pairingField(): HTMLElement | null {
  return screen.queryByRole("textbox", { name: "Pairing code" });
}

describe("CompanionPanel", () => {
  beforeEach(() => {
    changeLanguage("en");
    invoke.mockReset();
    qrToDataUrl.mockReset();
    qrToDataUrl.mockResolvedValue("data:image/png;base64,fixture-qr");
    invoke.mockImplementation(async (command: string) => {
      if (command === "companion_pairing_state") return unpairedState;
      if (command === "start_companion") return pairing;
      return null;
    });
  });

  it("keeps explicit activation operable after the Strict Mode effect remount", async () => {
    render(<StrictMode><CompanionPanel /></StrictMode>);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("companion_pairing_state"));
    expect(invoke).not.toHaveBeenCalledWith("start_companion", expect.anything());
    fireEvent.click(screen.getByRole("button", { name: "Enable connection" }));
    await screen.findByRole("button", { name: "Stop connection" });
    expect(invoke).toHaveBeenCalledWith("start_companion", { lan: true });
  });

  it("activates only after an explicit user action, renders the exact QR payload, and copies pairing", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    render(<CompanionPanel />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("companion_pairing_state"));
    expect(invoke).not.toHaveBeenCalledWith("start_companion", expect.anything());

    fireEvent.click(screen.getByRole("button", { name: "Enable connection" }));
    await screen.findByRole("button", { name: "Copy code" });
    expect(invoke).toHaveBeenCalledWith("start_companion", { lan: true });
    await screen.findByRole("img", { name: "Sbobbino iOS pairing QR code" });
    expect(qrToDataUrl).toHaveBeenCalledWith(
      JSON.stringify(pairing),
      expect.objectContaining({ width: 384, margin: 2, errorCorrectionLevel: "M" }),
    );

    fireEvent.click(screen.getByRole("button", { name: "Copy code" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(JSON.stringify(pairing)));
  });

  it("shows a saved pairing on remount as stopped, without secrets, QR or any automatic start", async () => {
    invoke.mockImplementation(async (command: string) =>
      command === "companion_pairing_state" ? stoppedSavedState : null,
    );

    render(<CompanionPanel />);

    expect(await screen.findByText(/The pairing is saved/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Resume connection" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Revoke pairing" })).toBeInTheDocument();
    expect(invoke).not.toHaveBeenCalledWith("start_companion", expect.anything());
    expect(qrToDataUrl).not.toHaveBeenCalled();
    expect(qrImage()).toBeNull();
    expect(pairingField()).toBeNull();
    expect(screen.queryByText(/synthetic-token-must-not-render/)).toBeNull();
    expect(screen.queryByText(/synthetic-certificate-must-not-render/)).toBeNull();
  });

  it("stops the connection, keeps the pairing saved and reloads the real stopped state", async () => {
    let stateCalls = 0;
    invoke.mockImplementation(async (command: string) => {
      if (command === "companion_pairing_state") {
        stateCalls += 1;
        return stateCalls === 1 ? activeState : stoppedSavedState;
      }
      return null;
    });

    render(<CompanionPanel />);
    await screen.findByRole("button", { name: "Stop connection" });
    await screen.findByRole("img", { name: "Sbobbino iOS pairing QR code" });

    fireEvent.click(screen.getByRole("button", { name: "Stop connection" }));
    expect(invoke).not.toHaveBeenCalledWith("stop_companion");
    expect(screen.getByText(/interrupted/i)).toBeInTheDocument();
    expect(screen.getByText(/saved pairing and the local archive stay intact/i)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Stop and interrupt jobs" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("stop_companion"));

    expect(await screen.findByRole("button", { name: "Resume connection" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Revoke pairing" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Stop connection" })).toBeNull();
    expect(qrImage()).toBeNull();
    expect(pairingField()).toBeNull();
    expect(screen.queryByText(/synthetic-token-must-not-render/)).toBeNull();
  });

  it("cancels the stop confirmation without invoking a destructive command", async () => {
    invoke.mockImplementation(async (command: string) =>
      command === "companion_pairing_state" ? activeState : null,
    );

    render(<CompanionPanel />);
    fireEvent.click(await screen.findByRole("button", { name: "Stop connection" }));
    expect(screen.getByText(/interrupted/i)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(screen.queryByText(/interrupted/i)).toBeNull();
    expect(invoke).not.toHaveBeenCalledWith("stop_companion");
    expect(invoke).not.toHaveBeenCalledWith("revoke_companion_pairing");
    expect(screen.getByRole("button", { name: "Stop connection" })).toBeInTheDocument();
  });

  it("revokes an active pairing only after a separate confirmation that requires a new pairing", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "companion_pairing_state") return activeState;
      if (command === "revoke_companion_pairing") return unpairedState;
      return null;
    });

    render(<CompanionPanel />);
    fireEvent.click(await screen.findByRole("button", { name: "Revoke pairing" }));

    expect(invoke).not.toHaveBeenCalledWith("revoke_companion_pairing");
    expect(screen.getByText(/must be paired again/i)).toBeInTheDocument();
    expect(screen.getByText(/local archive stays intact/i)).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Revoke and require new pairing" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("revoke_companion_pairing"));

    expect(await screen.findByRole("button", { name: "Enable connection" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Revoke pairing" })).toBeNull();
    expect(qrImage()).toBeNull();
    expect(pairingField()).toBeNull();
  });

  it("revokes a saved pairing even when the server is stopped", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "companion_pairing_state") return stoppedSavedState;
      if (command === "revoke_companion_pairing") return unpairedState;
      return null;
    });

    render(<CompanionPanel />);
    fireEvent.click(await screen.findByRole("button", { name: "Revoke pairing" }));
    fireEvent.click(screen.getByRole("button", { name: "Revoke and require new pairing" }));

    await waitFor(() => expect(invoke).toHaveBeenCalledWith("revoke_companion_pairing"));
    expect(await screen.findByRole("button", { name: "Enable connection" })).toBeInTheDocument();
    expect(invoke).not.toHaveBeenCalledWith("start_companion", expect.anything());
  });

  it("cancels the revoke confirmation without any destructive invoke", async () => {
    invoke.mockImplementation(async (command: string) =>
      command === "companion_pairing_state" ? activeState : null,
    );

    render(<CompanionPanel />);
    fireEvent.click(await screen.findByRole("button", { name: "Revoke pairing" }));
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(invoke).not.toHaveBeenCalledWith("revoke_companion_pairing");
    expect(invoke).not.toHaveBeenCalledWith("stop_companion");
    expect(screen.getByRole("button", { name: "Stop connection" })).toBeInTheDocument();
  });

  it("reports an unconfirmed revocation and shows the real stopped state without reviving the QR", async () => {
    let stateCalls = 0;
    invoke.mockImplementation(async (command: string) => {
      if (command === "companion_pairing_state") {
        stateCalls += 1;
        return stateCalls === 1 ? activeState : stoppedSavedState;
      }
      if (command === "revoke_companion_pairing") throw new Error("internal revoke secret");
      return null;
    });

    render(<CompanionPanel />);
    fireEvent.click(await screen.findByRole("button", { name: "Revoke pairing" }));
    fireEvent.click(screen.getByRole("button", { name: "Revoke and require new pairing" }));

    expect(await screen.findByText(/Revocation not confirmed/i)).toBeInTheDocument();
    expect(await screen.findByRole("button", { name: "Resume connection" })).toBeInTheDocument();
    expect(screen.queryByText(/internal revoke secret/)).toBeNull();
    expect(qrImage()).toBeNull();
    expect(pairingField()).toBeNull();
  });

  it("ignores a late QR response after the connection is stopped", async () => {
    const qrResponse = deferred<string>();
    qrToDataUrl.mockReturnValueOnce(qrResponse.promise);
    let stateCalls = 0;
    invoke.mockImplementation(async (command: string) => {
      if (command === "companion_pairing_state") {
        stateCalls += 1;
        return stateCalls === 1 ? activeState : stoppedSavedState;
      }
      return null;
    });

    render(<CompanionPanel />);
    fireEvent.click(await screen.findByRole("button", { name: "Stop connection" }));
    fireEvent.click(screen.getByRole("button", { name: "Stop and interrupt jobs" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("stop_companion"));
    await screen.findByRole("button", { name: "Resume connection" });
    expect(qrImage()).toBeNull();

    qrResponse.resolve("data:image/png;base64,late-qr");
    await Promise.resolve();
    expect(qrImage()).toBeNull();
  });

  it("ignores a late status response after a new session starts", async () => {
    const staleStatus = deferred<typeof activeState>();
    invoke.mockImplementation(async (command: string) => {
      if (command === "companion_pairing_state") return staleStatus.promise;
      if (command === "start_companion") return pairing;
      return null;
    });

    render(<CompanionPanel />);
    fireEvent.click(screen.getByRole("button", { name: "Enable connection" }));
    await screen.findByRole("button", { name: "Copy code" });
    staleStatus.resolve({ state: "active", pairing: { ...pairing, endpoint: "https://198.51.100.1:8443" } });
    await Promise.resolve();
    expect(screen.getByDisplayValue(JSON.stringify(pairing))).toBeInTheDocument();
  });

  it("ignores a late AI status response after the pairing is revoked", async () => {
    const aiResponse = deferred<typeof localAiStatus>();
    invoke.mockImplementation(async (command: string) => {
      if (command === "companion_pairing_state") return activeState;
      if (command === "companion_ai_status") return aiResponse.promise;
      if (command === "revoke_companion_pairing") return unpairedState;
      return null;
    });

    render(<CompanionPanel />);
    fireEvent.click(await screen.findByRole("button", { name: "Revoke pairing" }));
    fireEvent.click(screen.getByRole("button", { name: "Revoke and require new pairing" }));
    await screen.findByRole("button", { name: "Enable connection" });

    aiResponse.resolve(localAiStatus);
    await Promise.resolve();
    expect(screen.queryByRole("combobox", { name: "Local AI provider" })).toBeNull();
    expect(screen.queryByText(/Cloud provider \(must not render\)/)).toBeNull();
  });

  it("disables competing actions while a command is in flight", async () => {
    const pendingStart = deferred<typeof pairing>();
    invoke.mockImplementation(async (command: string) => {
      if (command === "companion_pairing_state") return unpairedState;
      if (command === "start_companion") return pendingStart.promise;
      return null;
    });

    render(<CompanionPanel />);
    fireEvent.click(screen.getByRole("button", { name: "Enable connection" }));

    const busyButton = await screen.findByRole("button", { name: "Please wait…" });
    expect(busyButton).toBeDisabled();
    fireEvent.click(busyButton);
    await waitFor(() => expect(invoke).toHaveBeenCalledTimes(2));
    expect(invoke.mock.calls.filter(([command]) => command === "start_companion")).toHaveLength(1);
  });

  it("shows an unknown state when the saved pairing cannot be read, without starting anything", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "companion_pairing_state") throw new Error("internal fs path secret");
      return null;
    });

    render(<CompanionPanel />);

    expect(await screen.findByText(/could not be read/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Check again" })).toBeInTheDocument();
    expect(screen.queryByText(/internal fs path secret/)).toBeNull();
    expect(invoke).not.toHaveBeenCalledWith("start_companion", expect.anything());
  });

  it("keeps AI disabled by default, offers only local options, and hides backend errors", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "companion_pairing_state") return activeState;
      if (command === "companion_ai_status") return localAiStatus;
      if (command === "select_companion_ai_provider") throw new Error("internal token should not render");
      return null;
    });

    render(<CompanionPanel />);
    const providerSelect = await screen.findByRole("combobox", { name: "Local AI provider" });
    expect(providerSelect).toHaveValue("");
    expect(screen.queryByText("Cloud provider (must not render)")).toBeNull();
    expect(screen.getByRole("button", { name: "Check availability" })).toBeInTheDocument();

    fireEvent.change(providerSelect, { target: { value: "remote:ollama" } });
    await waitFor(() =>
      expect(invoke).toHaveBeenCalledWith("select_companion_ai_provider", { selection: "remote:ollama" }),
    );
    expect(await screen.findByText(/Backend unavailable/)).toBeInTheDocument();
    expect(screen.queryByText("internal token should not render")).toBeNull();
  });

  it("shows an actionable error when the AI backend probe is unavailable", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "companion_pairing_state") return activeState;
      if (command === "companion_ai_status") throw new Error("secret probe details");
      return null;
    });

    render(<CompanionPanel />);
    expect(await screen.findByText(/Backend unavailable/)).toBeInTheDocument();
    expect(screen.queryByText("secret probe details")).toBeNull();
  });

  it("shows an actionable connection error without rendering pairing secrets", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "companion_pairing_state") return unpairedState;
      if (command === "start_companion") throw new Error("internal synthetic secret");
      return null;
    });
    render(<CompanionPanel />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("companion_pairing_state"));
    fireEvent.click(screen.getByRole("button", { name: "Enable connection" }));
    expect(await screen.findByText(/Check the local network/)).toBeInTheDocument();
    expect(screen.queryByText("internal synthetic secret")).toBeNull();
  });
});
