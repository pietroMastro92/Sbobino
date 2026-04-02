import type { ChatMessageViewModel } from "./chatTypes";

export function findPreviousUserQuestion(
  messages: ChatMessageViewModel[],
  assistantIndex: number,
): string {
  for (let index = assistantIndex - 1; index >= 0; index -= 1) {
    if (messages[index]?.role === "user") {
      return messages[index].text;
    }
  }

  return "";
}

export function buildChatClipboardText(params: {
  messages: ChatMessageViewModel[];
  assistantIndex: number;
  questionLabel: string;
  answerLabel: string;
}): string {
  const assistantMessage = params.messages[params.assistantIndex];
  if (!assistantMessage || assistantMessage.role !== "assistant") {
    return "";
  }

  const question = findPreviousUserQuestion(params.messages, params.assistantIndex);
  if (!question) {
    return assistantMessage.text;
  }

  return `${params.questionLabel}:\n${question}\n\n${params.answerLabel}:\n${assistantMessage.text}`;
}
