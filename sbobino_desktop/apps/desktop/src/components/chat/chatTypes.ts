export type ChatMessageRole = "user" | "assistant";

export type ChatMessageStatus = "pending" | "complete" | "error";

export type ChatMessageOrigin = "typed" | "prompt" | "emotion";

export type ChatMessageViewModel = {
  id: string;
  role: ChatMessageRole;
  text: string;
  status: ChatMessageStatus;
  origin: ChatMessageOrigin;
  canCopy: boolean;
};

export type ChatPromptSuggestion = {
  id: string;
  label: string;
  body: string;
};
