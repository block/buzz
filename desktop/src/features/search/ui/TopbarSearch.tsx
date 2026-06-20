import { Search } from "lucide-react";
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
import {
  POPOVER_CUSTOM_ENTER_MOTION_CLASS,
  POPOVER_SHADOW_STYLE,
  POPOVER_SURFACE_CLASS,
} from "@/shared/ui/popoverSurface";
import { Skeleton } from "@/shared/ui/skeleton";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type SearchPlacement = "topbar" | "sidebar";

type TopbarSearchProps = {
  channels: Channel[];
  className?: string;
  currentPubkey?: string;
  focusRequest?: number;
  onOpenChannel: (channelId: string) => void;
  onOpenResult: (hit: SearchHit) => void;
  /**
   * Controls how the results popover is anchored. `topbar` centers a wide
   * panel under the input; `sidebar` left-anchors a narrower panel that fits
   * within the persistent sidebar column.
   */
  placement?: SearchPlacement;
};

const RESULTS_POPOVER_CLASS: Record<SearchPlacement, string> = {
  topbar:
    "left-1/2 w-[620px] max-w-[min(82vw,620px)] -translate-x-1/2 slide-in-from-top-1",
  sidebar: "left-0 right-0 w-auto slide-in-from-top-1",
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

const searchSkeletonRows = [
  {
    iconShape: "rounded-md",
    key: "channel",
    metaWidth: "w-16",
    previewWidth: "w-48",
    titleWidth: "w-28",
    trailingWidth: "w-14",
  },
  {
    iconShape: "rounded-full",
    key: "message",
    metaWidth: "w-24",
    previewWidth: "w-72",
    titleWidth: "w-24",
    trailingWidth: "w-20",
  },
  {
    iconShape: "rounded-full",
    key: "note",
    metaWidth: "w-20",
    previewWidth: "w-60",
    titleWidth: "w-32",
    trailingWidth: "w-16",
  },
] as const;

function SearchResultsSkeleton() {
  return (
    <div
      aria-hidden="true"
      className="max-h-[360px] overflow-y-auto p-1"
      data-testid="search-results-loading"
    >
      {searchSkeletonRows.map((row) => (
        <div
          className="flex w-full items-center gap-3 rounded-lg px-3 py-2"
          key={row.key}
        >
          <Skeleton className={cn("h-7 w-7 shrink-0", row.iconShape)} />
          <div className="min-w-0 flex-1">
            <div className="flex min-w-0 items-center gap-1.5">
              <Skeleton className={cn("h-4", row.titleWidth)} />
              <Skeleton className={cn("h-3", row.metaWidth)} />
            </div>
            <Skeleton
              className={cn("mt-1.5 h-3 max-w-full", row.previewWidth)}
            />
          </div>
          <Skeleton className={cn("h-3 shrink-0", row.trailingWidth)} />
        </div>
      ))}
    </div>
  );
}

export function TopbarSearch({
  channels,
  className,
  currentPubkey,
  focusRequest = 0,
  onOpenChannel,
  onOpenResult,
  placement = "topbar",
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
      <div className="group/search flex h-7 items-center gap-2 rounded-lg border border-border/70 bg-background px-2.5 text-xs text-muted-foreground shadow-xs transition-colors duration-150 ease-out focus-within:border-border focus-within:bg-muted/70 focus-within:text-foreground hover:bg-muted/70">
        <Search className="h-4 w-4 shrink-0 text-muted-foreground/55 transition-colors duration-150 ease-out group-focus-within/search:text-foreground group-hover/search:text-muted-foreground" />
        <input
          aria-label="Search everything"
          autoCapitalize="none"
          autoCorrect="off"
          className="min-w-0 translate-y-px flex-1 bg-transparent text-xs text-foreground placeholder:text-muted-foreground/55 outline-none transition-colors duration-150 ease-out group-focus-within/search:placeholder:text-foreground group-hover/search:placeholder:text-muted-foreground"
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
          spellCheck={false}
          value={query}
        />
        <kbd className="shrink-0 text-2xs text-muted-foreground/70">
          &#x2318;K
        </kbd>
      </div>

      {showSuggestions ? (
        <div
          aria-busy={searchQuery.isLoading && results.length === 0}
          className={cn(
            "absolute top-full z-50 mt-1 origin-top overflow-hidden rounded-xl",
            RESULTS_POPOVER_CLASS[placement],
            POPOVER_CUSTOM_ENTER_MOTION_CLASS,
            POPOVER_SURFACE_CLASS,
          )}
          data-testid="search-results"
          style={POPOVER_SHADOW_STYLE}
        >
          {debouncedQuery.length < MIN_SEARCH_QUERY_LENGTH ? (
            <div className="px-3 py-3 text-2xs text-muted-foreground">
              <p>Type at least two characters for live suggestions.</p>
            </div>
          ) : searchQuery.isLoading && results.length === 0 ? (
            <SearchResultsSkeleton />
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
            <div className="max-h-[360px] overflow-y-auto p-1">
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
                  <span className="shrink-0 text-2xs text-muted-foreground/75">
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
