import { Hash, Search, X } from "lucide-react";
import * as React from "react";

import { buildChannelLink } from "@/features/messages/lib/channelLink";
import type { Channel } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { ACTION_TRAY_SURFACE_CLASS } from "@/shared/ui/actionTray";
import { Button } from "@/shared/ui/button";
import {
  MODAL_SEARCH_INPUT_CLASS,
  MODAL_SEARCH_SHELL_CLASS,
} from "@/shared/ui/modalSearchStyles";
import { POPOVER_CUSTOM_ENTER_MOTION_CLASS } from "@/shared/ui/popoverSurface";

type WelcomeChannelChipPickerProps = {
  channels: Channel[];
  insert: { id: string; title: string; url: string };
  onClose: () => void;
  onRemove: () => void;
  onSelect: (channel: Channel) => void;
  position: { left: number; top: number };
};

export function WelcomeChannelChipPicker({
  channels,
  insert,
  onClose,
  onRemove,
  onSelect,
  position,
}: WelcomeChannelChipPickerProps) {
  const inputRef = React.useRef<HTMLInputElement>(null);
  const [query, setQuery] = React.useState(insert.url ? insert.title : "");
  const [highlightedIndex, setHighlightedIndex] = React.useState(0);
  const matchingChannels = React.useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();
    return channels
      .filter(
        (channel) =>
          channel.channelType !== "dm" && channel.archivedAt === null,
      )
      .filter(
        (channel) =>
          !normalizedQuery ||
          channel.name.toLowerCase().includes(normalizedQuery) ||
          channel.id.toLowerCase().includes(normalizedQuery),
      )
      .sort((a, b) => a.name.localeCompare(b.name));
  }, [channels, query]);
  const highlightedChannel = matchingChannels[highlightedIndex];
  const isSearching = query.trim().length > 0;

  React.useEffect(() => {
    inputRef.current?.focus({ preventScroll: true });
  }, []);

  function handleKeyDown(event: React.KeyboardEvent<HTMLInputElement>) {
    if (matchingChannels.length === 0) return;
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setHighlightedIndex((index) =>
        Math.min(index + 1, matchingChannels.length - 1),
      );
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      setHighlightedIndex((index) => Math.max(index - 1, 0));
    } else if (event.key === "Enter" && highlightedChannel) {
      event.preventDefault();
      onSelect(highlightedChannel);
    }
  }

  return (
    <div
      aria-label="Edit channel"
      className={cn(
        "absolute z-30 w-80 space-y-3 overflow-visible rounded-xl p-4",
        ACTION_TRAY_SURFACE_CLASS,
        POPOVER_CUSTOM_ENTER_MOTION_CLASS,
      )}
      onKeyDown={(event) => {
        if (event.key !== "Escape") return;
        event.preventDefault();
        event.stopPropagation();
        onClose();
      }}
      role="dialog"
      style={position}
    >
      <div className="flex items-center justify-between gap-3">
        <p className="text-sm font-semibold leading-none">Channel</p>
        <Button
          aria-label="Close chip editor"
          className="h-8 w-8"
          onClick={onClose}
          size="icon"
          type="button"
          variant="ghost"
        >
          <X className="h-4 w-4" />
        </Button>
      </div>
      <div className="relative">
        <label
          className={cn(MODAL_SEARCH_SHELL_CLASS, "mt-0")}
          htmlFor={`welcome-channel-search-${insert.id}`}
        >
          <Search className="h-4 w-4 shrink-0 text-muted-foreground/55 transition-colors duration-150 ease-out group-hover/search:text-muted-foreground group-focus-within/search:text-foreground" />
          <input
            aria-controls={
              isSearching ? `welcome-channel-results-${insert.id}` : undefined
            }
            aria-activedescendant={
              isSearching && highlightedChannel
                ? `welcome-channel-result-${highlightedChannel.id}`
                : undefined
            }
            aria-expanded={isSearching}
            aria-label="Search channels"
            aria-autocomplete="list"
            autoCapitalize="none"
            autoComplete="off"
            autoCorrect="off"
            className={MODAL_SEARCH_INPUT_CLASS}
            id={`welcome-channel-search-${insert.id}`}
            onChange={(event) => {
              setQuery(event.target.value);
              setHighlightedIndex(0);
            }}
            onKeyDown={handleKeyDown}
            placeholder="Search channels…"
            ref={inputRef}
            role="combobox"
            spellCheck={false}
            value={query}
          />
        </label>
        {isSearching ? (
          <div
            aria-label="Channel results"
            className="absolute left-0 right-0 top-full z-40 mt-2 max-h-64 overflow-y-auto rounded-xl border border-border/70 bg-background shadow-lg divide-y divide-border/55"
            id={`welcome-channel-results-${insert.id}`}
            role="listbox"
          >
            {matchingChannels.length > 0 ? (
              matchingChannels.map((channel, index) => (
                <button
                  aria-selected={insert.url === buildChannelLink(channel.id)}
                  className={cn(
                    "flex min-h-14 w-full items-center gap-3 px-4 py-3 text-left transition-colors duration-150 ease-out hover:bg-muted/40 focus-visible:bg-muted/40 focus-visible:outline-none",
                    index === highlightedIndex && "bg-muted/40",
                  )}
                  data-testid={`welcome-channel-result-${channel.id}`}
                  id={`welcome-channel-result-${channel.id}`}
                  key={channel.id}
                  onClick={() => onSelect(channel)}
                  onMouseMove={() => setHighlightedIndex(index)}
                  role="option"
                  type="button"
                >
                  <Hash className="h-4 w-4 shrink-0 text-muted-foreground" />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm font-medium text-foreground">
                      {channel.name}
                    </span>
                    {channel.description ? (
                      <span className="mt-0.5 block truncate text-xs text-muted-foreground">
                        {channel.description}
                      </span>
                    ) : null}
                  </span>
                </button>
              ))
            ) : (
              <p className="px-4 py-5 text-center text-sm text-muted-foreground">
                No channels found.
              </p>
            )}
          </div>
        ) : null}
      </div>
      <Button
        className="text-destructive hover:bg-destructive/10 hover:text-destructive"
        onClick={onRemove}
        size="sm"
        type="button"
        variant="ghost"
      >
        Remove chip
      </Button>
    </div>
  );
}
