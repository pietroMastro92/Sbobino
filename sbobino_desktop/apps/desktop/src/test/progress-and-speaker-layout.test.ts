import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const currentDir = path.dirname(fileURLToPath(import.meta.url));
const cssPath = path.resolve(currentDir, "../styles.css");
const appPath = path.resolve(currentDir, "../App.tsx");
const cssSource = fs.readFileSync(cssPath, "utf8");
const appSource = fs.readFileSync(appPath, "utf8");

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

describe("progress and speaker layout", () => {
  it("uses a dedicated stable progress dot for diarization", () => {
    expect(appSource).toContain("transcribing-cancel-pill--diarization");
    expect(appSource).toContain('toolbarFocusedProgress?.stage === "diarizing"');
    expect(appSource).toContain("title={postProcessingProgress.text}");
    expect(extractBlock(cssSource, ".transcribing-cancel-pill--diarization")).toContain("#7c3aed");
  });

  it("does not move speaker chips or reveal actions by changing layout on hover", () => {
    expect(extractBlock(cssSource, ".speaker-chip-list")).toContain("display: grid;");
    expect(extractBlock(cssSource, ".speaker-chip-row")).toContain("grid-template-columns:");
    expect(extractBlock(cssSource, ".speaker-chip-inline-remove")).not.toContain("position: absolute;");
    expect(cssSource).not.toContain(".speaker-chip-row:hover .speaker-chip-button");
    expect(cssSource).not.toContain(".speaker-chip-row:focus-within .speaker-chip-button");
    expect(cssSource).not.toContain("padding-right: 38px;");
  });
});
