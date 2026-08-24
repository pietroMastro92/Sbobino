import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { TranscriptReviewQueue } from "./TranscriptReviewQueue";

const item = {
  id: "segment-0-words-1-1",
  segmentIndex: 0,
  originalText: "Sbobbino",
  contextText: "Welcome to Sbobbino today",
  confidence: 0.41,
  startSeconds: 1,
  endSeconds: 2,
  kind: "low_confidence" as const,
};

describe("TranscriptReviewQueue", () => {
  it("offers confirm, ignore, and remembered correction actions", async () => {
    const onReview = vi.fn().mockResolvedValue(undefined);
    render(
      <TranscriptReviewQueue
        items={[item]}
        decisions={{}}
        busy={false}
        onReview={onReview}
      />,
    );

    expect(screen.getByText("41% confidence")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Correct" }));
    fireEvent.change(screen.getByLabelText("Correction"), { target: { value: "Sbobino" } });
    fireEvent.click(screen.getByLabelText("Remember this correction"));
    fireEvent.click(screen.getByRole("button", { name: "Apply correction" }));

    expect(onReview).toHaveBeenCalledWith(item, "corrected", "Sbobino", true);
  });

  it("shows a completed state once all items have decisions", () => {
    render(
      <TranscriptReviewQueue
        items={[item]}
        decisions={{ [item.id]: "confirmed" }}
        busy={false}
        onReview={vi.fn()}
      />,
    );
    expect(screen.getByText("Review complete")).toBeInTheDocument();
  });
});
