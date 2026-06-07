import { LoaderCircle, Search } from "lucide-react";
import * as React from "react";

import { resolveUserLabel } from "@/features/profile/lib/identity";
import {
  MIN_SEARCH_QUERY_LENGTH,
  useSearchResults,
} from "@/features/search/useSearchResults";
import {
  resultIcon,
  resultKey,
  resultTestId,
  type SearchResult,
} from "@/features/search/ui/SearchResultItem";
import type { Channel, SearchHit } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type TopbarSearchProps = {
  channels: Channel[];
  className?: string;
  currentPubkey?: string;
  focusRequest?: number;
  onOpenChannel: (channelId: string) => void;
  onOpenResult: (hit: SearchHit) => void;
};

function describeSearchHit(hit: SearchHit) {
  switch (hit.kind) {
    case 45001:
      return "Forum post";
    case 45003:
      return "Forum reply";
    case 43001:
      return "Agent job";
    case 43003:
      return "Agent update";
    case 46010:
      return "Approval";
    default:
      return "Message";
  }
}

function truncateResultText(content: string, maxLength = 96) {
  const trimmed = content.trim();
  if (trimmed.length === 0) {
    return "No message body.";
  }

  if (trimmed.length <= maxLength) {
    return trimmed;
  }

  return `${trimmed.slice(0, maxLength - 3).trimEnd()}...`;
}

function formatRelativeTime(unixSeconds: number) {
  const diff = Math.floor(Date.now() / 1_000) - unixSeconds;

  if (diff < 60) {
    return "now";
  }

  if (diff < 60 * 60) {
    return `${Math.floor(diff / 60)}m`;
  }

  if (diff < 60 * 60 * 24) {
    return `${Math.floor(diff / (60 * 60))}h`;
  }

  return new Intl.DateTimeFormat("en-US", {
    month: "short",
    day: "numeric",
  }).format(new Date(unixSeconds * 1_000));
}

export function TopbarSearch({
  channels,
  className,
  currentPubkey,
  focusRequest = 0,
  onOpenChannel,
  onOpenResult,
}: TopbarSearchProps) {
  const [isOpen, setIsOpen] = React.useState(false);
  const [selectedMenuIndex, setSelectedMenuIndex] = React.useState(0);
  const inputRef = React.useRef<HTMLInputElement>(null);
  const rootRef = React.useRef<HTMLDivElement>(null);
  const {
    channelLookup,
    debouncedQuery,
    query,
    resultProfiles,
    results,
    searchQuery,
    setQuery,
  } = useSearchResults({ channels, enabled: isOpen, limit: 8 });
  const trimmedQuery = query.trim();
  const showSuggestions = isOpen;
  const selectableCount = showSuggestions ? results.length : 0;

  const openResult = React.useCallback(
    (result: SearchResult) => {
      setIsOpen(false);
      setQuery("");

      if (result.kind === "channel") {
        onOpenChannel(result.channel.id);
        return;
      }

      onOpenResult(result.hit);
    },
    [onOpenChannel, onOpenResult, setQuery],
  );

  React.useEffect(() => {
    function handlePointerDown(event: PointerEvent) {
      if (
        event.target instanceof Node &&
        rootRef.current?.contains(event.target)
      ) {
        return;
      }

      setIsOpen(false);
    }

    window.addEventListener("pointerdown", handlePointerDown);
    return () => {
      window.removeEventListener("pointerdown", handlePointerDown);
    };
  }, []);

  React.useEffect(() => {
    if (focusRequest === 0) {
      return;
    }

    setIsOpen(true);
    inputRef.current?.focus();
    inputRef.current?.select();
  }, [focusRequest]);

  React.useEffect(() => {
    setSelectedMenuIndex((current) => {
      if (selectableCount === 0) {
        return 0;
      }

      return Math.min(current, selectableCount - 1);
    });
  }, [selectableCount]);

  return (
    <div className={cn("relative", className)} ref={rootRef}>
      <div className="flex h-7 items-center gap-2 rounded-lg border border-border/70 bg-muted/45 px-2.5 text-xs text-muted-foreground shadow-xs backdrop-blur transition-colors focus-within:border-border focus-within:bg-muted/70 focus-within:text-foreground hover:bg-muted/70 supports-[backdrop-filter]:bg-muted/35">
        <Search className="h-3.5 w-3.5 shrink-0" />
        <input
          aria-label="Search everything"
          className="min-w-0 flex-1 bg-transparent text-xs text-foreground placeholder:text-muted-foreground outline-none"
          data-testid="open-search"
          ref={inputRef}
          onChange={(event) => {
            setIsOpen(true);
            setQuery(event.target.value);
            setSelectedMenuIndex(0);
          }}
          onFocus={() => setIsOpen(true)}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown" && selectableCount > 0) {
              event.preventDefault();
              setSelectedMenuIndex((current) =>
                Math.min(current + 1, selectableCount - 1),
              );
              return;
            }

            if (event.key === "ArrowUp" && selectableCount > 0) {
              event.preventDefault();
              setSelectedMenuIndex((current) => Math.max(current - 1, 0));
              return;
            }

            if (event.key === "Escape") {
              event.preventDefault();
              setIsOpen(false);
              return;
            }

            if (event.key === "Enter" && !event.nativeEvent.isComposing) {
              event.preventDefault();
              const result = results[selectedMenuIndex];
              if (result) {
                openResult(result);
              }
            }
          }}
          placeholder="Search everything"
          value={query}
        />
        <kbd className="shrink-0 text-[10px] text-muted-foreground/70">
          &#x2318;K
        </kbd>
      </div>

      {showSuggestions ? (
        <div
          className="absolute left-1/2 top-full z-50 mt-1 w-[620px] max-w-[min(82vw,620px)] -translate-x-1/2 overflow-hidden rounded-xl border border-border/80 bg-popover text-popover-foreground shadow-xl"
          data-testid="search-results"
        >
          {debouncedQuery.length < MIN_SEARCH_QUERY_LENGTH ? (
            <div className="px-3 py-3 text-[11px] text-muted-foreground">
              <p>Type at least two characters for live suggestions.</p>
            </div>
          ) : searchQuery.isLoading && results.length === 0 ? (
            <div className="flex items-center gap-2 px-3 py-3 text-xs text-muted-foreground">
              <LoaderCircle className="h-3.5 w-3.5 animate-spin" />
              Searching...
            </div>
          ) : searchQuery.error instanceof Error && results.length === 0 ? (
            <p className="px-3 py-3 text-xs text-destructive">
              {searchQuery.error.message}
            </p>
          ) : results.length === 0 ? (
            <p className="px-3 py-3 text-xs text-muted-foreground">
              No matches for{" "}
              <span className="font-semibold">{trimmedQuery}</span>.
            </p>
          ) : (
            <div className="max-h-[360px] overflow-y-auto p-1.5">
              {results.map((result, index) => (
                <button
                  className={cn(
                    "flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left transition-colors",
                    index === selectedMenuIndex
                      ? "bg-accent text-accent-foreground"
                      : "hover:bg-accent/70",
                  )}
                  key={resultKey(result)}
                  onClick={() => openResult(result)}
                  onMouseEnter={() => setSelectedMenuIndex(index)}
                  type="button"
                  data-testid={resultTestId(result)}
                >
                  {result.kind === "message" ? (
                    <UserAvatar
                      avatarUrl={
                        resultProfiles?.[result.hit.pubkey.toLowerCase()]
                          ?.avatarUrl ?? null
                      }
                      className="h-7 w-7"
                      displayName={resolveUserLabel({
                        currentPubkey,
                        profiles: resultProfiles,
                        pubkey: result.hit.pubkey,
                        preferResolvedSelfLabel: true,
                      })}
                      size="sm"
                    />
                  ) : (
                    <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-muted/70 text-muted-foreground">
                      {React.createElement(resultIcon(result, channelLookup), {
                        className: "h-4 w-4",
                      })}
                    </span>
                  )}
                  <span className="min-w-0 flex-1">
                    <span className="flex min-w-0 items-center gap-1.5">
                      <span className="truncate text-sm font-semibold">
                        {result.kind === "channel"
                          ? result.channel.name
                          : resolveUserLabel({
                              currentPubkey,
                              profiles: resultProfiles,
                              pubkey: result.hit.pubkey,
                              preferResolvedSelfLabel: true,
                            })}
                      </span>
                      <span className="truncate text-xs text-muted-foreground">
                        {result.kind === "channel"
                          ? result.channel.channelType
                          : `in #${result.hit.channelName ?? "unknown"}`}
                      </span>
                    </span>
                    <span className="block truncate text-xs text-muted-foreground">
                      {result.kind === "channel"
                        ? result.channel.description || "Channel"
                        : truncateResultText(result.hit.content)}
                    </span>
                  </span>
                  <span className="shrink-0 text-[11px] text-muted-foreground/75">
                    {result.kind === "channel"
                      ? "Channel"
                      : `${describeSearchHit(result.hit)} · ${formatRelativeTime(result.hit.createdAt)}`}
                  </span>
                </button>
              ))}
            </div>
          )}
        </div>
      ) : null}
    </div>
  );
}
