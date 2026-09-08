import {
  IconArrowUp,
  IconLetterCase,
  IconMicrophone,
  IconPaperclip,
  IconPlayerStopFilled,
} from "@tabler/icons-react";
import { type FormEvent, useEffect, useRef, useState } from "react";

import { IconButton } from "@/shared/ui/IconButton";

import { EmojiPicker } from "./EmojiPicker";

/**
 * The first production Composer composition.
 *
 * It owns visual authoring and local submission feedback only. Draft scope and
 * the send transaction belong to its caller. Attachment, emoji, formatting,
 * and voice controls are intentionally visual seams in this pass: their real
 * behavior arrives one capability at a time, not as decorative placeholders.
 */
export function MessageComposer({
  disabled,
  draft,
  onDraftChange,
  onSend,
  placeholder = "Message the session",
  responseControl,
}: {
  disabled?: boolean;
  draft: string;
  onDraftChange: (value: string) => void;
  onSend: (content: string) => Promise<void>;
  placeholder?: string;
  responseControl?: {
    state: "responding" | "stopping";
    onStop?: () => void;
  };
}) {
  const [sending, setSending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const textarea = useRef<HTMLTextAreaElement>(null);
  const selection = useRef({ start: 0, end: 0 });

  useEffect(() => {
    textarea.current?.focus();
  }, []);

  async function submit(event: FormEvent) {
    event.preventDefault();
    const content = draft.trim();
    if (!content || sending || disabled) return;

    setSending(true);
    setError(null);
    try {
      await onSend(content);
      onDraftChange("");
    } catch (caught) {
      setError(
        caught instanceof Error ? caught.message : "Your message was not sent.",
      );
    } finally {
      setSending(false);
      textarea.current?.focus();
    }
  }

  function rememberSelection() {
    const element = textarea.current;
    if (!element) return;
    selection.current = {
      start: element.selectionStart,
      end: element.selectionEnd,
    };
  }

  function insertEmoji(emoji: string) {
    const { start, end } = selection.current;
    const next = `${draft.slice(0, start)}${emoji}${draft.slice(end)}`;
    onDraftChange(next);
    requestAnimationFrame(() => {
      const element = textarea.current;
      if (!element) return;
      const position = start + emoji.length;
      element.focus();
      element.setSelectionRange(position, position);
      selection.current = { start: position, end: position };
    });
  }

  const unavailable = Boolean(disabled || sending);
  const canSend = Boolean(draft.trim()) && !unavailable;
  const state = disabled
    ? "disabled"
    : sending
      ? "sending"
      : error
        ? "failed"
        : "ready";

  return (
    <form className="message-composer" data-state={state} onSubmit={submit}>
      <div className="message-composer-input">
        <textarea
          ref={textarea}
          value={draft}
          rows={1}
          disabled={disabled}
          aria-label={placeholder}
          aria-describedby={error ? "composer-send-error" : undefined}
          placeholder={placeholder}
          onChange={(event) => {
            selection.current = {
              start: event.currentTarget.selectionStart,
              end: event.currentTarget.selectionEnd,
            };
            setError(null);
            onDraftChange(event.target.value);
          }}
          onSelect={rememberSelection}
          onKeyUp={rememberSelection}
          onMouseUp={rememberSelection}
          onKeyDown={(event) => {
            if (
              event.key === "Enter" &&
              !event.shiftKey &&
              !event.nativeEvent.isComposing
            ) {
              event.preventDefault();
              event.currentTarget.form?.requestSubmit();
            }
          }}
        />
        {error ? (
          <p
            id="composer-send-error"
            className="message-composer-feedback text-body-sm text-red-12"
            role="alert"
          >
            Couldn’t send. Your message is still here — try again.
          </p>
        ) : sending ? (
          <p
            className="message-composer-feedback text-body-sm text-secondary"
            role="status"
          >
            Sending message…
          </p>
        ) : null}
      </div>
      <div className="message-composer-actions">
        <div className="message-composer-actions-start">
          <IconButton
            aria-label="Attach file"
            icon={<IconPaperclip size={16} stroke={2} aria-hidden="true" />}
            size="toolbar"
            variant="ghost"
          />
          <EmojiPicker
            disabled={unavailable}
            textarea={textarea}
            onInsert={insertEmoji}
          />
          <IconButton
            aria-label="Toggle formatting"
            icon={<IconLetterCase size={16} stroke={2} aria-hidden="true" />}
            size="toolbar"
            variant="ghost"
          />
        </div>
        <div className="message-composer-actions-end">
          <IconButton
            aria-label="Record voice note"
            icon={<IconMicrophone size={16} stroke={2} aria-hidden="true" />}
            size="toolbar"
            variant="ghost"
          />
          {responseControl ? (
            <IconButton
              aria-label={
                responseControl.state === "stopping"
                  ? "Stopping response"
                  : "Stop response"
              }
              aria-busy={responseControl.state === "stopping" || undefined}
              icon={<IconPlayerStopFilled size={14} aria-hidden="true" />}
              shape="round"
              size="toolbar"
              variant="tint"
              disabled={responseControl.state === "stopping"}
              onClick={responseControl.onStop}
            />
          ) : (
            <IconButton
              type="submit"
              aria-label={sending ? "Sending message" : "Send message"}
              icon={<IconArrowUp size={16} stroke={2} aria-hidden="true" />}
              shape="round"
              size="toolbar"
              variant="tint"
              disabled={!canSend}
            />
          )}
        </div>
      </div>
    </form>
  );
}
