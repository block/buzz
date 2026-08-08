import * as React from "react";

import type { EmojiSuggestion } from "@/features/messages/lib/useEmojiAutocomplete";
import { cn } from "@/shared/lib/cn";
import {
  type ListVirtualizer,
  VirtualizedList,
} from "@/shared/ui/VirtualizedList";
import {
  POPOVER_CUSTOM_ENTER_MOTION_CLASS,
  POPOVER_SHADOW_STYLE,
  POPOVER_SURFACE_CLASS,
} from "@/shared/ui/popoverSurface";

type EmojiAutocompleteProps = {
  suggestions: EmojiSuggestion[];
  selectedIndex: number;
  onSelect: (suggestion: EmojiSuggestion) => void;
  position?: "above" | "below";
};

export const EmojiAutocomplete = React.memo(function EmojiAutocomplete({
  suggestions,
  selectedIndex,
  onSelect,
  position = "above",
}: EmojiAutocompleteProps) {
  const listVirtualizerRef = React.useRef<ListVirtualizer | null>(null);

  React.useEffect(() => {
    listVirtualizerRef.current?.scrollToIndex(selectedIndex, {
      align: "auto",
    });
  }, [selectedIndex]);

  const handleVirtualizer = React.useCallback(
    (virtualizer: ListVirtualizer) => {
      listVirtualizerRef.current = virtualizer;
    },
    [],
  );

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
          "rounded-xl p-1",
          POPOVER_CUSTOM_ENTER_MOTION_CLASS,
          position === "below"
            ? "origin-top slide-in-from-top-1"
            : "origin-bottom slide-in-from-bottom-1",
          POPOVER_SURFACE_CLASS,
        )}
        data-testid="emoji-autocomplete"
        style={POPOVER_SHADOW_STYLE}
      >
        <VirtualizedList
          className="max-h-48"
          estimateSize={36}
          getItemKey={(suggestion) => suggestion.id}
          items={suggestions}
          onVirtualizer={handleVirtualizer}
          renderItem={(suggestion, index) => (
            <button
              className={cn(
                "flex w-full cursor-pointer items-center gap-2 rounded-lg px-3 py-1.5 text-left text-sm",
                index === selectedIndex
                  ? "bg-accent text-accent-foreground"
                  : "text-popover-foreground hover:bg-accent/50",
              )}
              onMouseDown={(event) => {
                event.preventDefault();
                onSelect(suggestion);
              }}
              tabIndex={-1}
              type="button"
            >
              {suggestion.url ? (
                <img
                  alt={`:${suggestion.id}:`}
                  src={suggestion.url}
                  className="h-5 w-5 object-contain"
                  draggable={false}
                />
              ) : (
                <span className="text-lg leading-none">
                  {suggestion.native}
                </span>
              )}
              <span className="truncate text-muted-foreground">
                :{suggestion.id}:
              </span>
            </button>
          )}
        />
      </div>
    </div>
  );
});
