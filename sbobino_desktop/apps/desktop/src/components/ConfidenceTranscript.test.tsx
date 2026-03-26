import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ConfidenceTranscript } from "./ConfidenceTranscript";

describe("ConfidenceTranscript", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders the confidence tooltip in a portal on hover", () => {
    vi.useFakeTimers();

    render(
      <ConfidenceTranscript
        fontSize={18}
        document={{
          confidenceWordCount: 1,
          fragments: [
            {
              text: "ciao",
              confidence: 0.91,
              color: "rgb(78, 178, 101)",
              colorIndex: 6,
              tooltip: "91% confidence",
            },
          ],
        }}
      />,
    );

    fireEvent.mouseEnter(screen.getByText("ciao"));
    expect(screen.getByText("91% confidence")).toBeTruthy();

    fireEvent.mouseLeave(screen.getByText("ciao"));
    expect(screen.getByText("91% confidence")).toBeTruthy();

    act(() => {
      vi.advanceTimersByTime(500);
    });
    expect(screen.queryByText("91% confidence")).toBeNull();
  });
});
