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
  Dialog,
  DialogContent,
  DialogTitle,
  DialogTrigger,
} from "@/shared/ui/dialog";
import { Skeleton } from "@/shared/ui/skeleton";
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
}: TopbarSearchProps) {
  const [isOpen, setIsOpen] = React.useState(false);
  const [selectedMenuIndex, setSelectedMenuIndex] = React.useState(0);
  const triggerRef = React.useRef<HTMLButtonElement>(null);
  const dialogInputRef = React.useRef<HTMLInputElement>(null);
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
    if (focusRequest === 0) {
      return;
    }

    setIsOpen(true);
    triggerRef.current?.focus();
  }, [focusRequest]);

  React.useEffect(() => {
    if (!isOpen) {
      return;
    }

    const animationFrame = window.requestAnimationFrame(() => {
      dialogInputRef.current?.focus();
    });

    return () => {
      window.cancelAnimationFrame(animationFrame);
    };
  }, [isOpen]);

  React.useEffect(() => {
    setSelectedMenuIndex((current) => {
      if (results.length === 0) {
        return 0;
      }

      return Math.min(current, results.length - 1);
    });
  }, [results]);

  const handleDialogInputKeyDown = React.useCallback(
    (event: React.KeyboardEvent<HTMLInputElement>) => {
      if (event.key === "ArrowDown" && results.length > 0) {
        event.preventDefault();
        setSelectedMenuIndex((current) =>
          Math.min(current + 1, results.length - 1),
        );
        return;
      }

      if (event.key === "ArrowUp" && results.length > 0) {
        event.preventDefault();
        setSelectedMenuIndex((current) => Math.max(current - 1, 0));
        return;
      }

      if (event.key === "Enter" && !event.nativeEvent.isComposing) {
        event.preventDefault();
        const result = results[selectedMenuIndex];
        if (result) {
          openResult(result);
        }
      }
    },
    [openResult, results, selectedMenuIndex],
  );

  const searchResultContent =
    debouncedQuery.length < MIN_SEARCH_QUERY_LENGTH ? (
      <div className="px-4 py-5 text-sm text-muted-foreground">
        <p>Type at least two characters for live suggestions.</p>
      </div>
    ) : searchQuery.isLoading && results.length === 0 ? (
      <SearchResultsSkeleton />
    ) : searchQuery.error instanceof Error && results.length === 0 ? (
      <p className="px-4 py-5 text-sm text-destructive">
        {searchQuery.error.message}
      </p>
    ) : results.length === 0 ? (
      <p className="px-4 py-5 text-sm text-muted-foreground">
        No matches for <span className="font-semibold">{trimmedQuery}</span>.
      </p>
    ) : (
      <div className="max-h-[420px] overflow-y-auto p-1.5" role="listbox">
        {results.map((result, index) => (
          <button
            aria-selected={index === selectedMenuIndex}
            className={cn(
              "flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-left transition-colors",
              index === selectedMenuIndex
                ? "bg-accent text-accent-foreground"
                : "hover:bg-accent/70",
            )}
            key={resultKey(result)}
            onClick={() => openResult(result)}
            onMouseEnter={() => setSelectedMenuIndex(index)}
            role="option"
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
    );

  return (
    <div className={cn("relative", className)}>
      <Dialog open={isOpen} onOpenChange={setIsOpen}>
        <DialogTrigger asChild>
          <button
            aria-label="Search everything"
            className="group/search flex h-7 w-full items-center gap-2 rounded-lg border border-border/70 bg-background px-2.5 text-left text-xs text-muted-foreground shadow-xs transition-colors duration-150 ease-out hover:bg-muted/70 hover:text-foreground focus-visible:border-border focus-visible:bg-muted/70 focus-visible:text-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
            data-testid="open-search"
            ref={triggerRef}
            type="button"
          >
            <Search className="h-4 w-4 shrink-0 text-muted-foreground/55 transition-colors duration-150 ease-out group-hover/search:text-muted-foreground group-focus-visible/search:text-foreground" />
            <span
              className={cn(
                "min-w-0 flex-1 translate-y-px truncate transition-colors duration-150 ease-out",
                query ? "text-foreground" : "text-muted-foreground/55",
              )}
            >
              {query || "Search everything"}
            </span>
            <kbd className="shrink-0 text-2xs text-muted-foreground/70">
              &#x2318;K
            </kbd>
          </button>
        </DialogTrigger>
        <DialogContent
          aria-busy={searchQuery.isLoading && results.length === 0}
          className="mt-[18vh] max-w-2xl self-start gap-0 overflow-hidden rounded-2xl p-0 shadow-2xl data-[state=closed]:zoom-out-100 data-[state=open]:zoom-in-100"
          data-testid="search-results"
          onOpenAutoFocus={(event) => {
            event.preventDefault();
            dialogInputRef.current?.focus();
          }}
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            triggerRef.current?.focus();
          }}
          showCloseButton={false}
        >
          <DialogTitle className="sr-only">Search everything</DialogTitle>
          <div className="flex h-12 items-center gap-3 border-b border-border/70 px-4">
            <Search className="h-4 w-4 shrink-0 text-muted-foreground" />
            <input
              aria-label="Search everything"
              autoCapitalize="none"
              autoCorrect="off"
              className="min-w-0 flex-1 bg-transparent text-base text-foreground placeholder:text-muted-foreground outline-none"
              data-testid="search-dialog-input"
              ref={dialogInputRef}
              onChange={(event) => {
                setQuery(event.target.value);
                setSelectedMenuIndex(0);
              }}
              onKeyDown={handleDialogInputKeyDown}
              placeholder="Search everything"
              spellCheck={false}
              value={query}
            />
            <kbd className="shrink-0 rounded border border-border/70 bg-muted/70 px-1.5 py-0.5 text-2xs text-muted-foreground">
              ESC
            </kbd>
          </div>
          {searchResultContent}
        </DialogContent>
      </Dialog>
    </div>
  );
}
