import * as React from "react";
import {
  LoaderCircle,
  MessagesSquare,
  Search,
  type LucideIcon,
} from "lucide-react";

import { useUsersBatchQuery } from "@/features/profile/hooks";
import { useSearchMessagesQuery } from "@/features/search/hooks";
import type { Channel, SearchHit } from "@/shared/api/types";
import {
  ChannelResultBody,
  MessageResultBody,
  resultIcon,
  resultKey,
  resultTestId,
  SearchResultShell,
  type SearchResult,
} from "@/features/search/ui/SearchResultItem";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import { Skeleton } from "@/shared/ui/skeleton";

const MIN_QUERY_LENGTH = 2;

function SearchState({
  icon: Icon,
  title,
  description,
}: {
  icon: LucideIcon;
  title: string;
  description: string;
}) {
  return (
    <div className="flex flex-col items-center justify-center px-6 py-16 text-center">
      <div className="flex h-12 w-12 items-center justify-center rounded-2xl bg-primary/10 text-primary">
        <Icon className="h-5 w-5" />
      </div>
      <p className="mt-4 text-base font-semibold tracking-tight">{title}</p>
      <p className="mt-2 max-w-md text-sm text-muted-foreground">
        {description}
      </p>
    </div>
  );
}

function SearchLoadingState() {
  return (
    <div className="space-y-3 px-3 py-3" data-testid="search-loading">
      {["first", "second", "third"].map((row) => (
        <div
          className="rounded-2xl border border-border/80 bg-card/60 p-4"
          key={row}
        >
          <Skeleton className="h-4 w-32" />
          <Skeleton className="mt-3 h-4 w-full" />
          <Skeleton className="mt-2 h-4 w-3/4" />
        </div>
      ))}
    </div>
  );
}

type SearchDialogProps = {
  channels: Channel[];
  currentPubkey?: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onOpenChannel: (channelId: string) => void;
  onOpenResult: (hit: SearchHit) => void;
};

export function SearchDialog({
  channels,
  currentPubkey,
  open,
  onOpenChange,
  onOpenChannel,
  onOpenResult,
}: SearchDialogProps) {
  const [query, setQuery] = React.useState("");
  const [debouncedQuery, setDebouncedQuery] = React.useState("");
  const [selectedIndex, setSelectedIndex] = React.useState(0);
  const inputRef = React.useRef<HTMLInputElement>(null);
  const channelLookup = React.useMemo(
    () => new Map(channels.map((channel) => [channel.id, channel])),
    [channels],
  );

  const searchQuery = useSearchMessagesQuery(debouncedQuery, {
    enabled: open,
    limit: 12,
  });

  const messageResults = searchQuery.data?.hits ?? [];
  const channelResults = React.useMemo(() => {
    if (debouncedQuery.length < MIN_QUERY_LENGTH) {
      return [];
    }

    const normalizedQuery = debouncedQuery.toLowerCase();

    return channels
      .filter(
        (channel) =>
          channel.channelType !== "dm" &&
          (channel.archivedAt
            ? channel.isMember
            : channel.visibility === "open" || channel.isMember) &&
          (channel.name.toLowerCase().includes(normalizedQuery) ||
            channel.description.toLowerCase().includes(normalizedQuery)),
      )
      .sort((a, b) => {
        const aNameMatches = a.name.toLowerCase().includes(normalizedQuery);
        const bNameMatches = b.name.toLowerCase().includes(normalizedQuery);

        if (aNameMatches !== bNameMatches) {
          return aNameMatches ? -1 : 1;
        }

        return a.name.localeCompare(b.name);
      })
      .slice(0, 5);
  }, [channels, debouncedQuery]);
  const results = React.useMemo<SearchResult[]>(
    () => [
      ...channelResults.map((channel) => ({
        kind: "channel" as const,
        channel,
      })),
      ...messageResults.map((hit) => ({
        kind: "message" as const,
        hit,
      })),
    ],
    [channelResults, messageResults],
  );
  const resultProfilesQuery = useUsersBatchQuery(
    messageResults.map((hit) => hit.pubkey),
    {
      enabled: open && messageResults.length > 0,
    },
  );
  const resultProfiles = resultProfilesQuery.data?.profiles;

  const openResult = React.useCallback(
    (result: SearchResult) => {
      onOpenChange(false);

      if (result.kind === "channel") {
        onOpenChannel(result.channel.id);
        return;
      }

      onOpenResult(result.hit);
    },
    [onOpenChange, onOpenChannel, onOpenResult],
  );

  React.useEffect(() => {
    const trimmed = query.trim();
    if (trimmed.length < MIN_QUERY_LENGTH) {
      setDebouncedQuery("");
      return;
    }

    const timeout = window.setTimeout(() => {
      setDebouncedQuery(trimmed);
    }, 300);

    return () => {
      window.clearTimeout(timeout);
    };
  }, [query]);

  React.useEffect(() => {
    if (!open) {
      setQuery("");
      setDebouncedQuery("");
      setSelectedIndex(0);
    }
  }, [open]);

  React.useEffect(() => {
    setSelectedIndex((current) => {
      if (results.length === 0) {
        return 0;
      }

      return Math.min(current, results.length - 1);
    });
  }, [results]);

  const selectedResult = results[selectedIndex];

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        className="gap-0 overflow-hidden p-0"
        data-testid="search-dialog"
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          inputRef.current?.focus();
        }}
      >
        <DialogHeader className="border-b border-border/80 px-6 py-5">
          <DialogTitle className="flex items-center gap-3">
            <span className="flex h-10 w-10 items-center justify-center rounded-2xl bg-primary text-primary-foreground shadow-sm">
              <Search className="h-4 w-4" />
            </span>
            Search
          </DialogTitle>
          <DialogDescription>
            Full-text search across accessible channels.
          </DialogDescription>
          <div className="mt-4 flex items-center gap-3 rounded-2xl border border-input bg-card px-3 py-3 shadow-sm">
            <Search className="h-4 w-4 text-muted-foreground" />
            <Input
              autoFocus
              className="h-auto border-0 bg-transparent px-0 py-0 text-base shadow-none focus-visible:ring-0"
              data-testid="search-input"
              onChange={(event) => {
                setQuery(event.target.value);
                setSelectedIndex(0);
              }}
              onKeyDown={(event) => {
                if (event.key === "ArrowDown" && results.length > 0) {
                  event.preventDefault();
                  setSelectedIndex((current) =>
                    Math.min(current + 1, results.length - 1),
                  );
                  return;
                }

                if (event.key === "ArrowUp" && results.length > 0) {
                  event.preventDefault();
                  setSelectedIndex((current) => Math.max(current - 1, 0));
                  return;
                }

                if (
                  event.key === "Enter" &&
                  !event.nativeEvent.isComposing &&
                  selectedResult
                ) {
                  event.preventDefault();
                  openResult(selectedResult);
                }
              }}
              placeholder="Search messages, approvals, and forum posts"
              ref={inputRef}
              value={query}
            />
            <span className="hidden shrink-0 text-xs text-muted-foreground/50 sm:block">
              &#x2318;K
            </span>
          </div>
        </DialogHeader>

        <div className="max-h-[60vh] overflow-y-auto">
          {debouncedQuery.length < MIN_QUERY_LENGTH ? (
            <SearchState
              description="Type at least two characters to search the relay-backed history for streams, forums, DMs, approvals, and agent updates."
              icon={MessagesSquare}
              title="Search message history"
            />
          ) : searchQuery.isLoading && results.length === 0 ? (
            <SearchLoadingState />
          ) : searchQuery.error instanceof Error && results.length === 0 ? (
            <SearchState
              description={searchQuery.error.message}
              icon={LoaderCircle}
              title="Search unavailable"
            />
          ) : results.length === 0 ? (
            <SearchState
              description="Try a different keyword, channel name, or phrase from the message body."
              icon={Search}
              title="No matches found"
            />
          ) : (
            <div className="p-3" data-testid="search-results">
              <div className="mb-3 flex items-center justify-between px-2 text-xs font-semibold uppercase tracking-[0.16em] text-muted-foreground">
                <span>
                  {channelResults.length +
                    (searchQuery.data?.found ?? messageResults.length)}{" "}
                  results
                </span>
                <span>Enter to open</span>
              </div>

              <div className="space-y-2">
                {results.map((result, index) => (
                  <SearchResultShell
                    icon={resultIcon(result, channelLookup)}
                    isSelected={index === selectedIndex}
                    key={resultKey(result)}
                    onClick={() => openResult(result)}
                    onMouseEnter={() => setSelectedIndex(index)}
                    testId={resultTestId(result)}
                  >
                    {result.kind === "channel" ? (
                      <ChannelResultBody channel={result.channel} />
                    ) : (
                      <MessageResultBody
                        currentPubkey={currentPubkey}
                        hit={result.hit}
                        resultProfiles={resultProfiles}
                      />
                    )}
                  </SearchResultShell>
                ))}
              </div>
            </div>
          )}
        </div>

        <div className="border-t border-border/80 bg-card/50 px-6 py-3 text-xs text-muted-foreground">
          Search is relay-backed and scoped to channels you can access.
        </div>
      </DialogContent>
    </Dialog>
  );
}
