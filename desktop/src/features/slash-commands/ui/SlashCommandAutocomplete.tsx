import * as React from "react";

import { cn } from "@/shared/lib/cn";
import {
  POPOVER_CUSTOM_ENTER_MOTION_CLASS,
  POPOVER_SHADOW_STYLE,
  POPOVER_SURFACE_CLASS,
} from "@/shared/ui/popoverSurface";
import type { SlashCommandDefinition } from "../lib/types";

type SlashCommandAutocompleteProps = {
  suggestions: readonly SlashCommandDefinition[];
  selectedIndex: number;
  /** Whether the owning composer owns document focus. */
  composerOwnsFocus: boolean;
  onSelect: (command: SlashCommandDefinition) => void;
  position?: "above" | "below";
};

export function slashCommandRowLabel(command: SlashCommandDefinition): string {
  return [`/${command.name}`, command.argumentHint, command.description]
    .filter(Boolean)
    .join(" ");
}

export const SlashCommandAutocomplete = React.memo(
  function SlashCommandAutocomplete({
    suggestions,
    selectedIndex,
    composerOwnsFocus,
    onSelect,
    position = "above",
  }: SlashCommandAutocompleteProps) {
    const listRef = React.useRef<HTMLDivElement>(null);

    React.useEffect(() => {
      const activeItem = listRef.current?.querySelector<HTMLElement>(
        `[data-slash-command-index="${selectedIndex}"]`,
      );
      activeItem?.scrollIntoView({ block: "nearest" });
    }, [selectedIndex]);

    if (!composerOwnsFocus || suggestions.length === 0) {
      return null;
    }

    return (
      <div
        className={cn(
          "absolute left-0 right-0 z-50 px-3 sm:px-4",
          position === "below" ? "top-full mt-1" : "bottom-full mb-1",
        )}
      >
        {/* biome-ignore lint/a11y/noStaticElementInteractions: pointer-only guard, no behavior of its own — an unprevented mousedown here (scrollbar, padding ring) blurs the editor, and the focus gate above would unmount the overlay mid-press. */}
        <div
          className={cn(
            "max-h-48 w-full max-w-2xl overflow-y-auto rounded-xl p-1",
            POPOVER_CUSTOM_ENTER_MOTION_CLASS,
            position === "below"
              ? "origin-top slide-in-from-top-1"
              : "origin-bottom slide-in-from-bottom-1",
            POPOVER_SURFACE_CLASS,
          )}
          data-testid="slash-command-autocomplete"
          onMouseDown={(event) => event.preventDefault()}
          ref={listRef}
          style={POPOVER_SHADOW_STYLE}
        >
          {suggestions.map((command, index) => (
            <button
              aria-label={slashCommandRowLabel(command)}
              aria-current={index === selectedIndex ? "true" : undefined}
              className={cn(
                "flex w-full cursor-pointer items-baseline gap-2 rounded-lg px-3 py-1.5 text-left text-sm",
                index === selectedIndex
                  ? "bg-accent text-accent-foreground"
                  : "text-popover-foreground hover:bg-accent/50",
              )}
              data-slash-command-index={index}
              data-testid={`slash-command-${command.name}`}
              key={command.name}
              onMouseDown={(event) => {
                event.preventDefault();
                onSelect(command);
              }}
              tabIndex={-1}
              type="button"
            >
              <span className="shrink-0 font-medium">/{command.name}</span>
              {command.argumentHint ? (
                <span
                  aria-hidden="true"
                  className={cn(
                    "shrink-0 text-xs",
                    index === selectedIndex
                      ? "text-accent-foreground/60"
                      : "text-muted-foreground",
                  )}
                >
                  {command.argumentHint}
                </span>
              ) : null}
              <span
                aria-hidden="true"
                className={cn(
                  "min-w-0 truncate text-xs",
                  index === selectedIndex
                    ? "text-accent-foreground/60"
                    : "text-muted-foreground",
                )}
              >
                {command.description}
              </span>
            </button>
          ))}
        </div>
      </div>
    );
  },
);
