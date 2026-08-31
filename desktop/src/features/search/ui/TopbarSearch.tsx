import { Search } from "lucide-react";
import * as React from "react";

import { resolveUserLabel } from "@/features/profile/lib/identity";
import { getMinimumSearchQueryLength } from "@/features/search/hooks";
import { parseSearchOperators } from "@/features/search/lib/parseSearchOperators";
import { buildSearchResultPreview } from "@/features/search/lib/searchMatch";
import { useSearchResults } from "@/features/search/useSearchResults";
import {
  resultIcon,
  resultKey,
  resultTestId,
  type SearchResult,
} from "@/features/search/ui/SearchResultItem";
import {
  getChannelScopeLabel,
  SearchDialogInputRow,
} from "@/features/search/ui/SearchScopeControls";
import { SearchResultTrailing } from "@/features/search/ui/SearchResultTrailing";
import { HighlightedSearchText } from "@/features/search/ui/HighlightedSearchText";
import { useSearchMenuKeyboardNavigation } from "@/features/search/ui/useSearchMenuKeyboardNavigation";
import {
  getChannelActivityTime,
  getSuggestedSearchResults,
} from "@/features/search/ui/topbarSearchSuggestions";
import type { Channel, SearchHit, UserSearchResult } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { Dialog, DialogContent, DialogTitle } from "@/shared/ui/dialog";
import { useDeferredModalOpen } from "@/shared/ui/deferredModalOpen";
import {
  MENTION_CHIP_BASE_CLASSES,
  MESSAGE_MARKDOWN_CLASS,
} from "@/shared/ui/mentionChip";
import { Skeleton } from "@/shared/ui/skeleton";
import { UserAvatar } from "@/shared/ui/UserAvatar";

type TopbarSearchProps = {
  channelLabels?: Record<string, string>;
  channels: Channel[];
  className?: string;
  currentPubkey?: string;
  currentChannelId?: string | null;
  focusRequest?: number;
  onOpenChannel: (channelId: string) => void;
  onOpenResult: (hit: SearchHit, query: string) => void;
  onOpenUser?: (user: UserSearchResult) => void | Promise<void>;
  onBrowseChannels?: () => void | Promise<void>;
  onCreateAgent?: () => void | Promise<void>;
  onCreateChannel?: () => void | Promise<void>;
  suggestionChannels?: Channel[];
  unreadChannelCounts?: ReadonlyMap<string, number>;
  unreadChannelIds?: ReadonlySet<string>;
  scopeFocusRequest?: number;
  variant?: "bar" | "icon";
};

const SEARCH_RESULT_LIMIT = 40;
const EMPTY_UNREAD_CHANNEL_COUNTS: ReadonlyMap<string, number> = new Map(),
  EMPTY_UNREAD_CHANNEL_IDS: ReadonlySet<string> = new Set();
const SEARCH_SECTION_TITLE_CLASS =
  "sticky top-0 z-10 bg-background px-2.5 pb-1.5 pt-2 text-xs font-medium text-muted-foreground/70";
const SEARCH_RESULT_SECTION_ORDER = [
  "current-channel-messages",
  "channels",
  "direct-messages",
  "people",
  "agents",
  "messages",
  "actions",
] as const;

type SearchResultSectionKey = (typeof SEARCH_RESULT_SECTION_ORDER)[number];

type SearchResultSection = {
  key: SearchResultSectionKey;
  results: SearchResult[];
  title: string;
};

type SearchHitContextLabel = {
  channelLabel: string | null;
  text: string;
};

function formatRelativeTime(unixSeconds: number) {
  const diff = Math.floor(Date.now() / 1_000) - unixSeconds;

  if (diff < 60) {
    return "just now";
  }

  if (diff < 60 * 60) {
    return `${Math.floor(diff / 60)}m ago`;
  }

  if (diff < 60 * 60 * 24) {
    return `${Math.floor(diff / (60 * 60))}h ago`;
  }

  if (diff < 60 * 60 * 24 * 7) {
    return `${Math.floor(diff / (60 * 60 * 24))}d ago`;
  }

  return new Intl.DateTimeFormat("en-US", {
    month: "short",
    day: "numeric",
  }).format(new Date(unixSeconds * 1_000));
}

function getChannelSuggestionMeta(channel: Channel) {
  const activityTime = getChannelActivityTime(channel);

  if (activityTime > 0) {
    return formatRelativeTime(Math.floor(activityTime / 1_000));
  }

  return null;
}

function getChannelDisplayName(
  channel: Channel,
  channelLabels?: Record<string, string>,
) {
  return channelLabels?.[channel.id]?.trim() || channel.name;
}

function getChannelPreview(channel: Channel) {
  if (channel.channelType === "dm") {
    return "";
  }

  if (channel.description.trim()) {
    return channel.description;
  }

  return "";
}

function getUserDisplayName(user: UserSearchResult) {
  return (
    user.displayName?.trim() ||
    user.nip05Handle?.trim() ||
    truncatePubkey(user.pubkey)
  );
}

function getUserSecondaryLabel(user: UserSearchResult) {
  const displayName = user.displayName?.trim();
  const nip05Handle = user.nip05Handle?.trim();

  if (nip05Handle && nip05Handle !== displayName) {
    return nip05Handle;
  }

  return null;
}

function getSearchHitChannelName(
  hit: SearchHit,
  channelLookup: ReadonlyMap<string, Channel>,
  channelLabels?: Record<string, string>,
) {
  const channel = hit.channelId ? channelLookup.get(hit.channelId) : null;
  const channelName =
    (hit.channelId ? channelLabels?.[hit.channelId]?.trim() : null) ||
    hit.channelName?.trim() ||
    channel?.name.trim() ||
    null;

  if (!channelName) {
    return null;
  }

  return channelName;
}

function getSearchHitContextLabel(
  hit: SearchHit,
  channelLookup: ReadonlyMap<string, Channel>,
  channelLabels?: Record<string, string>,
): SearchHitContextLabel {
  const channel = hit.channelId ? channelLookup.get(hit.channelId) : null;
  const channelName = getSearchHitChannelName(
    hit,
    channelLookup,
    channelLabels,
  );

  if (channel?.channelType === "dm") {
    return {
      channelLabel: null,
      text: "Direct message",
    };
  }

  const isThread = hit.kind === 45003 || Boolean(hit.threadRootId);

  return {
    channelLabel: channelName,
    text: channelName
      ? `${isThread ? "Thread" : "Message"} in`
      : isThread
        ? "Thread"
        : "Message",
  };
}

function getResultSectionKey(
  result: SearchResult,
  currentChannelId?: string | null,
): SearchResultSectionKey {
  if (result.kind === "message" && result.hit.channelId === currentChannelId) {
    return "current-channel-messages";
  }

  if (result.kind === "channel") {
    return result.channel.channelType === "dm" ? "direct-messages" : "channels";
  }

  if (result.kind === "user") {
    return result.user.isAgent ? "agents" : "people";
  }

  if (result.kind === "action") {
    return "actions";
  }

  return "messages";
}

function getSectionTitle(sectionKey: SearchResultSectionKey) {
  switch (sectionKey) {
    case "current-channel-messages":
      return "In this conversation";
    case "channels":
      return "Channels";
    case "direct-messages":
      return "Direct messages";
    case "people":
      return "People";
    case "agents":
      return "Agents";
    case "messages":
      return "Most relevant";
    case "actions":
      return "Actions";
  }
}

function SearchHitContextLine({ label }: { label: SearchHitContextLabel }) {
  return (
    <span
      className={cn(
        MESSAGE_MARKDOWN_CLASS,
        "mt-0 flex min-w-0 items-center gap-1.5 text-2xs font-medium leading-3 text-muted-foreground/80",
      )}
    >
      <span className="shrink-0">{label.text}</span>
      {label.channelLabel ? (
        <span
          className={cn(
            MENTION_CHIP_BASE_CLASSES,
            "search-channel-chip min-w-0 max-w-full overflow-hidden",
          )}
          data-channel-link=""
        >
          <span className="truncate">#{label.channelLabel}</span>
        </span>
      ) : null}
    </span>
  );
}

function groupSearchResults(
  results: SearchResult[],
  currentChannelId?: string | null,
): SearchResultSection[] {
  const resultsBySection = new Map<SearchResultSectionKey, SearchResult[]>();

  for (const result of results) {
    const sectionKey = getResultSectionKey(result, currentChannelId);
    const sectionResults = resultsBySection.get(sectionKey) ?? [];
    sectionResults.push(result);
    resultsBySection.set(sectionKey, sectionResults);
  }

  return SEARCH_RESULT_SECTION_ORDER.flatMap((sectionKey) => {
    const sectionResults = resultsBySection.get(sectionKey);

    if (!sectionResults || sectionResults.length === 0) {
      return [];
    }

    return [
      {
        key: sectionKey,
        results: sectionResults,
        title: getSectionTitle(sectionKey),
      },
    ];
  });
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
      className="p-1"
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
  channelLabels,
  channels,
  className,
  currentChannelId,
  currentPubkey,
  focusRequest = 0,
  onOpenChannel,
  onOpenResult,
  onOpenUser,
  onBrowseChannels,
  onCreateAgent,
  onCreateChannel,
  scopeFocusRequest = 0,
  suggestionChannels,
  unreadChannelCounts = EMPTY_UNREAD_CHANNEL_COUNTS,
  unreadChannelIds = EMPTY_UNREAD_CHANNEL_IDS,
  variant = "bar",
}: TopbarSearchProps) {
  const [isOpen, setIsOpen] = React.useState(false);
  const [scopeChannelId, setScopeChannelId] = React.useState<string | null>(
    null,
  );
  const [selectedMenuIndex, setSelectedMenuIndex] = React.useState(0);
  const triggerRef = React.useRef<HTMLButtonElement>(null);
  const dialogInputRef = React.useRef<HTMLInputElement>(null);
  const { cancelDeferredModalOpen, openAfterExit, openNextFrame } =
    useDeferredModalOpen();
  const {
    channelLookup,
    debouncedQuery,
    fuzzyUserCandidatesQuery,
    isWaitingOnFromResolution,
    prioritizedChannelSearchQuery,
    query,
    resultProfiles,
    results,
    searchQuery,
    setQuery,
    userSearchQuery,
  } = useSearchResults({
    channelLabels,
    channels,
    enabled: isOpen,
    limit: SEARCH_RESULT_LIMIT,
    prioritizedChannelId: scopeChannelId ? null : currentChannelId,
    scopeChannelId,
  });
  const trimmedQuery = query.trim();
  // Bind highlights to the debounced result source so stale results can never
  // pair with newly typed text during the debounce window.
  const resultQuery = parseSearchOperators(debouncedQuery).text;
  const resultsAreCurrent = debouncedQuery === trimmedQuery;
  const isIconVariant = variant === "icon";
  const scopeChannel = scopeChannelId
    ? (channelLookup.get(scopeChannelId) ?? null)
    : null;
  const currentChannel = currentChannelId
    ? (channelLookup.get(currentChannelId) ?? null)
    : null;
  const currentScopeActionLabel = currentChannel
    ? currentChannel.channelType === "dm"
      ? "Search conversation"
      : "Search channel"
    : undefined;
  const scopeLabel = scopeChannel
    ? getChannelScopeLabel(scopeChannel, channelLabels, currentPubkey)
    : null;
  const currentPubkeyNormalized =
    currentPubkey && normalizePubkey(currentPubkey);
  const { suggestedResults, unreadResults } = React.useMemo(
    () =>
      getSuggestedSearchResults(
        suggestionChannels ?? channels,
        unreadChannelIds,
      ),
    [channels, suggestionChannels, unreadChannelIds],
  );
  const suggestionActionResults = React.useMemo(() => {
    const actions: SearchResult[] = [];

    if (onBrowseChannels) {
      actions.push({
        kind: "action",
        action: {
          id: "browse-channels",
          title: "Browse channels",
        },
      });
    }

    if (onCreateChannel) {
      actions.push({
        kind: "action",
        action: {
          id: "create-channel",
          title: "Create a new channel",
        },
      });
    }

    if (onCreateAgent) {
      actions.push({
        kind: "action",
        action: {
          id: "create-agent",
          title: "Create a new agent",
        },
      });
    }

    return actions;
  }, [onBrowseChannels, onCreateAgent, onCreateChannel]);
  const suggestionResults = React.useMemo(
    () => [...unreadResults, ...suggestedResults, ...suggestionActionResults],
    [unreadResults, suggestedResults, suggestionActionResults],
  );
  const minimumQueryLength = getMinimumSearchQueryLength(scopeChannelId);
  const isShowingSuggestions =
    Math.max(debouncedQuery.length, trimmedQuery.length) < minimumQueryLength;
  const searchableResults = React.useMemo(
    () =>
      results.filter(
        (result) =>
          result.kind !== "user" ||
          normalizePubkey(result.user.pubkey) !== currentPubkeyNormalized,
      ),
    [currentPubkeyNormalized, results],
  );
  const visibleSearchableResults = resultsAreCurrent ? searchableResults : [];
  const searchResultSections = React.useMemo(
    () =>
      groupSearchResults(
        visibleSearchableResults,
        scopeChannel ? null : currentChannelId,
      ),
    [currentChannelId, scopeChannel, visibleSearchableResults],
  );
  const groupedSearchResults = React.useMemo(
    () => searchResultSections.flatMap((section) => section.results),
    [searchResultSections],
  );
  const activeResults = isShowingSuggestions
    ? scopeChannel
      ? []
      : suggestionResults
    : resultsAreCurrent
      ? groupedSearchResults
      : [];
  const isSearchLoading =
    (!isShowingSuggestions && !resultsAreCurrent) ||
    isWaitingOnFromResolution ||
    searchQuery.isLoading ||
    prioritizedChannelSearchQuery.isLoading ||
    fuzzyUserCandidatesQuery.isLoading ||
    userSearchQuery.isLoading;

  const openSearchDialog = React.useCallback(
    (nextScopeChannelId: string | null = null) => {
      setScopeChannelId(nextScopeChannelId);
      setSelectedMenuIndex(0);
      openNextFrame(() => setIsOpen(true));
    },
    [openNextFrame],
  );

  const handleSearchOpenChange = React.useCallback(
    (nextOpen: boolean) => {
      if (nextOpen) {
        openSearchDialog(null);
        return;
      }

      cancelDeferredModalOpen();
      setSelectedMenuIndex(0);
      setScopeChannelId(null);
      setIsOpen(false);
    },
    [cancelDeferredModalOpen, openSearchDialog],
  );

  const openResult = React.useCallback(
    (result: SearchResult) => {
      setIsOpen(false);
      setScopeChannelId(null);
      setQuery("");

      if (result.kind === "channel") {
        onOpenChannel(result.channel.id);
        return;
      }

      if (result.kind === "user") {
        void onOpenUser?.(result.user);
        return;
      }

      if (result.kind === "action") {
        setSelectedMenuIndex(0);
        if (result.action.id === "browse-channels") {
          openAfterExit(() => {
            void onBrowseChannels?.();
          });
        } else if (result.action.id === "create-channel") {
          openAfterExit(() => {
            void onCreateChannel?.();
          });
        } else {
          openAfterExit(() => {
            void onCreateAgent?.();
          });
        }
        return;
      }

      onOpenResult(result.hit, resultQuery);
    },
    [
      onBrowseChannels,
      onCreateAgent,
      onCreateChannel,
      onOpenChannel,
      onOpenResult,
      onOpenUser,
      openAfterExit,
      setQuery,
      resultQuery,
    ],
  );

  // Edge-trigger: the counter never resets, so `!== 0` would replay on remount.
  const lastFocusRequestRef = React.useRef(focusRequest);
  React.useEffect(() => {
    if (focusRequest === lastFocusRequestRef.current) {
      return;
    }
    lastFocusRequestRef.current = focusRequest;

    openSearchDialog(null);
  }, [focusRequest, openSearchDialog]);

  const lastScopeFocusRequestRef = React.useRef(scopeFocusRequest);
  React.useEffect(() => {
    if (scopeFocusRequest === lastScopeFocusRequestRef.current) {
      return;
    }
    lastScopeFocusRequestRef.current = scopeFocusRequest;

    if (currentChannelId) {
      openSearchDialog(currentChannelId);
    }
  }, [currentChannelId, openSearchDialog, scopeFocusRequest]);

  const focusDialogInput = React.useCallback(() => {
    window.requestAnimationFrame(() => dialogInputRef.current?.focus());
  }, []);

  const activateCurrentChannelScope = React.useCallback(() => {
    if (!currentChannelId) {
      return;
    }

    setScopeChannelId(currentChannelId);
    setSelectedMenuIndex(0);
    focusDialogInput();
  }, [currentChannelId, focusDialogInput]);

  const removeChannelScope = React.useCallback(() => {
    setScopeChannelId(null);
    setSelectedMenuIndex(0);
    focusDialogInput();
  }, [focusDialogInput]);

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

  const handleDialogInputKeyDown = useSearchMenuKeyboardNavigation({
    activeResults,
    onActivateCurrentScope: currentChannel
      ? activateCurrentChannelScope
      : undefined,
    onOpenResult: openResult,
    onRemoveScope: removeChannelScope,
    query,
    scopeActive: Boolean(scopeChannel),
    selectedMenuIndex,
    setSelectedMenuIndex,
  });

  const renderSearchResultRow = (result: SearchResult, menuIndex: number) => {
    const channelDisplayName =
      result.kind === "channel"
        ? getChannelDisplayName(result.channel, channelLabels)
        : null;
    const userDisplayName =
      result.kind === "user" ? getUserDisplayName(result.user) : null;
    const messageAuthorLabel =
      result.kind === "message"
        ? resolveUserLabel({
            currentPubkey,
            profiles: resultProfiles,
            pubkey: result.hit.pubkey,
            preferResolvedSelfLabel: true,
          })
        : null;
    const messageContextLabel =
      result.kind === "message"
        ? getSearchHitContextLabel(result.hit, channelLookup, channelLabels)
        : null;
    const title =
      result.kind === "channel"
        ? channelDisplayName
        : result.kind === "action"
          ? result.action.title
          : result.kind === "user"
            ? userDisplayName
            : messageAuthorLabel;
    const preview =
      result.kind === "channel"
        ? getChannelPreview(result.channel)
        : result.kind === "action"
          ? result.action.description
          : result.kind === "user"
            ? getUserSecondaryLabel(result.user)
            : buildSearchResultPreview(result.hit.content, resultQuery);
    const unreadCount =
      result.kind === "channel"
        ? (unreadChannelCounts.get(result.channel.id) ?? 0)
        : 0;
    const isUnreadResult = result.kind === "channel" && unreadCount > 0;
    const trailingLabel =
      result.kind === "channel"
        ? getChannelSuggestionMeta(result.channel)
        : result.kind === "message"
          ? formatRelativeTime(result.hit.createdAt)
          : null;

    return (
      <button
        aria-selected={menuIndex === selectedMenuIndex}
        className={cn(
          "search-result-row flex w-full gap-3 rounded-lg px-2.5 text-left transition-colors",
          result.kind === "message" ? "items-start" : "items-center",
          result.kind === "message" ? "py-3.5" : "py-2.5",
          menuIndex === selectedMenuIndex
            ? "bg-muted/45 text-foreground"
            : "hover:bg-muted/35",
        )}
        key={resultKey(result)}
        onClick={() => openResult(result)}
        onMouseEnter={() => setSelectedMenuIndex(menuIndex)}
        role="option"
        type="button"
        data-testid={resultTestId(result)}
        data-search-result-index={menuIndex}
      >
        {result.kind === "message" ? (
          <UserAvatar
            avatarUrl={
              resultProfiles?.[result.hit.pubkey.toLowerCase()]?.avatarUrl ??
              null
            }
            className="h-8 w-8"
            displayName={resolveUserLabel({
              currentPubkey,
              profiles: resultProfiles,
              pubkey: result.hit.pubkey,
              preferResolvedSelfLabel: true,
            })}
            size="md"
          />
        ) : result.kind === "user" ? (
          <UserAvatar
            avatarUrl={result.user.avatarUrl}
            className="h-7 w-7"
            displayName={userDisplayName ?? result.user.pubkey}
            size="sm"
          />
        ) : (
          <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-background/70 text-muted-foreground">
            {React.createElement(resultIcon(result, channelLookup), {
              className: "h-4 w-4",
            })}
          </span>
        )}
        <span className="min-w-0 flex-1">
          {result.kind === "message" ? (
            <span className="grid w-full min-w-0 grid-cols-[minmax(0,1fr)_auto] gap-x-3">
              <span className="col-start-1 row-start-1 min-w-0 truncate text-sm font-semibold leading-4 text-foreground">
                {title}
              </span>
              {trailingLabel ? (
                <span className="col-start-2 row-start-1 flex shrink-0 items-center justify-self-end text-xs font-medium leading-4 text-muted-foreground/70">
                  {trailingLabel}
                </span>
              ) : null}
              {messageContextLabel ? (
                <span className="col-start-1 min-w-0">
                  <SearchHitContextLine label={messageContextLabel} />
                </span>
              ) : null}
              {preview ? (
                <span className="col-start-1 mt-1.5 block min-w-0 truncate text-sm leading-5 text-muted-foreground">
                  <HighlightedSearchText query={resultQuery} text={preview} />
                </span>
              ) : null}
            </span>
          ) : (
            <span className="block space-y-0.5">
              <span className="block truncate text-sm font-semibold">
                {title}
              </span>
              {preview ? (
                <span className="block truncate text-xs text-muted-foreground">
                  {preview}
                </span>
              ) : null}
            </span>
          )}
        </span>
        {result.kind !== "message" ? (
          <SearchResultTrailing
            channelId={isUnreadResult ? result.channel.id : undefined}
            isSelected={menuIndex === selectedMenuIndex}
            trailingLabel={trailingLabel}
            unreadCount={unreadCount}
          />
        ) : null}
      </button>
    );
  };

  const renderSearchResultSections = (sections: SearchResultSection[]) => {
    let resultIndex = 0;

    return sections.map((section) => (
      <div data-search-section={section.key} key={section.key}>
        <div className={SEARCH_SECTION_TITLE_CLASS}>{section.title}</div>
        {section.results.map((result) =>
          renderSearchResultRow(result, resultIndex++),
        )}
      </div>
    ));
  };
  const searchResultContent = isShowingSuggestions ? (
    scopeChannel ? null : suggestionResults.length === 0 ? (
      <div className="max-h-96 overflow-y-auto">
        <div className="px-4 py-5 text-sm text-muted-foreground">
          <p>No recent activity yet.</p>
        </div>
      </div>
    ) : (
      <div
        aria-label="Recent activity"
        className="buzz-search-scrollbar max-h-96 overflow-y-auto"
        role="listbox"
      >
        <div className="p-1.5">
          {(() => {
            let resultIndex = 0;

            return (
              <>
                {unreadResults.length > 0 ? (
                  <div data-search-section="unread">
                    <div className={SEARCH_SECTION_TITLE_CLASS}>Unread</div>
                    {unreadResults.map((result) =>
                      renderSearchResultRow(result, resultIndex++),
                    )}
                  </div>
                ) : null}
                {suggestedResults.length > 0 ? (
                  <div data-search-section="recent-activity">
                    <div className={SEARCH_SECTION_TITLE_CLASS}>
                      Recent activity
                    </div>
                    {suggestedResults.map((result) =>
                      renderSearchResultRow(result, resultIndex++),
                    )}
                  </div>
                ) : null}
                {suggestionActionResults.length > 0 ? (
                  <div>
                    <div className={SEARCH_SECTION_TITLE_CLASS}>Actions</div>
                    {suggestionActionResults.map((result) =>
                      renderSearchResultRow(result, resultIndex++),
                    )}
                  </div>
                ) : null}
              </>
            );
          })()}
        </div>
      </div>
    )
  ) : isSearchLoading && visibleSearchableResults.length === 0 ? (
    <div className="max-h-[min(60vh,32rem)] overflow-y-auto">
      <SearchResultsSkeleton />
    </div>
  ) : searchQuery.error instanceof Error &&
    visibleSearchableResults.length === 0 ? (
    <div className="max-h-[min(60vh,32rem)] overflow-y-auto">
      <p className="px-4 py-5 text-sm text-destructive">
        {searchQuery.error.message}
      </p>
    </div>
  ) : visibleSearchableResults.length === 0 ? (
    <div className="max-h-[min(60vh,32rem)] overflow-y-auto">
      <p className="px-4 py-5 text-sm text-muted-foreground">
        No {scopeChannel ? "messages" : "matches"} for{" "}
        <span className="font-semibold">{trimmedQuery}</span>
        {scopeLabel ? (
          <>
            {" "}
            in <span className="font-semibold">{scopeLabel}</span>
          </>
        ) : null}
        .
      </p>
    </div>
  ) : (
    <div
      className="buzz-search-scrollbar max-h-[min(60vh,32rem)] overflow-y-auto"
      data-testid="search-results-list"
      role="listbox"
    >
      <div className="p-1.5">
        {renderSearchResultSections(searchResultSections)}
      </div>
    </div>
  );
  return (
    <div className={cn("relative", className)}>
      <Dialog open={isOpen} onOpenChange={handleSearchOpenChange}>
        <button
          aria-label="Search everything"
          className={
            isIconVariant
              ? "group/search flex size-6 items-center justify-center rounded p-1 text-sidebar-foreground/50 transition-colors hover:bg-sidebar-border/35 hover:text-sidebar-foreground focus-visible:bg-sidebar-border/35 focus-visible:text-sidebar-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-sidebar-ring"
              : "group/search flex h-8 w-full items-center gap-2 rounded-md bg-sidebar-border/35 px-2 text-left text-sm text-sidebar-foreground/55 transition-colors duration-150 ease-out hover:bg-sidebar-border/35 hover:text-sidebar-foreground focus-visible:bg-sidebar-border/35 focus-visible:text-sidebar-foreground focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-sidebar-ring"
          }
          data-testid="open-search"
          onClick={() => openSearchDialog(null)}
          ref={triggerRef}
          title="Search everything"
          type="button"
        >
          <Search
            className={
              isIconVariant
                ? "h-4 w-4 shrink-0"
                : "h-4 w-4 shrink-0 text-sidebar-foreground/45 transition-colors duration-150 ease-out group-hover/search:text-sidebar-foreground/65 group-focus-visible/search:text-sidebar-foreground"
            }
          />
          {isIconVariant ? null : (
            <>
              <span
                className={cn(
                  "min-w-0 flex-1 truncate transition-colors duration-150 ease-out",
                  query
                    ? "text-sidebar-foreground"
                    : "text-sidebar-foreground/55",
                )}
              >
                {query || "Search everything"}
              </span>
              <kbd className="shrink-0 text-2xs text-sidebar-foreground/45">
                &#x2318;K
              </kbd>
            </>
          )}
        </button>
        <DialogContent
          aria-busy={isSearchLoading && visibleSearchableResults.length === 0}
          className="mt-[18vh] max-w-2xl self-start gap-0 overflow-hidden rounded-2xl p-0 shadow-2xl"
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
          <DialogTitle className="sr-only">
            {scopeLabel ? `Search in ${scopeLabel}` : "Search everything"}
          </DialogTitle>
          <SearchDialogInputRow
            currentScopeActionLabel={currentScopeActionLabel}
            inputRef={dialogInputRef}
            onActivateCurrentScope={
              currentChannel ? activateCurrentChannelScope : undefined
            }
            onChange={(nextQuery) => {
              setQuery(nextQuery);
              setSelectedMenuIndex(0);
            }}
            onKeyDown={handleDialogInputKeyDown}
            onRemoveScope={removeChannelScope}
            query={query}
            scopeLabel={scopeLabel}
          />
          {searchResultContent}
        </DialogContent>
      </Dialog>
    </div>
  );
}
