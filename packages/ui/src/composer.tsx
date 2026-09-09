import { useId, useRef, useState, type ReactNode } from "react";
import { ArrowUp, Mic, Plus, Square } from "lucide-react";
import { IconButton } from "./button";
import { cx } from "./classes";

/** Slots receive the same availability state as the input and submit action. */
export interface ComposerSlotState {
  disabled: boolean;
}
/** A controlled draft. The host owns attachments, generation, and clearing after success. */
export interface AIComposerProps {
  value: string;
  onValueChange: (value: string) => void;
  /** Resolves when the host accepts the prompt; rejects to preserve the draft and show retry feedback. */
  onSubmit: (text: string) => void | Promise<void>;
  label?: string;
  placeholder?: string;
  disabled?: boolean;
  maxLength?: number;
  /** Context chips, filenames, or annotations; render removal controls disabled when indicated. */
  context?: (state: ComposerSlotState) => ReactNode;
  /** Model, reasoning, or permission controls, composed from the shared primitives. */
  controls?: (state: ComposerSlotState) => ReactNode;
  onAttach?: () => void;
  /** The host owns microphone permission, capture, and transcription. */
  voice?: { label: string; active?: boolean; onToggle: () => void };
  /** Presence means a response is running and guarantees an explicit stop action. */
  generation?: { onStop: () => void };
  className?: string;
}
/** Multiline AI input with one submission path, IME-safe keyboard handling, and retained failed drafts. */
export function AIComposer({
  value,
  onValueChange,
  onSubmit,
  label = "Message the assistant",
  placeholder = "Ask anything…",
  disabled = false,
  maxLength = 16000,
  context,
  controls,
  onAttach,
  voice,
  generation,
  className,
}: AIComposerProps) {
  const helpId = useId();
  const errorId = useId();
  const composing = useRef(false);
  const submitting = useRef(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const locked = disabled || pending || !!generation;
  const canSend =
    !locked && value.trim().length > 0 && value.length <= maxLength;
  return (
    <form
      aria-label="AI composer"
      className={cx("bui-composer", className)}
      onSubmit={async (event) => {
        event.preventDefault();
        if (!canSend || submitting.current || composing.current) return;
        submitting.current = true;
        setPending(true);
        setError(null);
        try {
          await onSubmit(value.trim());
        } catch {
          setError("Couldn’t send. Your draft is still here. Try again.");
        } finally {
          submitting.current = false;
          setPending(false);
        }
      }}
    >
      {context && (
        <div className="bui-composer-context">
          {context({ disabled: locked })}
        </div>
      )}
      <textarea
        aria-label={label}
        aria-describedby={`${helpId}${error ? ` ${errorId}` : ""}`}
        aria-invalid={error ? true : undefined}
        className="bui-composer-input"
        value={value}
        onChange={(event) => {
          onValueChange(event.target.value);
          setError(null);
        }}
        placeholder={placeholder}
        rows={3}
        maxLength={maxLength}
        disabled={disabled}
        readOnly={pending || !!generation}
        onCompositionStart={() => {
          composing.current = true;
        }}
        onCompositionEnd={() => {
          composing.current = false;
        }}
        onKeyDown={(event) => {
          if (
            event.key !== "Enter" ||
            event.shiftKey ||
            event.altKey ||
            event.ctrlKey ||
            event.metaKey ||
            event.nativeEvent.isComposing ||
            event.nativeEvent.keyCode === 229 ||
            composing.current
          )
            return;
          event.preventDefault();
          event.currentTarget.form?.requestSubmit();
        }}
      />
      <div className="bui-composer-footer">
        {onAttach && (
          <IconButton
            aria-label="Add attachment"
            size="sm"
            variant="ghost"
            disabled={locked}
            onClick={onAttach}
          >
            <Plus aria-hidden="true" />
          </IconButton>
        )}
        <div className="bui-composer-controls">
          {controls?.({ disabled: locked })}
        </div>
        {voice && (
          <IconButton
            aria-label={voice.label}
            size="sm"
            aria-pressed={voice.active ?? false}
            variant="ghost"
            disabled={locked}
            onClick={voice.onToggle}
          >
            <Mic aria-hidden="true" />
          </IconButton>
        )}
        {generation ? (
          <IconButton aria-label="Stop response" onClick={generation.onStop}>
            <Square aria-hidden="true" />
          </IconButton>
        ) : (
          <IconButton
            type="submit"
            aria-label={pending ? "Sending message" : "Send message"}
            disabled={!canSend}
            loading={pending}
          >
            <ArrowUp aria-hidden="true" />
          </IconButton>
        )}
      </div>
      <p id={helpId} className="bui-sr-only">
        Enter to send. Shift+Enter for a new line.
      </p>
      {error && (
        <p id={errorId} role="alert" className="bui-field-error">
          {error}
        </p>
      )}
    </form>
  );
}
