import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { ChatConversation } from "./ChatConversation";
import type { ChatMessageViewModel } from "./chatTypes";

const messages: ChatMessageViewModel[] = [
  {
    id: "user-1",
    role: "user",
    text: "Summarize the main decision.",
    status: "complete",
    origin: "prompt",
    canCopy: false,
  },
  {
    id: "assistant-1",
    role: "assistant",
    text: "The team approved the launch for next week.",
    status: "complete",
    origin: "prompt",
    canCopy: true,
  },
  {
    id: "assistant-2",
    role: "assistant",
    text: "Thinking...",
    status: "pending",
    origin: "typed",
    canCopy: false,
  },
];

describe("ChatConversation", () => {
  it("renders the empty state when there are no messages", () => {
    render(
      <ChatConversation
        messages={[]}
        copiedMessageId={null}
        emptyTitle="AI Chat"
        emptyDescription="Ask questions on the current transcript."
        scrollToLatestLabel="Scroll to latest"
        pendingLabel="Thinking..."
        pendingDescription="Preparing a reply based on your transcript."
        promptOriginLabel="Prompt"
        emotionOriginLabel="Emotion insight"
        copyLabel="Copy"
        copiedLabel="Copied"
        onCopyMessage={() => {}}
      />,
    );

    expect(screen.getByText("AI Chat")).toBeInTheDocument();
    expect(screen.getByText("Ask questions on the current transcript.")).toBeInTheDocument();
  });

  it("renders origin chips, pending state, and copy actions", () => {
    const onCopyMessage = vi.fn();

    render(
      <ChatConversation
        messages={messages}
        copiedMessageId={"assistant-1"}
        emptyTitle="AI Chat"
        emptyDescription="Ask questions on the current transcript."
        scrollToLatestLabel="Scroll to latest"
        pendingLabel="Thinking..."
        pendingDescription="Preparing a reply based on your transcript."
        promptOriginLabel="Prompt"
        emotionOriginLabel="Emotion insight"
        copyLabel="Copy"
        copiedLabel="Copied"
        onCopyMessage={onCopyMessage}
      />,
    );

    expect(screen.getByText("Prompt")).toBeInTheDocument();
    expect(screen.getByText("Preparing a reply based on your transcript.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Copy" }));
    expect(onCopyMessage).toHaveBeenCalledWith("assistant-1");
    expect(screen.getByText("Copied")).toBeInTheDocument();
  });
});
