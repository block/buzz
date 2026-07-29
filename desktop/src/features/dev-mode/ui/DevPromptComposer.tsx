import * as React from "react";

import { useAuthorColorResolver } from "@/features/dev-mode/lib/authorColors";
import { cn } from "@/shared/lib/cn";
import type { MentionRecord } from "@/features/dev-mode/lib/mentionRecords";
import { useChannelRefAutocomplete } from "@/features/dev-mode/lib/useChannelRefAutocomplete";
import { useComposerAutoGrow } from "@/features/dev-mode/lib/useComposerAutoGrow";
import type { DevComposerMode } from "@/features/dev-mode/lib/useDevComposerModes";
import { useMentionAutocomplete } from "@/features/dev-mode/lib/useMentionAutocomplete";
import { DevChannelSuggestions } from "@/features/dev-mode/ui/DevChannelSuggestions";
import { DevMentionSuggestions } from "@/features/dev-mode/ui/DevMentionSuggestions";
import { DevComposerModeLine } from "@/features/dev-mode/ui/DevComposerModeLine";
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
  /**
   * When set the composer is drafting something other than a channel message
   * (e.g. a new tab) — renders a highlighted banner so the redirect is
   * unmissable, not just placeholder text.
   */
  draftLabel?: string | null;
  /** Increment to pull focus back here (e.g. after the palette closes). */
  focusSignal: number;
  /** Channel whose members rank first in `@` mention suggestions. */
  channelId: string | null;
  /** Excluded from mention suggestions. */
  selfPubkey: string | null;
  onChange: (value: string) => void;
  /** Mentions are the `@Name`s still present in the submitted text. */
  onSubmit: (mentions: MentionRecord[]) => void;
  /** Tab / Shift+Tab. */
  onCycleMode: (direction: 1 | -1) => void;
  /** ArrowUp / ArrowDown while the input is empty — channels or prompt cards. */
  onNavigate: (direction: 1 | -1) => void;
  /** ⌥ArrowUp / ⌥ArrowDown — switch channels without leaving the box. */
  onStepChannel: (direction: 1 | -1) => void;
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
  draftLabel = null,
  focusSignal,
  channelId,
  selfPubkey,
  onChange,
  onSubmit,
  onCycleMode,
  onNavigate,
  onStepChannel,
  onSwitchPane,
  onOpenPalette,
  onEscape,
  onReactivate,
}: DevPromptComposerProps) {
  const { textareaRef, dragging, resizeHandleProps } = useComposerAutoGrow(
    value,
    "buzz.devMode.composerHeight",
  );
  const autocomplete = useChannelRefAutocomplete({
    value,
    onChange,
    textareaRef,
  });
  const mentionAutocomplete = useMentionAutocomplete({
    channelId,
    selfPubkey,
    value,
    onChange,
    textareaRef,
  });
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
    // Open `#channel` / `@user` suggestions own Tab/Enter/arrows/Escape.
    if (autocomplete.handleKeyDown(event)) {
      return;
    }
    if (mentionAutocomplete.handleKeyDown(event)) {
      return;
    }

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
      onSubmit(mentionAutocomplete.extract(value));
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

    if (event.key === "ArrowUp" || event.key === "ArrowDown") {
      // ⌥↑/⌥↓ steps through the channel list, draft or not, without the
      // caret ever leaving the box.
      if (event.altKey) {
        event.preventDefault();
        onStepChannel(event.key === "ArrowUp" ? -1 : 1);
        return;
      }
      if (!value) {
        event.preventDefault();
        onNavigate(event.key === "ArrowUp" ? -1 : 1);
        return;
      }
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
        draftLabel && "bg-primary/5",
      )}
    >
      <DevComposerResizeHandle
        dragging={dragging}
        testId="dev-mode-composer-resize"
        {...resizeHandleProps}
      />
      {draftLabel ? (
        <div
          className="flex items-baseline gap-2 border-y border-primary/40 bg-primary/10 px-4 py-1 text-xs text-primary"
          data-testid="dev-mode-draft-banner"
        >
          <span className="font-semibold">{draftLabel}</span>
          <span className="text-primary/60">esc cancels</span>
        </div>
      ) : null}
      {/* Mirror the composer row's prefix so the mode line lines up exactly
          with the textarea's left edge. */}
      <div className="flex items-start gap-2 px-4 pt-2">
        <span aria-hidden className="invisible select-none text-sm">
          ⏵
        </span>
        <DevComposerModeLine agentColor={agentColor} busy={busy} mode={mode} />
      </div>
      <div className="relative flex items-start gap-2 px-4 pt-1">
        {active && autocomplete.open ? (
          <DevChannelSuggestions
            onAccept={autocomplete.accept}
            selectedIndex={autocomplete.selectedIndex}
            suggestions={autocomplete.suggestions}
          />
        ) : active && mentionAutocomplete.open ? (
          <DevMentionSuggestions
            onAccept={mentionAutocomplete.accept}
            selectedIndex={mentionAutocomplete.selectedIndex}
            suggestions={mentionAutocomplete.suggestions}
          />
        ) : null}
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
          onChange={(event) => {
            onChange(event.target.value);
            autocomplete.syncCursor(event.target);
            mentionAutocomplete.syncCursor(event.target);
          }}
          onFocus={() => onSwitchPane("main")}
          onKeyDown={handleKeyDown}
          onMouseDown={() => {
            if (!active) onReactivate?.();
          }}
          onSelect={(event) => {
            autocomplete.syncCursor(event.currentTarget);
            mentionAutocomplete.syncCursor(event.currentTarget);
          }}
          placeholder={placeholder}
          readOnly={!active}
          rows={1}
          spellCheck={false}
          value={value}
        />
      </div>
      <div className="mt-2 flex items-center justify-end pr-4 pb-3 pl-10 text-xs text-muted-foreground">
        <span className="select-none">{hint}</span>
      </div>
    </div>
  );
}
