import * as React from "react";
import { Bot, Terminal } from "lucide-react";

import type {
  SlashCommandGroup,
  SlashCommandSuggestion,
} from "@/features/messages/lib/slashCommandAutocomplete";
import { cn } from "@/shared/lib/cn";
import {
  POPOVER_CUSTOM_ENTER_MOTION_CLASS,
  POPOVER_SHADOW_STYLE,
  POPOVER_SURFACE_CLASS,
} from "@/shared/ui/popoverSurface";

type SlashCommandAutocompleteProps = {
  groups: readonly SlashCommandGroup[];
  onSelect: (suggestion: SlashCommandSuggestion) => void;
  selectedIndex: number;
};

export const SlashCommandAutocomplete = React.memo(
  function SlashCommandAutocomplete({
    groups,
    onSelect,
    selectedIndex,
  }: SlashCommandAutocompleteProps) {
    const listRef = React.useRef<HTMLDivElement>(null);

    React.useEffect(() => {
      listRef.current
        ?.querySelector<HTMLElement>(`[data-command-index="${selectedIndex}"]`)
        ?.scrollIntoView({ block: "nearest" });
    }, [selectedIndex]);

    if (groups.length === 0) return null;

    let commandIndex = -1;
    return (
      <div className="absolute bottom-full left-0 right-0 z-50 mb-1 px-3 sm:px-4">
        <div
          className={cn(
            "max-h-64 overflow-y-auto rounded-xl p-1",
            POPOVER_CUSTOM_ENTER_MOTION_CLASS,
            "origin-bottom slide-in-from-bottom-1",
            POPOVER_SURFACE_CLASS,
          )}
          data-testid="slash-command-autocomplete"
          ref={listRef}
          style={POPOVER_SHADOW_STYLE}
        >
          {groups.map((group) => (
            <div key={group.agentPubkey}>
              <div className="flex items-center gap-1.5 px-3 pb-1 pt-2 text-2xs font-medium text-muted-foreground first:pt-1">
                <Bot aria-hidden="true" className="h-3 w-3" />
                <span className="truncate">{group.agentDisplayName}</span>
              </div>
              {group.commands.map((command) => {
                commandIndex += 1;
                const index = commandIndex;
                return (
                  <button
                    className={cn(
                      "flex w-full cursor-pointer items-start gap-2 rounded-lg px-3 py-1.5 text-left text-sm",
                      index === selectedIndex
                        ? "bg-accent text-accent-foreground"
                        : "text-popover-foreground hover:bg-accent/50",
                    )}
                    data-command-index={index}
                    data-testid={`slash-command-suggestion-${group.agentPubkey}-${command.name}`}
                    key={`${group.agentPubkey}:${command.name}`}
                    onMouseDown={(event) => {
                      event.preventDefault();
                      onSelect(command);
                    }}
                    tabIndex={-1}
                    type="button"
                  >
                    <Terminal
                      aria-hidden="true"
                      className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground"
                    />
                    <span className="min-w-0 flex-1">
                      <span className="block font-medium">/{command.name}</span>
                      {command.description ? (
                        <span className="block truncate text-xs text-muted-foreground">
                          {command.description}
                        </span>
                      ) : null}
                    </span>
                  </button>
                );
              })}
            </div>
          ))}
        </div>
      </div>
    );
  },
);
