import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { parseSummaryMarkdown, SummaryMarkdown } from "./SummaryMarkdown";

describe("SummaryMarkdown", () => {
  it("normalizes generated markdown headings and lists into readable blocks", () => {
    render(
      <SummaryMarkdown
        markdown={"## Summary of Transcript ##\n\n- First point\n- Second point"}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "Summary of Transcript", level: 2 }),
    ).toBeInTheDocument();
    expect(screen.getByText("First point")).toBeInTheDocument();
    expect(screen.queryByDisplayValue(/Summary of Transcript/)).toBeNull();
  });

  it("keeps paragraph text as plain React text", () => {
    const blocks = parseSummaryMarkdown("Plain paragraph\ncontinued");

    expect(blocks).toEqual([
      { kind: "paragraph", text: "Plain paragraph continued" },
    ]);
  });
});
