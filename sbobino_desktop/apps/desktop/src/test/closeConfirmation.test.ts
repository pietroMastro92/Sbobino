import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const currentDir = path.dirname(fileURLToPath(import.meta.url));
const appSource = fs.readFileSync(path.resolve(currentDir, "../App.tsx"), "utf8");
const capabilitySource = fs.readFileSync(
  path.resolve(currentDir, "../../src-tauri/capabilities/default.json"),
  "utf8",
);

function extractFunction(source: string, name: string): string {
  const start = source.indexOf(`function ${name}`);
  if (start < 0) return "";
  const bodyStart = source.indexOf("{", start);
  let depth = 0;
  for (let index = bodyStart; index < source.length; index += 1) {
    const char = source[index];
    if (char === "{") depth += 1;
    if (char === "}") {
      depth -= 1;
      if (depth === 0) return source.slice(start, index + 1);
    }
  }
  return "";
}

describe("close and cancel confirmations", () => {
  it("intercepts main-window close and offers quit or keep-open", () => {
    expect(appSource).toContain(".onCloseRequested((event) =>");
    expect(appSource).toContain("event.preventDefault()");
    expect(appSource).toContain("appClose.quitButton");
    expect(appSource).toContain("appClose.keepOpenButton");
    expect(appSource).toContain("await exitProcess(0)");
    expect(appSource).toContain("await appWindow.hide()");
    expect(appSource).not.toContain("await appWindow.minimize()");
  });

  it("allows the Tauri window commands used by the close dialog", () => {
    const capability = JSON.parse(capabilitySource) as {
      permissions?: string[];
    };

    expect(capability.permissions).toContain("core:window:allow-hide");
    expect(capability.permissions).not.toContain("core:window:allow-close");
    expect(capability.permissions).not.toContain("core:window:allow-minimize");
  });

  it("reopens the hidden main window from the macOS Dock icon", () => {
    const libSource = fs.readFileSync(
      path.resolve(currentDir, "../../src-tauri/src/lib.rs"),
      "utf8",
    );

    expect(libSource).toContain("RunEvent::Reopen");
    expect(libSource).toContain("show_main_window");
    expect(libSource).toContain("main_window.show()");
    expect(libSource).toContain("main_window.set_focus()");
  });

  it("asks before cancelling an active transcription", () => {
    const onCancel = extractFunction(appSource, "onCancel");

    expect(onCancel).toContain("confirmDialog(");
    expect(onCancel).toContain("transcriptionCancel.title");
    expect(onCancel).toContain("transcriptionCancel.confirmButton");
    expect(onCancel).toContain("transcriptionCancel.keepRunning");
    expect(onCancel).toContain("await cancelTranscription(activeJobId)");
  });
});
