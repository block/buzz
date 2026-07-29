import * as React from "react";

import { useAuthorColorResolver } from "@/features/dev-mode/lib/authorColors";
import { cn } from "@/shared/lib/cn";
import { useComposerAutoGrow } from "@/features/dev-mode/lib/useComposerAutoGrow";
import {
  devComposerModeLabel,
  type DevComposerMode,
} from "@/features/dev-mode/lib/useDevComposerModes";
import { DevComposerResizeHandle } from "@/features/dev-mode/ui/DevComposerResizeHandle";

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
  /** Click on the box while inactive — the user wants to type again. */
  onReactivate?: () => void;
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
  onReactivate,
}: DevPromptComposerProps) {
  const { textareaRef, dragging, resizeHandleProps } = useComposerAutoGrow(
    value,
    "buzz.devMode.composerHeight",
  );
  const resolveColor = useAuthorColorResolver();
  // The pill (and caret) wear the same color the agent's name has in chat.
  const agentColor =
    mode.kind === "agent" ? resolveColor(mode.target.pubkey) : null;

  // biome-ignore lint/correctness/useExhaustiveDependencies: focusSignal is an intentional focus-pull trigger
  React.useEffect(() => {
    if (active) {
      textareaRef.current?.focus();
    } else {
      // Inactive means something else owns the keyboard (card selection,
      // side chat, palette) — the caret must leave the box.
      textareaRef.current?.blur();
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

  return (
    <div
      className={cn(
        "bg-background/80 font-mono transition-opacity",
        !active && "opacity-55",
      )}
    >
      <DevComposerResizeHandle
        dragging={dragging}
        testId="dev-mode-composer-resize"
        {...resizeHandleProps}
      />
      <div className="flex items-start gap-2 px-4 pt-2">
        <span
          aria-hidden
          className={cn(
            "select-none pt-[3px] text-sm",
            !agentColor && "text-muted-foreground",
          )}
          style={agentColor ? { color: agentColor } : undefined}
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
          onMouseDown={() => {
            if (!active) onReactivate?.();
          }}
          placeholder={placeholder}
          readOnly={!active}
          rows={1}
          spellCheck={false}
          value={value}
        />
      </div>
      <div className="mt-2 flex items-center justify-between pr-4 pb-3 pl-10 text-xs text-muted-foreground">
        <span
          className={cn(
            "rounded-none border px-1.5 py-0.5 font-medium",
            !agentColor && "border-border text-muted-foreground",
          )}
          data-testid="dev-mode-pill"
          style={
            agentColor
              ? { color: agentColor, borderColor: `${agentColor}80` }
              : undefined
          }
        >
          {busy ? "working…" : devComposerModeLabel(mode)}
        </span>
        <span className="select-none">{hint}</span>
      </div>
    </div>
  );
}
