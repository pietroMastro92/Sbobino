import { describe, expect, it } from "vitest";

import type { ChatMessageViewModel } from "./chatTypes";
import { buildChatClipboardText, findPreviousUserQuestion } from "./chatUtils";

const messages: ChatMessageViewModel[] = [
  {
    id: "user-1",
    role: "user",
    text: "What changed in the meeting?",
    status: "complete",
    origin: "typed",
    canCopy: false,
  },
  {
    id: "assistant-1",
    role: "assistant",
    text: "Three decisions were made.",
    status: "complete",
    origin: "typed",
    canCopy: true,
  },
  {
    id: "user-2",
    role: "user",
    text: "Use the emotion panel insight to explain the tension.",
    status: "complete",
    origin: "emotion",
    canCopy: false,
  },
  {
    id: "assistant-2",
    role: "assistant",
    text: "The tension peaked during the roadmap disagreement.",
    status: "complete",
    origin: "emotion",
    canCopy: true,
  },
];

describe("chatUtils", () => {
  it("finds the previous user question for an assistant reply", () => {
    expect(findPreviousUserQuestion(messages, 3)).toBe(
      "Use the emotion panel insight to explain the tension.",
    );
  });

  it("builds clipboard text with question and answer labels", () => {
    expect(buildChatClipboardText({
      messages,
      assistantIndex: 3,
      questionLabel: "Question",
      answerLabel: "Answer",
    })).toBe(
      "Question:\nUse the emotion panel insight to explain the tension.\n\nAnswer:\nThe tension peaked during the roadmap disagreement.",
    );
  });
});
