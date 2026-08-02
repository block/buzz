import * as React from "react";
import { Command } from "lucide-react";

import type { AgentAvailableCommand } from "@/features/agents/lib/agentAvailableCommands";
import { cn } from "@/shared/lib/cn";
import {
  POPOVER_CUSTOM_ENTER_MOTION_CLASS,
  POPOVER_SHADOW_STYLE,
  POPOVER_SURFACE_CLASS,
} from "@/shared/ui/popoverSurface";

type SlashCommandAutocompleteProps = {
  suggestions: AgentAvailableCommand[];
  selectedIndex: number;
  onSelect: (suggestion: AgentAvailableCommand) => void;
};

export const SlashCommandAutocomplete = React.memo(
  function SlashCommandAutocomplete({
    suggestions,
    selectedIndex,
    onSelect,
  }: SlashCommandAutocompleteProps) {
    const scrollSelectedIntoView = React.useCallback(
      (element: HTMLButtonElement | null) => {
        element?.scrollIntoView({ block: "nearest" });
      },
      [],
    );

    if (suggestions.length === 0) return null;

    return (
      <div className="absolute bottom-full left-0 right-0 z-50 mb-1 px-3 sm:px-4">
        <div
          className={cn(
            "origin-bottom rounded-xl p-1",
            "slide-in-from-bottom-1",
            POPOVER_CUSTOM_ENTER_MOTION_CLASS,
            POPOVER_SURFACE_CLASS,
          )}
          data-testid="slash-command-autocomplete"
          style={POPOVER_SHADOW_STYLE}
        >
          <div className="flex items-center gap-2 border-b border-border/50 px-3 py-2 text-xs font-medium text-muted-foreground">
            <Command className="size-3.5" aria-hidden="true" />
            Agent commands
          </div>
          <div className="max-h-60 overflow-y-auto py-1">
            {suggestions.map((suggestion, index) => (
              <button
                className={cn(
                  "flex w-full cursor-pointer items-start gap-3 rounded-lg px-3 py-2 text-left",
                  index === selectedIndex
                    ? "bg-accent text-accent-foreground"
                    : "text-popover-foreground hover:bg-accent/50",
                )}
                data-testid={`slash-command-suggestion-${suggestion.name}`}
                key={suggestion.name}
                onMouseDown={(event) => {
                  event.preventDefault();
                  onSelect(suggestion);
                }}
                ref={
                  index === selectedIndex ? scrollSelectedIntoView : undefined
                }
                tabIndex={-1}
                type="button"
              >
                <span className="min-w-36 shrink-0 font-mono text-sm font-medium">
                  /{suggestion.name}
                </span>
                <span className="min-w-0 flex-1">
                  {suggestion.description ? (
                    <span className="block text-xs leading-relaxed text-muted-foreground">
                      {suggestion.description}
                    </span>
                  ) : null}
                  {suggestion.inputHint ? (
                    <span className="mt-0.5 block font-mono text-2xs text-muted-foreground/75">
                      {suggestion.inputHint}
                    </span>
                  ) : null}
                </span>
              </button>
            ))}
          </div>
          <div className="flex items-center gap-3 border-t border-border/50 px-3 py-1.5 text-2xs text-muted-foreground/75">
            <span>↑↓ Navigate</span>
            <span>↵ Select</span>
            <span>esc Close</span>
          </div>
        </div>
      </div>
    );
  },
);
