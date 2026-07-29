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
  /** Keybinding help line, contextual to the shell state. */
  hint: string;
  busy: boolean;
  /** Whether this composer owns the keyboard (side chat may own it instead). */
  active: boolean;
  /** Increment to pull focus back here (e.g. after the palette closes). */
  focusSignal: number;
  onChange: (value: string) => void;
  onSubmit: () => void;
  /** Tab / Shift+Tab. */
  onCycleMode: (direction: 1 | -1) => void;
  /** ArrowUp / ArrowDown while the input is empty — channels or prompt cards. */
  onNavigate: (direction: 1 | -1) => void;
  /** ArrowLeft / ArrowRight while the input is empty — side-chat pane focus. */
  onSwitchPane: (pane: "main" | "thread") => void;
  /** `/` on an empty input. */
  onOpenPalette: () => void;
  onEscape: () => void;
};

export function DevPromptComposer({
  value,
  mode,
  placeholder,
  hint,
  busy,
  active,
  focusSignal,
  onChange,
  onSubmit,
  onCycleMode,
  onNavigate,
  onSwitchPane,
  onOpenPalette,
  onEscape,
}: DevPromptComposerProps) {
  const textareaRef = React.useRef<HTMLTextAreaElement>(null);

  // biome-ignore lint/correctness/useExhaustiveDependencies: focusSignal is an intentional focus-pull trigger
  React.useEffect(() => {
    if (active) {
      textareaRef.current?.focus();
    }
  }, [active, focusSignal]);

  const handleKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Tab") {
      event.preventDefault();
      onCycleMode(event.shiftKey ? -1 : 1);
      return;
    }

    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      // Empty-input Enter is also meaningful (opens the highlighted channel
      // or the selected card's side chat), so the shell decides; busy only
      // blocks actual sends there.
      onSubmit();
      return;
    }

    if (event.key === "Escape") {
      event.preventDefault();
      onEscape();
      return;
    }

    if (event.key === "/" && !value) {
      event.preventDefault();
      onOpenPalette();
      return;
    }

    if ((event.key === "ArrowUp" || event.key === "ArrowDown") && !value) {
      event.preventDefault();
      onNavigate(event.key === "ArrowUp" ? -1 : 1);
      return;
    }

    if ((event.key === "ArrowLeft" || event.key === "ArrowRight") && !value) {
      event.preventDefault();
      onSwitchPane(event.key === "ArrowLeft" ? "main" : "thread");
    }
  };

  const rowCount = Math.min(value.split("\n").length, 8);

  return (
    <div
      className={cn(
        "border-t border-border/60 bg-background/80 px-4 py-3 font-mono transition-opacity",
        !active && "opacity-55",
      )}
    >
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
          onFocus={() => onSwitchPane("main")}
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
            "rounded-none border px-1.5 py-0.5 font-medium",
            mode.kind === "agent"
              ? "border-primary/50 text-primary"
              : "border-border text-muted-foreground",
          )}
          data-testid="dev-mode-pill"
        >
          {busy ? "working…" : devComposerModeLabel(mode)}
        </span>
        <span className="select-none">{hint}</span>
      </div>
    </div>
  );
}
