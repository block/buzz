import * as React from "react";

import { cn } from "@/shared/lib/cn";
import {
  devComposerModeLabel,
  type DevComposerMode,
} from "@/features/dev-mode/lib/useDevComposerModes";

type DevPromptComposerProps = {
  value: string;
  mode: DevComposerMode;
  placeholder: string;
  busy: boolean;
  onChange: (value: string) => void;
  onSubmit: () => void;
  /** Tab / Shift+Tab. */
  onCycleMode: (direction: 1 | -1) => void;
  /** ArrowUp / ArrowDown while the input is empty. */
  onNavigateSessions: (direction: 1 | -1) => void;
  onEscape: () => void;
};

export function DevPromptComposer({
  value,
  mode,
  placeholder,
  busy,
  onChange,
  onSubmit,
  onCycleMode,
  onNavigateSessions,
  onEscape,
}: DevPromptComposerProps) {
  const textareaRef = React.useRef<HTMLTextAreaElement>(null);

  React.useEffect(() => {
    textareaRef.current?.focus();
  }, []);

  const handleKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Tab") {
      event.preventDefault();
      onCycleMode(event.shiftKey ? -1 : 1);
      return;
    }

    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      if (!busy) onSubmit();
      return;
    }

    if (event.key === "Escape") {
      event.preventDefault();
      onEscape();
      return;
    }

    if ((event.key === "ArrowUp" || event.key === "ArrowDown") && !value) {
      event.preventDefault();
      onNavigateSessions(event.key === "ArrowUp" ? -1 : 1);
    }
  };

  const rowCount = Math.min(value.split("\n").length, 8);

  return (
    <div className="border-t border-border/60 bg-background/80 px-4 py-3 font-mono">
      <div className="flex items-start gap-2">
        <span
          aria-hidden
          className={cn(
            "select-none pt-[3px] text-sm",
            mode.kind === "agent" ? "text-primary" : "text-muted-foreground",
          )}
        >
          ⏵
        </span>
        <textarea
          ref={textareaRef}
          className="min-h-6 w-full resize-none bg-transparent text-sm leading-6 outline-none placeholder:text-muted-foreground/60"
          data-testid="dev-mode-composer"
          onChange={(event) => onChange(event.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={placeholder}
          rows={rowCount}
          spellCheck={false}
          value={value}
        />
      </div>
      <div className="mt-2 flex items-center justify-between pl-6 text-xs text-muted-foreground">
        <span
          className={cn(
            "rounded-sm border px-1.5 py-0.5 font-medium",
            mode.kind === "agent"
              ? "border-primary/50 text-primary"
              : "border-border text-muted-foreground",
          )}
          data-testid="dev-mode-pill"
        >
          {busy ? "working…" : devComposerModeLabel(mode)}
        </span>
        <span className="select-none">
          tab: switch target · enter: send · ↑↓: sessions · esc: new session
        </span>
      </div>
    </div>
  );
}
