import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const currentDir = path.dirname(fileURLToPath(import.meta.url));
const appSource = fs.readFileSync(path.resolve(currentDir, "../App.tsx"), "utf8");
const cssSource = fs.readFileSync(path.resolve(currentDir, "../styles.css"), "utf8");

function extractBlock(source: string, selector: string): string {
  const startToken = `${selector} {`;
  const startIndex = source.indexOf(startToken);
  if (startIndex < 0) return "";
  const blockStart = source.indexOf("{", startIndex);
  let depth = 0;
  for (let index = blockStart; index < source.length; index += 1) {
    const ch = source[index];
    if (ch === "{") depth += 1;
    if (ch === "}") {
      depth -= 1;
      if (depth === 0) return source.slice(blockStart + 1, index);
    }
  }
  return "";
}

describe("update UI placement", () => {
  it("keeps the compact updater control scoped to the left sidebar", () => {
    expect(appSource).toContain("renderSidebarUpdateButton()");
    expect(appSource).not.toContain("renderCompactUpdateButton");
    expect(appSource).not.toContain("topbar-update-button");
    expect(cssSource).toContain(".sidebar-update-pill");
    expect(cssSource).not.toContain(".topbar-update-button");
  });

  it("does not render the compact update pill in the standalone settings header", () => {
    const settingsWindowStart = appSource.indexOf(
      "if (standaloneSettingsWindow) {\n    return (",
    );
    const mainShellStart = appSource.indexOf('className="app-shell"', settingsWindowStart);
    const settingsWindowSource = appSource.slice(settingsWindowStart, mainShellStart);

    expect(settingsWindowSource).toContain("settings-window-header");
    expect(settingsWindowSource).not.toContain("renderSidebarUpdateButton()");
  });

  it("keeps sidebar update pill clickable instead of disabled", () => {
    const pillStart = appSource.indexOf('className={`sidebar-update-pill');
    const pillEnd = appSource.indexOf("</button>", pillStart);
    const pillSource = appSource.slice(pillStart, pillEnd);
    const pillCss = extractBlock(cssSource, ".sidebar-update-pill");

    expect(pillSource).toContain("onClick");
    expect(pillSource).not.toContain("disabled=");
    expect(pillCss).toContain("cursor: pointer");
    expect(pillCss).not.toContain("not-allowed");
  });

  it("clears the checking state through the native updater return path", () => {
    const refreshStart = appSource.indexOf("async function refreshUpdates");
    const refreshEnd = appSource.indexOf(
      "async function syncNativeUpdateForVersion",
      refreshStart,
    );
    const refreshSource = appSource.slice(refreshStart, refreshEnd);
    const nativeAvailableIndex = refreshSource.indexOf(
      'setUpdateInstallPhase("available")',
    );
    const finallyIndex = refreshSource.lastIndexOf("finally");

    expect(nativeAvailableIndex).toBeGreaterThan(-1);
    expect(finallyIndex).toBeGreaterThan(nativeAvailableIndex);
    expect(refreshSource).toContain("setCheckingUpdates(false)");
  });
});
