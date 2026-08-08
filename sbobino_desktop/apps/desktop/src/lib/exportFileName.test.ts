import { describe, expect, it } from "vitest";

import { buildExportFileName } from "./exportFileName";

describe("buildExportFileName", () => {
  it("removes cross-platform forbidden characters and control bytes", () => {
    expect(buildExportFileName('  Team: Q3 / "Review"\u0000  ', ".PDF")).toBe(
      "Team_Q3_Review.pdf",
    );
  });

  it("protects Windows reserved basenames", () => {
    expect(buildExportFileName("CON", "txt")).toBe("_CON.txt");
    expect(buildExportFileName("LPT9.notes", "md")).toBe("_LPT9.notes.md");
  });

  it("falls back for an empty basename and caps the complete filename", () => {
    expect(buildExportFileName(" ... ", "csv")).toBe("transcript.csv");
    expect(
      new TextEncoder().encode(buildExportFileName("è".repeat(400), "json")).length,
    ).toBeLessThanOrEqual(180);
  });
});
