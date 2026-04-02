import { ArrowUp } from "lucide-react";

import type { ChatPromptSuggestion } from "./chatTypes";

type ChatComposerProps = {
  inputValue: string;
  inputPlaceholder: string;
  suggestionsTitle: string;
  serviceSelectValue: string;
  serviceOptions: Array<{
    value: string;
    label: string;
    disabled?: boolean;
  }>;
  promptSuggestions: ChatPromptSuggestion[];
  submitLabel: string;
  disabled: boolean;
  submitDisabled: boolean;
  footerMessage?: string | null;
  onInputChange: (value: string) => void;
  onSelectService: (value: string) => void;
  onSelectPrompt: (suggestion: ChatPromptSuggestion) => void;
  onSubmit: () => void;
};

export function ChatComposer({
  inputValue,
  inputPlaceholder,
  suggestionsTitle,
  serviceSelectValue,
  serviceOptions,
  promptSuggestions,
  submitLabel,
  disabled,
  submitDisabled,
  footerMessage,
  onInputChange,
  onSelectService,
  onSelectPrompt,
  onSubmit,
}: ChatComposerProps): JSX.Element {
  return (
    <div className="chat-composer">
      {promptSuggestions.length > 0 ? (
        <div className="chat-suggestions">
          <div className="chat-suggestions-header">
            <span>{suggestionsTitle}</span>
          </div>
          <div className="chat-suggestions-list">
            {promptSuggestions.map((suggestion) => (
              <button
                key={suggestion.id}
                type="button"
                className="chat-suggestion-chip"
                onClick={() => onSelectPrompt(suggestion)}
                disabled={disabled}
                title={suggestion.body}
              >
                {suggestion.label}
              </button>
            ))}
          </div>
        </div>
      ) : null}

      <div className="chat-input-bar">
        <input
          placeholder={inputPlaceholder}
          value={inputValue}
          disabled={disabled}
          onChange={(event) => onInputChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              if (!submitDisabled) {
                onSubmit();
              }
            }
          }}
        />
        <select
          value={serviceSelectValue}
          disabled={disabled}
          onChange={(event) => onSelectService(event.target.value)}
        >
          {serviceOptions.map((option) => (
            <option key={option.value} value={option.value} disabled={option.disabled}>
              {option.label}
            </option>
          ))}
        </select>
        <button
          type="button"
          className="chat-submit-button"
          onClick={onSubmit}
          disabled={submitDisabled}
          aria-label={submitLabel}
          title={submitLabel}
        >
          <ArrowUp size={20} />
        </button>
      </div>

      {footerMessage ? <p className="chat-composer-footer muted">{footerMessage}</p> : null}
    </div>
  );
}
