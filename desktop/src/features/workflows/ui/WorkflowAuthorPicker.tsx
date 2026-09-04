import { Check, LoaderCircle, Search } from "lucide-react";
import * as React from "react";

import { useChannelMembersQuery } from "@/features/channels/hooks";
import { useRelayMembersQuery } from "@/features/community-members/hooks";
import {
  useFlattenedUserSearchResults,
  useInfiniteUserSearchQuery,
  useUsersBatchQuery,
} from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { cn } from "@/shared/lib/cn";
import { truncatePubkey } from "@/shared/lib/pubkey";
import { Input } from "@/shared/ui/input";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import {
  enrichAuthorCandidates,
  filterAuthorCandidatePage,
  mergeAuthorCandidateSources,
  nextWorkflowAuthorIndex,
  parseDirectAuthorInput,
  type WorkflowAuthorCandidate,
} from "./workflowAuthorCandidates";

const PAGE_SIZE = 50;

export function WorkflowAuthorPicker({
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
  onChange: (pubkey: string) => void;
  onEscape?: () => void;
  value: string;
}) {
  const pickerRef = React.useRef<HTMLDivElement>(null);
  const optionRefs = React.useRef(new Map<string, HTMLButtonElement>());
  const [query, setQuery] = React.useState("");
  const [activeIndex, setActiveIndex] = React.useState<number | null>(null);
  const [columnCount, setColumnCount] = React.useState(2);
  const trimmedQuery = query.trim();
  const deferredQuery = React.useDeferredValue(trimmedQuery);
  const normalizedValue = parseDirectAuthorInput(value);
  const channelMembersQuery = useChannelMembersQuery(channelId ?? null);
  const relayMembersQuery = useRelayMembersQuery(true);
  const directoryQuery = useInfiniteUserSearchQuery(deferredQuery, {
    allowEmpty: true,
    limit: PAGE_SIZE,
  });
  const directoryResults = useFlattenedUserSearchResults(directoryQuery.data);
  const directPubkey = parseDirectAuthorInput(deferredQuery);

  const baseCandidates = React.useMemo(
    () =>
      mergeAuthorCandidateSources([
        normalizedValue ? [{ pubkey: normalizedValue }] : [],
        directPubkey ? [{ pubkey: directPubkey }] : [],
        channelMembersQuery.data ?? [],
        relayMembersQuery.data ?? [],
        directoryResults,
      ]),
    [
      channelMembersQuery.data,
      directPubkey,
      directoryResults,
      normalizedValue,
      relayMembersQuery.data,
    ],
  );
  const candidatePage = React.useMemo(
    () =>
      filterAuthorCandidatePage(
        baseCandidates,
        deferredQuery,
        directPubkey,
        PAGE_SIZE,
      ),
    [baseCandidates, deferredQuery, directPubkey],
  );
  const profileQuery = useUsersBatchQuery(
    candidatePage.map(({ pubkey }) => pubkey),
  );
  const candidates = React.useMemo(
    () =>
      enrichAuthorCandidates(candidatePage, profileQuery.data?.profiles ?? {}),
    [candidatePage, profileQuery.data?.profiles],
  );
  const visibleCandidates = candidates;
  const listId = `${id}-list`;

  React.useEffect(() => {
    if (activeIndex !== null && activeIndex >= visibleCandidates.length) {
      setActiveIndex(
        visibleCandidates.length > 0 ? visibleCandidates.length - 1 : null,
      );
    }
  }, [activeIndex, visibleCandidates.length]);

  React.useEffect(() => {
    const picker = pickerRef.current;
    if (!picker) return;
    const updateColumnCount = () =>
      setColumnCount(picker.clientWidth >= 544 ? 3 : 2);
    updateColumnCount();
    const observer = new ResizeObserver(updateColumnCount);
    observer.observe(picker);
    return () => observer.disconnect();
  }, []);

  React.useEffect(() => {
    if (activeIndex === null) return;
    const candidate = visibleCandidates[activeIndex];
    if (!candidate) return;
    optionRefs.current
      .get(candidate.pubkey)
      ?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [activeIndex, visibleCandidates]);

  const loading =
    (channelMembersQuery.isLoading ||
      relayMembersQuery.isLoading ||
      directoryQuery.isLoading) &&
    visibleCandidates.length === 0;
  const failed =
    channelMembersQuery.isError ||
    relayMembersQuery.isError ||
    directoryQuery.isError;

  function moveActive(delta: number) {
    setActiveIndex((current) =>
      nextWorkflowAuthorIndex(current, delta, visibleCandidates.length),
    );
  }

  return (
    <div
      className="flex h-full min-h-0 flex-1 flex-col overflow-hidden rounded-lg border border-border/70 bg-background/35"
      data-testid="workflow-author-picker"
      ref={pickerRef}
    >
      <div className="relative shrink-0 border-b border-border/70">
        <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
        <Input
          aria-activedescendant={
            activeIndex !== null && visibleCandidates[activeIndex]
              ? `${listId}-${visibleCandidates[activeIndex].pubkey}`
              : undefined
          }
          aria-controls={listId}
          aria-label="Search authors or paste a public key"
          aria-expanded="true"
          autoCapitalize="none"
          autoComplete="off"
          autoCorrect="off"
          className="h-11 rounded-none border-0 bg-transparent pl-9 focus-visible:ring-0"
          data-workflow-filter-picker-search="true"
          disabled={disabled}
          id={id}
          onChange={(event) => {
            setQuery(event.target.value);
            setActiveIndex(null);
          }}
          onKeyDown={(event) => {
            const currentQuery = event.currentTarget.value.trim();
            const delta =
              event.key === "ArrowDown"
                ? columnCount
                : event.key === "ArrowUp"
                  ? -columnCount
                  : event.key === "ArrowRight"
                    ? 1
                    : event.key === "ArrowLeft"
                      ? -1
                      : 0;
            if (delta) {
              event.preventDefault();
              moveActive(delta);
            } else if (
              event.key === "Enter" &&
              currentQuery === deferredQuery &&
              visibleCandidates[activeIndex ?? 0]
            ) {
              event.preventDefault();
              const candidate = visibleCandidates[activeIndex ?? 0];
              onChange(
                candidate.pubkey === normalizedValue ? "" : candidate.pubkey,
              );
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
          placeholder="Search people or paste a public key…"
          role="combobox"
          spellCheck={false}
          value={query}
        />
      </div>

      <div
        aria-label="Authors"
        className={cn(
          "grid min-h-0 flex-1 gap-2 overflow-y-auto overscroll-contain p-2",
          columnCount === 3 ? "grid-cols-3" : "grid-cols-2",
        )}
        data-testid="workflow-author-picker-results"
        id={listId}
        onScroll={(event) => {
          const list = event.currentTarget;
          if (
            list.scrollHeight - list.scrollTop - list.clientHeight < 64 &&
            directoryQuery.hasNextPage &&
            !directoryQuery.isFetchingNextPage
          ) {
            void directoryQuery.fetchNextPage();
          }
        }}
        role="listbox"
      >
        {visibleCandidates.map((candidate, index) => (
          <AuthorOption
            active={activeIndex !== null && index === activeIndex}
            candidate={candidate}
            disabled={disabled}
            id={`${listId}-${candidate.pubkey}`}
            key={candidate.pubkey}
            onSelect={() => {
              setActiveIndex(null);
              onChange(
                candidate.pubkey === normalizedValue ? "" : candidate.pubkey,
              );
            }}
            optionRef={(node) => {
              if (node) optionRefs.current.set(candidate.pubkey, node);
              else optionRefs.current.delete(candidate.pubkey);
            }}
            selected={candidate.pubkey === normalizedValue}
          />
        ))}
        {loading ? (
          <p
            className="col-span-full flex items-center justify-center gap-2 px-3 py-8 text-sm text-muted-foreground"
            role="status"
          >
            <LoaderCircle className="h-4 w-4 animate-spin" /> Loading authors…
          </p>
        ) : visibleCandidates.length === 0 ? (
          <p className="col-span-full px-3 py-8 text-center text-sm text-muted-foreground">
            {failed ? "Couldn’t load authors." : "No authors found."}
          </p>
        ) : null}
        {failed ? (
          <button
            className="col-span-full rounded-md px-3 py-2 text-xs font-medium text-muted-foreground hover:bg-muted/45 hover:text-foreground"
            onClick={() => {
              void channelMembersQuery.refetch();
              void relayMembersQuery.refetch();
              void directoryQuery.refetch();
            }}
            type="button"
          >
            Couldn’t load all authors. Retry
          </button>
        ) : null}
        {directoryQuery.hasNextPage ? (
          <button
            className="col-span-full rounded-md px-3 py-2 text-xs font-medium text-muted-foreground hover:bg-muted/45 hover:text-foreground"
            disabled={disabled || directoryQuery.isFetchingNextPage}
            onClick={() => void directoryQuery.fetchNextPage()}
            type="button"
          >
            {directoryQuery.isFetchingNextPage
              ? "Loading more…"
              : "Load more authors"}
          </button>
        ) : null}
      </div>
    </div>
  );
}

function AuthorOption({
  active,
  candidate,
  disabled,
  id,
  onSelect,
  optionRef,
  selected,
}: {
  active: boolean;
  candidate: WorkflowAuthorCandidate;
  disabled?: boolean;
  id: string;
  onSelect: () => void;
  optionRef: (node: HTMLButtonElement | null) => void;
  selected: boolean;
}) {
  const label = resolveUserLabel({
    fallbackName: candidate.displayName,
    profiles: {
      [candidate.pubkey]: {
        displayName: candidate.displayName,
        avatarUrl: candidate.avatarUrl,
        nip05Handle: candidate.nip05Handle,
        ownerPubkey: candidate.ownerPubkey,
        isAgent: candidate.isAgent,
      },
    },
    pubkey: candidate.pubkey,
  });
  return (
    <button
      aria-selected={selected}
      className={cn(
        "relative flex min-h-24 min-w-0 flex-col items-center justify-center gap-2 rounded-lg border bg-muted/20 p-3 text-center transition-colors",
        selected
          ? "border-primary bg-primary/10"
          : "border-border hover:bg-accent",
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
      <UserAvatar
        avatarUrl={candidate.avatarUrl}
        displayName={label}
        shape={candidate.isAgent ? "squircle" : "circle"}
        size="md"
      />
      <span className="min-w-0 max-w-full">
        <span className="block truncate text-sm font-medium">{label}</span>
        <span className="block truncate text-xs text-muted-foreground">
          {truncatePubkey(candidate.pubkey)}
        </span>
      </span>
      <Check
        className={cn(
          "absolute right-2 top-2 h-4 w-4",
          !selected && "opacity-0",
        )}
      />
    </button>
  );
}
