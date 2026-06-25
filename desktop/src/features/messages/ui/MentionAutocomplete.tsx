import * as React from "react";
import { Bot } from "lucide-react";

import { Badge } from "@/shared/ui/badge";
import { cn } from "@/shared/lib/cn";
import {
  POPOVER_CUSTOM_ENTER_MOTION_CLASS,
  POPOVER_SHADOW_STYLE,
  POPOVER_SURFACE_CLASS,
} from "@/shared/ui/popoverSurface";
import { UserAvatar } from "@/shared/ui/UserAvatar";

export type MentionSuggestion = {
  pubkey?: string;
  personaId?: string;
  kind?: "identity" | "persona";
  displayName: string;
  avatarUrl?: string | null;
  isAgent?: boolean;
  notInChannel?: boolean;
  ownerLabel?: string | null;
  role?: string | null;
};

type MentionAutocompleteProps = {
  suggestions: MentionSuggestion[];
  selectedIndex: number;
  onSelect: (suggestion: MentionSuggestion) => void;
  position?: "above" | "below";
};

export const MentionAutocomplete = React.memo(function MentionAutocomplete({
  suggestions,
  selectedIndex,
  onSelect,
  position = "above",
}: MentionAutocompleteProps) {
  const listRef = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    const activeItem = listRef.current?.children[selectedIndex] as
      | HTMLElement
      | undefined;
    activeItem?.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  if (suggestions.length === 0) {
    return null;
  }

  return (
    <div
      className={cn(
        "absolute left-0 right-0 z-50 px-3 sm:px-4",
        position === "below" ? "top-full mt-1" : "bottom-full mb-1",
      )}
    >
      <div
        className={cn(
          "max-h-48 overflow-y-auto rounded-xl p-1",
          POPOVER_CUSTOM_ENTER_MOTION_CLASS,
          position === "below"
            ? "origin-top slide-in-from-top-1"
            : "origin-bottom slide-in-from-bottom-1",
          POPOVER_SURFACE_CLASS,
        )}
        data-testid="mention-autocomplete"
        ref={listRef}
        style={POPOVER_SHADOW_STYLE}
      >
        {suggestions.map((suggestion, index) => {
          const suggestionKey =
            suggestion.pubkey ??
            (suggestion.personaId ? `persona-${suggestion.personaId}` : null) ??
            suggestion.displayName;
          const agentLabel = "agent";

          return (
            <button
              className={cn(
                "flex w-full cursor-pointer items-center gap-2 rounded-lg px-3 py-1.5 text-left text-sm",
                index === selectedIndex
                  ? "bg-accent text-accent-foreground"
                  : "text-popover-foreground hover:bg-accent/50",
              )}
              data-testid={`mention-suggestion-${suggestionKey}`}
              key={suggestionKey}
              onMouseDown={(event) => {
                event.preventDefault();
                onSelect(suggestion);
              }}
              tabIndex={-1}
              type="button"
            >
              <UserAvatar
                avatarUrl={suggestion.avatarUrl ?? null}
                displayName={suggestion.displayName}
                size="xs"
                testId="mention-suggestion-avatar"
              />
              <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span
                  className="min-w-0 break-words font-medium leading-snug"
                  title={suggestion.displayName}
                >
                  {suggestion.displayName}
                </span>
                {suggestion.isAgent ||
                suggestion.role ||
                suggestion.ownerLabel ||
                suggestion.notInChannel ? (
                  <span
                    className={cn(
                      "flex min-w-0 items-center gap-1.5 text-2xs leading-none",
                      index === selectedIndex
                        ? "text-accent-foreground/60"
                        : "text-muted-foreground",
                    )}
                  >
                    {suggestion.isAgent ? (
                      <span className="inline-flex shrink-0 items-center gap-1">
                        <Bot
                          aria-hidden="true"
                          className="h-3.5 w-3.5"
                          data-testid="mention-agent-icon"
                        />
                        {agentLabel}
                      </span>
                    ) : suggestion.role ? (
                      <Badge
                        className="max-w-24 shrink-0 truncate"
                        variant="secondary"
                      >
                        {suggestion.role}
                      </Badge>
                    ) : null}
                    {suggestion.ownerLabel || suggestion.notInChannel ? (
                      <span
                        className="min-w-0 truncate"
                        title={
                          suggestion.ownerLabel
                            ? `owned by ${suggestion.ownerLabel}${suggestion.notInChannel ? " · not in channel" : ""}`
                            : "not in channel"
                        }
                      >
                        {suggestion.ownerLabel
                          ? `owned by ${suggestion.ownerLabel}${suggestion.notInChannel ? " · not in channel" : ""}`
                          : "not in channel"}
                      </span>
                    ) : null}
                  </span>
                ) : null}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
});
