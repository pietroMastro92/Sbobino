import { fireEvent, render, screen, waitFor } from "@testing-library/react";
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
  token: "synthetic",
  certificate: "fixture",
  desktop_name: "Sbobino Desktop",
  desktop_version: "2.0.32",
};

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

describe("CompanionPanel", () => {
  beforeEach(() => {
    changeLanguage("en");
    invoke.mockReset();
    qrToDataUrl.mockReset();
    qrToDataUrl.mockResolvedValue("data:image/png;base64,fixture-qr");
    invoke.mockImplementation(async (command: string) => (command === "start_companion" ? pairing : null));
  });

  it("activates only after an explicit user action, renders the exact QR payload, and copies pairing", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    render(<CompanionPanel />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("companion_status"));
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

  it("ignores a late QR response after the session is revoked", async () => {
    const qrResponse = deferred<string>();
    qrToDataUrl.mockReturnValueOnce(qrResponse.promise);
    invoke.mockImplementation(async (command: string) => {
      if (command === "start_companion") return pairing;
      return null;
    });

    render(<CompanionPanel />);
    fireEvent.click(screen.getByRole("button", { name: "Enable connection" }));
    await screen.findByRole("button", { name: "Disable" });
    expect(screen.queryByRole("img", { name: "Sbobbino iOS pairing QR code" })).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Disable" }));
    fireEvent.click(screen.getByRole("button", { name: "Disable and cancel remote jobs" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("stop_companion"));
    expect(screen.queryByRole("img", { name: "Sbobbino iOS pairing QR code" })).toBeNull();

    qrResponse.resolve("data:image/png;base64,late-qr");
    await Promise.resolve();
    expect(screen.queryByRole("img", { name: "Sbobbino iOS pairing QR code" })).toBeNull();
  });

  it("ignores a late status response after a new session starts", async () => {
    const staleStatus = deferred<typeof pairing | null>();
    invoke.mockImplementation(async (command: string) => {
      if (command === "companion_status") return staleStatus.promise;
      if (command === "start_companion") return pairing;
      return null;
    });

    render(<CompanionPanel />);
    fireEvent.click(screen.getByRole("button", { name: "Enable connection" }));
    await screen.findByRole("button", { name: "Copy code" });
    staleStatus.resolve({ ...pairing, endpoint: "https://198.51.100.1:8443" });
    await Promise.resolve();
    expect(screen.getByDisplayValue(JSON.stringify(pairing))).toBeInTheDocument();
  });

  it("requires confirmation before revoking a running session", async () => {
    invoke.mockResolvedValue(pairing);
    render(<CompanionPanel />);
    fireEvent.click(await screen.findByRole("button", { name: "Disable" }));
    expect(invoke).not.toHaveBeenCalledWith("stop_companion");
    fireEvent.click(screen.getByRole("button", { name: "Disable and cancel remote jobs" }));
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("stop_companion"));
  });

  it("keeps AI disabled by default, offers only local options, and hides backend errors", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "companion_status") return pairing;
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
    expect(await screen.findByRole("alert")).toHaveTextContent("Backend unavailable");
    expect(screen.queryByText("internal token should not render")).toBeNull();
  });

  it("shows an actionable error when the AI backend probe is unavailable", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "companion_status") return pairing;
      if (command === "companion_ai_status") throw new Error("secret probe details");
      return null;
    });

    render(<CompanionPanel />);
    expect(await screen.findByRole("alert")).toHaveTextContent("Backend unavailable");
    expect(screen.queryByText("secret probe details")).toBeNull();
  });

  it("shows an actionable connection error without rendering pairing secrets", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "start_companion") throw new Error("internal synthetic secret");
      return null;
    });
    render(<CompanionPanel />);
    await waitFor(() => expect(invoke).toHaveBeenCalledWith("companion_status"));
    fireEvent.click(screen.getByRole("button", { name: "Enable connection" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Check the local network");
    expect(screen.queryByText("internal synthetic secret")).toBeNull();
  });
});
