import { useInfiniteQuery, useQueries, useQuery } from "@tanstack/react-query";
import { Check, LoaderCircle, Search } from "lucide-react";
import * as React from "react";

import { parseChannelWindowResponse } from "@/features/messages/lib/channelWindowResponse";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import {
  resolveUserLabel,
  type UserProfileLookup,
} from "@/features/profile/lib/identity";
import { useSearchMessagesQuery } from "@/features/search/hooks";
import { getChannelWindowEvents } from "@/shared/api/channelWindow";
import { getEventById } from "@/shared/api/tauri";
import type { ChannelPageCursor } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Input } from "@/shared/ui/input";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import {
  mergeMessageCandidateSources,
  normalizeMessageEventId,
  type WorkflowMessageCandidate,
  validatedWorkflowMessageCandidate,
  validateWorkflowMessageSearchResults,
} from "./workflowMessageCandidates";

const PAGE_SIZE = 25;

function truncateContent(content: string | null): string {
  const normalized = content?.trim().replaceAll(/\s+/g, " ") ?? "";
  if (!normalized) return "No message body";
  return normalized.length > 120
    ? `${normalized.slice(0, 117)}...`
    : normalized;
}

function formatTimestamp(unixSeconds: number | null): string | null {
  if (unixSeconds === null) return null;
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(new Date(unixSeconds * 1_000));
}

export function WorkflowMessagePicker({
  channelId,
  disabled,
  id,
  onChange,
  onEscape,
  value,
}: {
  channelId?: string | null;
  disabled?: boolean;
  id: string;
  onChange: (messageId: string) => void;
  onEscape?: () => void;
  value: string;
}) {
  const optionRefs = React.useRef(new Map<string, HTMLButtonElement>());
  const [query, setQuery] = React.useState("");
  const [activeIndex, setActiveIndex] = React.useState<number | null>(null);
  const trimmedQuery = query.trim();
  const deferredQuery = React.useDeferredValue(trimmedQuery);
  const normalizedQuery = deferredQuery.toLowerCase();
  const selectedId = normalizeMessageEventId(value);
  const directId = normalizeMessageEventId(query);
  const lookupId = directId ?? selectedId;

  const historyQuery = useInfiniteQuery({
    enabled: Boolean(channelId),
    initialPageParam: null as ChannelPageCursor | null,
    queryKey: ["workflow-message-picker", channelId],
    queryFn: async ({ pageParam }) => {
      if (!channelId) throw new Error("Choose a channel first.");
      return parseChannelWindowResponse(
        await getChannelWindowEvents(channelId, pageParam, PAGE_SIZE),
        channelId,
        pageParam,
      );
    },
    getNextPageParam: (lastPage) => lastPage.nextCursor ?? undefined,
    staleTime: 30_000,
  });
  const searchQuery = useSearchMessagesQuery(deferredQuery, {
    channelId: channelId ?? undefined,
    enabled: Boolean(channelId && normalizedQuery && !directId),
    limit: 30,
    minimumQueryLength: 1,
  });
  const exactQuery = useQuery({
    enabled: Boolean(channelId && lookupId),
    queryKey: ["workflow-message-picker-exact", channelId, lookupId],
    queryFn: () => getEventById(lookupId ?? ""),
    retry: false,
    staleTime: 60_000,
  });

  const selectedFallback = selectedId
    ? [{ id: selectedId, pubkey: null, content: null, createdAt: null }]
    : [];
  const directFallback = directId
    ? [{ id: directId, pubkey: null, content: null, createdAt: null }]
    : [];
  const historyCandidates = React.useMemo(
    () =>
      (historyQuery.data?.pages ?? []).flatMap((page) =>
        page.rows.flatMap(({ event }) => {
          const candidate = channelId
            ? validatedWorkflowMessageCandidate(event, { channelId })
            : null;
          return candidate ? [candidate] : [];
        }),
      ),
    [channelId, historyQuery.data?.pages],
  );
  const searchHitIds = React.useMemo(
    () => [
      ...new Set(
        (searchQuery.data?.hits ?? []).flatMap((hit) => {
          const eventId = normalizeMessageEventId(hit.eventId);
          return eventId ? [eventId] : [];
        }),
      ),
    ],
    [searchQuery.data?.hits],
  );
  const searchEventQueries = useQueries({
    queries: searchHitIds.map((eventId) => ({
      enabled: Boolean(channelId),
      queryKey: ["workflow-message-picker-search-event", channelId, eventId],
      queryFn: () => getEventById(eventId),
      retry: false,
      staleTime: 60_000,
    })),
  });
  const searchCandidates = validateWorkflowMessageSearchResults(
    searchHitIds.map((requestedId, index) => ({
      requestedId,
      event: searchEventQueries[index]?.data,
    })),
    channelId ?? "",
  );
  const exactCandidate = React.useMemo(() => {
    if (!channelId || !lookupId) return null;
    return validatedWorkflowMessageCandidate(exactQuery.data, {
      channelId,
      requestedId: lookupId,
    });
  }, [channelId, exactQuery.data, lookupId]);
  const allCandidates = React.useMemo(() => {
    // Preserve the relay-provided history/search order. Exact lookups and raw-ID
    // fallbacks only fill gaps; selecting an existing row must not move it.
    return mergeMessageCandidateSources([
      historyCandidates,
      searchCandidates,
      exactCandidate ? [exactCandidate] : [],
      selectedFallback,
      directFallback,
    ]);
  }, [
    directFallback,
    exactCandidate,
    historyCandidates,
    searchCandidates,
    selectedFallback,
  ]);
  const visibleCandidates = React.useMemo(() => {
    if (directId) return allCandidates.filter(({ id }) => id === directId);
    if (!normalizedQuery) return allCandidates;
    return allCandidates.filter(
      (candidate) =>
        candidate.id.includes(normalizedQuery) ||
        candidate.content?.toLowerCase().includes(normalizedQuery),
    );
  }, [allCandidates, directId, normalizedQuery]);
  const profilePubkeys = React.useMemo(
    () => [
      ...new Set(
        visibleCandidates.flatMap(({ pubkey }) => (pubkey ? [pubkey] : [])),
      ),
    ],
    [visibleCandidates],
  );
  const profilesQuery = useUsersBatchQuery(profilePubkeys);
  const listId = `${id}-list`;

  React.useEffect(() => {
    if (activeIndex !== null && activeIndex >= visibleCandidates.length) {
      setActiveIndex(
        visibleCandidates.length > 0 ? visibleCandidates.length - 1 : null,
      );
    }
  }, [activeIndex, visibleCandidates.length]);
  React.useEffect(() => {
    if (activeIndex === null) return;
    const candidate = visibleCandidates[activeIndex];
    if (!candidate) return;
    optionRefs.current
      .get(candidate.id)
      ?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [activeIndex, visibleCandidates]);

  const searchEventsFetching = searchEventQueries.some(
    ({ isFetching }) => isFetching,
  );
  const searchEventsFailed = searchEventQueries.some(({ isError }) => isError);
  const loading =
    (historyQuery.isLoading ||
      searchQuery.isFetching ||
      searchEventsFetching ||
      exactQuery.isFetching) &&
    visibleCandidates.length === 0;
  const failed =
    historyQuery.isError ||
    searchQuery.isError ||
    searchEventsFailed ||
    exactQuery.isError;
  const invalidDirectResult = Boolean(
    directId && lookupId === directId && exactQuery.data && !exactCandidate,
  );

  return (
    <div
      className="flex h-full min-h-0 flex-1 flex-col overflow-hidden rounded-lg border border-border/70 bg-background/35"
      data-testid="workflow-message-picker"
    >
      <div className="relative shrink-0 border-b border-border/70">
        <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          aria-activedescendant={
            activeIndex !== null && visibleCandidates[activeIndex]
              ? `${listId}-${visibleCandidates[activeIndex].id}`
              : undefined
          }
          aria-controls={listId}
          aria-label="Search messages or paste a message ID"
          aria-expanded="true"
          autoCapitalize="none"
          autoComplete="off"
          autoCorrect="off"
          className="h-11 rounded-none border-0 bg-transparent pl-9 focus-visible:ring-0"
          data-workflow-filter-picker-search="true"
          disabled={disabled || !channelId}
          id={id}
          onChange={(event) => {
            setQuery(event.target.value);
            setActiveIndex(null);
          }}
          onKeyDown={(event) => {
            const currentQuery = event.currentTarget.value.trim();
            if (event.key === "ArrowDown" || event.key === "ArrowUp") {
              event.preventDefault();
              if (visibleCandidates.length > 0) {
                setActiveIndex((current) => {
                  const startingIndex = current ?? -1;
                  return (
                    (startingIndex +
                      (event.key === "ArrowDown" ? 1 : -1) +
                      visibleCandidates.length) %
                    visibleCandidates.length
                  );
                });
              }
            } else if (event.key === "Enter") {
              const currentDirectId = normalizeMessageEventId(currentQuery);
              if (currentQuery !== deferredQuery && !currentDirectId) return;
              const candidate = currentDirectId
                ? allCandidates.find(({ id }) => id === currentDirectId)
                : visibleCandidates[activeIndex ?? 0];
              if (!candidate) return;
              event.preventDefault();
              if (invalidDirectResult) return;
              onChange(candidate.id === selectedId ? "" : candidate.id);
            } else if (event.key === "Escape") {
              event.preventDefault();
              event.stopPropagation();
              if (query) {
                setQuery("");
                setActiveIndex(null);
              } else {
                onEscape?.();
              }
            }
          }}
          placeholder={
            channelId
              ? "Search messages or paste a message ID…"
              : "Choose a channel first"
          }
          role="combobox"
          spellCheck={false}
          value={query}
        />
        {historyQuery.isFetching ||
        searchQuery.isFetching ||
        searchEventsFetching ||
        exactQuery.isFetching ? (
          <LoaderCircle
            aria-label="Loading messages"
            className="absolute right-3 top-1/2 h-4 w-4 -translate-y-1/2 animate-spin text-muted-foreground"
          />
        ) : null}
      </div>
      {invalidDirectResult ? (
        <p className="shrink-0 border-b border-border/70 px-3 py-2 text-xs text-destructive">
          That message is not available in this channel.
        </p>
      ) : null}
      <div
        aria-label="Messages"
        className="min-h-0 flex-1 space-y-1 overflow-y-auto overscroll-contain p-2"
        data-testid="workflow-message-picker-results"
        id={listId}
        onScroll={(event) => {
          const list = event.currentTarget;
          if (
            !normalizedQuery &&
            list.scrollHeight - list.scrollTop - list.clientHeight < 64 &&
            historyQuery.hasNextPage &&
            !historyQuery.isFetchingNextPage
          ) {
            void historyQuery.fetchNextPage();
          }
        }}
        role="listbox"
      >
        {visibleCandidates.map((candidate, index) => (
          <MessageOption
            active={activeIndex !== null && index === activeIndex}
            candidate={candidate}
            disabled={disabled}
            id={`${listId}-${candidate.id}`}
            key={candidate.id}
            onSelect={() => {
              setActiveIndex(null);
              if (!invalidDirectResult) {
                onChange(candidate.id === selectedId ? "" : candidate.id);
              }
            }}
            optionRef={(node) => {
              if (node) optionRefs.current.set(candidate.id, node);
              else optionRefs.current.delete(candidate.id);
            }}
            profiles={profilesQuery.data?.profiles}
            selected={candidate.id === selectedId}
          />
        ))}
        {loading ? (
          <p
            className="flex items-center justify-center gap-2 px-3 py-8 text-sm text-muted-foreground"
            role="status"
          >
            <LoaderCircle className="h-4 w-4 animate-spin" /> Loading messages…
          </p>
        ) : visibleCandidates.length === 0 ? (
          <p className="px-3 py-8 text-center text-sm text-muted-foreground">
            {failed
              ? "Couldn’t load messages."
              : normalizedQuery
                ? "No messages found."
                : "No messages yet."}
          </p>
        ) : null}
        {failed ? (
          <button
            className="w-full rounded-md px-3 py-2 text-xs font-medium text-muted-foreground hover:bg-muted/45 hover:text-foreground"
            onClick={() => {
              void historyQuery.refetch();
              void searchQuery.refetch();
              for (const searchEventQuery of searchEventQueries) {
                void searchEventQuery.refetch();
              }
              void exactQuery.refetch();
            }}
            type="button"
          >
            Couldn’t load all messages. Retry
          </button>
        ) : null}
        {!normalizedQuery && historyQuery.hasNextPage ? (
          <button
            className="w-full rounded-md px-3 py-2 text-xs font-medium text-muted-foreground hover:bg-muted/45 hover:text-foreground"
            disabled={disabled || historyQuery.isFetchingNextPage}
            onClick={() => void historyQuery.fetchNextPage()}
            type="button"
          >
            {historyQuery.isFetchingNextPage
              ? "Loading older messages…"
              : "Load older messages"}
          </button>
        ) : null}
      </div>
    </div>
  );
}

function MessageOption({
  active,
  candidate,
  disabled,
  id,
  onSelect,
  optionRef,
  profiles,
  selected,
}: {
  active: boolean;
  candidate: WorkflowMessageCandidate;
  disabled?: boolean;
  id: string;
  onSelect: () => void;
  optionRef: (node: HTMLButtonElement | null) => void;
  profiles?: UserProfileLookup;
  selected: boolean;
}) {
  const author = candidate.pubkey
    ? resolveUserLabel({ profiles, pubkey: candidate.pubkey })
    : "Selected message";
  const profile = candidate.pubkey
    ? profiles?.[candidate.pubkey.toLowerCase()]
    : undefined;
  const timestamp = formatTimestamp(candidate.createdAt);
  return (
    <button
      aria-selected={selected}
      className={cn(
        "relative flex min-w-0 w-full items-start gap-2.5 overflow-hidden rounded-md border px-3 py-2.5 text-left transition-colors",
        selected
          ? "border-foreground/40 bg-muted/70"
          : "border-transparent hover:border-border hover:bg-muted/45",
        active && "ring-1 ring-ring",
      )}
      disabled={disabled}
      id={id}
      onClick={onSelect}
      ref={optionRef}
      role="option"
      tabIndex={-1}
      type="button"
    >
      {candidate.pubkey ? (
        <UserAvatar
          avatarUrl={profile?.avatarUrl ?? null}
          displayName={author}
          shape={profile?.isAgent === true ? "squircle" : "circle"}
          size="sm"
        />
      ) : null}
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-2 text-xs text-muted-foreground">
          <span className="truncate font-medium text-foreground">{author}</span>
          {timestamp ? (
            <span className="ml-auto shrink-0">{timestamp}</span>
          ) : null}
        </span>
        <span className="mt-1 block min-w-0 break-words text-sm leading-5 text-foreground [overflow-wrap:anywhere]">
          {truncateContent(candidate.content)}
        </span>
        <span className="mt-1 block font-mono text-2xs text-muted-foreground">
          {candidate.id.slice(0, 12)}…{candidate.id.slice(-8)}
        </span>
      </span>
      <Check
        className={cn("mt-1 h-4 w-4 shrink-0", !selected && "opacity-0")}
      />
    </button>
  );
}
