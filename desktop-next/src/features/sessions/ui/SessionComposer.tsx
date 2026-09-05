import { IconArrowUp } from "@tabler/icons-react";
import { type FormEvent, useEffect, useRef, useState } from "react";

export function SessionComposer({
  disabled,
  initialValue = "",
  onSend,
  placeholder = "Message the session",
}: {
  disabled?: boolean;
  initialValue?: string;
  onSend: (content: string) => Promise<void>;
  placeholder?: string;
}) {
  const [value, setValue] = useState(initialValue);
  const [sending, setSending] = useState(false);
  const textarea = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    textarea.current?.focus();
  }, []);

  async function submit(event: FormEvent) {
    event.preventDefault();
    const content = value.trim();
    if (!content || sending || disabled) return;
    setSending(true);
    try {
      await onSend(content);
      setValue("");
    } finally {
      setSending(false);
      textarea.current?.focus();
    }
  }

  return (
    <form className="session-composer" onSubmit={submit}>
      <textarea
        ref={textarea}
        value={value}
        rows={1}
        aria-label={placeholder}
        placeholder={placeholder}
        onChange={(event) => setValue(event.target.value)}
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
      <button
        type="submit"
        className="send-button"
        aria-label={sending ? "Sending message" : "Send message"}
        disabled={!value.trim() || sending || disabled}
      >
        <IconArrowUp size={18} stroke={1.8} aria-hidden="true" />
      </button>
    </form>
  );
}
