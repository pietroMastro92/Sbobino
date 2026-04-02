import { Bot, Check, ChevronDown, Copy, MessageSquareText, Sparkles } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";

import type { ChatMessageViewModel } from "./chatTypes";

type ChatConversationProps = {
  messages: ChatMessageViewModel[];
  copiedMessageId: string | null;
  emptyTitle: string;
  emptyDescription: string;
  scrollToLatestLabel: string;
  pendingLabel: string;
  pendingDescription: string;
  promptOriginLabel: string;
  emotionOriginLabel: string;
  copyLabel: string;
  copiedLabel: string;
  onCopyMessage: (messageId: string) => void;
};

const BOTTOM_THRESHOLD_PX = 48;

export function ChatConversation({
  messages,
  copiedMessageId,
  emptyTitle,
  emptyDescription,
  scrollToLatestLabel,
  pendingLabel,
  pendingDescription,
  promptOriginLabel,
  emotionOriginLabel,
  copyLabel,
  copiedLabel,
  onCopyMessage,
}: ChatConversationProps): JSX.Element {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const bottomRef = useRef<HTMLDivElement | null>(null);
  const [stickToBottom, setStickToBottom] = useState(true);
  const [showScrollButton, setShowScrollButton] = useState(false);

  const updateScrollAffordance = useCallback(() => {
    const node = scrollRef.current;
    if (!node) {
      return;
    }

    const distanceFromBottom = node.scrollHeight - node.scrollTop - node.clientHeight;
    const isNearBottom = distanceFromBottom <= BOTTOM_THRESHOLD_PX;
    setStickToBottom(isNearBottom);
    setShowScrollButton(!isNearBottom && node.scrollHeight > node.clientHeight + 16);
  }, []);

  useEffect(() => {
    updateScrollAffordance();
  }, [messages, updateScrollAffordance]);

  useEffect(() => {
    if (!stickToBottom) {
      return;
    }

    const frame = window.requestAnimationFrame(() => {
      bottomRef.current?.scrollIntoView({ block: "end" });
    });

    return () => {
      window.cancelAnimationFrame(frame);
    };
  }, [messages, stickToBottom]);

  function resolveOriginLabel(message: ChatMessageViewModel): string | null {
    if (message.origin === "prompt") {
      return promptOriginLabel;
    }

    if (message.origin === "emotion") {
      return emotionOriginLabel;
    }

    return null;
  }

  return (
    <div className="chat-conversation-shell">
      <div
        ref={scrollRef}
        className="chat-conversation-thread"
        onScroll={updateScrollAffordance}
      >
        {messages.length === 0 ? (
          <div className="chat-empty-state">
            <div className="chat-empty-state-icon">
              <MessageSquareText size={28} />
            </div>
            <h2>{emptyTitle}</h2>
            <p>{emptyDescription}</p>
          </div>
        ) : (
          <div className="chat-conversation-stack">
            {messages.map((message) => {
              const isCopied = copiedMessageId === message.id;
              const originLabel = message.role === "user" ? resolveOriginLabel(message) : null;
              const isPending = message.status === "pending";

              return (
                <article
                  key={message.id}
                  className={`chat-message chat-message--${message.role} chat-message--${message.status}`}
                >
                  <div className="chat-message-accessory" aria-hidden="true">
                    {message.role === "assistant" ? (
                      <span className={`chat-avatar ${isPending ? "chat-avatar--pending" : ""}`}>
                        {isPending ? <Sparkles size={15} /> : <Bot size={15} />}
                      </span>
                    ) : null}
                  </div>

                  <div className="chat-message-main">
                    {originLabel ? (
                      <div className="chat-message-meta">
                        <span className={`chat-origin-chip chat-origin-chip--${message.origin}`}>
                          {originLabel}
                        </span>
                      </div>
                    ) : null}

                    <div className="chat-message-card">
                      {message.role === "assistant" && message.canCopy ? (
                        <button
                          type="button"
                          className={`chat-copy-button ${isCopied ? "copied" : ""}`}
                          onClick={() => onCopyMessage(message.id)}
                          title={copyLabel}
                          aria-label={copyLabel}
                        >
                          {isCopied ? <Check size={14} /> : <Copy size={14} />}
                          <span>{isCopied ? copiedLabel : copyLabel}</span>
                        </button>
                      ) : null}

                      {isPending ? (
                        <div className="chat-pending-state" aria-live="polite">
                          <div className="chat-pending-title-row">
                            <strong>{pendingLabel}</strong>
                            <span className="chat-pending-dots" aria-hidden="true">
                              <span />
                              <span />
                              <span />
                            </span>
                          </div>
                          <p>{pendingDescription}</p>
                        </div>
                      ) : (
                        <div className="chat-message-text">{message.text}</div>
                      )}
                    </div>
                  </div>
                </article>
              );
            })}
          </div>
        )}

        <div ref={bottomRef} aria-hidden="true" />
      </div>

      {showScrollButton ? (
        <button
          type="button"
          className="chat-scroll-button"
          onClick={() => {
            setStickToBottom(true);
            bottomRef.current?.scrollIntoView({ behavior: "smooth", block: "end" });
          }}
          title={scrollToLatestLabel}
          aria-label={scrollToLatestLabel}
        >
          <ChevronDown size={16} />
        </button>
      ) : null}
    </div>
  );
}
