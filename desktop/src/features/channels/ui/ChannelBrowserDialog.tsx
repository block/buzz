import * as React from "react";
import {
  ArrowLeft,
  Compass,
  Plus,
  Search,
  X,
  type LucideIcon,
} from "lucide-react";

import type { Channel } from "@/shared/api/types";
import { scoreChannelMatch } from "@/features/channels/lib/channelSearchScore";
import {
  type ChannelSortMode,
  sortChannelsForSidebar,
} from "@/features/sidebar/lib/channelSortPreference";
import { ListSortDescending } from "@/shared/ui/icons";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import {
  MODAL_SEARCH_INPUT_CLASS,
  MODAL_SEARCH_SHELL_CLASS,
} from "@/shared/ui/modalSearchStyles";
import { Tabs, TabsList, TabsTrigger } from "@/shared/ui/tabs";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";

import {
  type CreateChannelInput,
  useCreateChannelForm,
} from "@/features/sidebar/lib/useCreateChannelForm";
import {
  CREATE_CHANNEL_FORM_ID,
  CreateChannelFormFields,
  CreateChannelFormFooter,
} from "@/features/sidebar/ui/CreateChannelFormFields";

type BrowserTab = "all" | "joined" | "archived";
type ChannelSort = ChannelSortMode | "members";

const CHANNEL_SORT_OPTIONS: { label: string; value: ChannelSort }[] = [
  { label: "Alphabetical", value: "alpha" },
  { label: "Recent", value: "recent" },
  { label: "Most members", value: "members" },
];

function BrowseState({
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
        <Icon className="h-4 w-4" />
      </div>
      <p className="mt-4 text-base font-semibold tracking-tight">{title}</p>
      <p className="mt-2 max-w-md text-sm text-muted-foreground">
        {description}
      </p>
    </div>
  );
}

type ChannelBrowserDialogProps = {
  channels: Channel[];
  channelTypeFilter?: "stream" | "forum";
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onJoinChannel: (channelId: string) => Promise<void>;
  onSelectChannel: (channelId: string) => void;
  /**
   * Create a new channel/forum from within the browser. When provided, the
   * dialog surfaces a "Create …" affordance (Are.na style) so search and
   * create live behind a single entry point.
   */
  onCreateChannel?: (input: CreateChannelInput) => Promise<void>;
  isCreatingChannel?: boolean;
};

export function ChannelBrowserDialog({
  channels,
  channelTypeFilter,
  open,
  onOpenChange,
  onJoinChannel,
  onSelectChannel,
  onCreateChannel,
  isCreatingChannel = false,
}: ChannelBrowserDialogProps) {
  const [query, setQuery] = React.useState("");
  const [activeTab, setActiveTab] = React.useState<BrowserTab>("all");
  const [sort, setSort] = React.useState<ChannelSort>("alpha");
  const [selectedIndex, setSelectedIndex] = React.useState<number | null>(null);
  const [joiningChannelId, setJoiningChannelId] = React.useState<string | null>(
    null,
  );
  const [mode, setMode] = React.useState<"browse" | "create">("browse");
  const [createInitialName, setCreateInitialName] = React.useState("");
  const inputRef = React.useRef<HTMLInputElement>(null);
  const tabListRef = React.useRef<HTMLDivElement>(null);
  const tabTriggerRefs = React.useRef<
    Record<BrowserTab, HTMLButtonElement | null>
  >({
    all: null,
    joined: null,
    archived: null,
  });
  const [tabIndicator, setTabIndicator] = React.useState({
    left: 0,
    width: 0,
  });
  const deferredQuery = React.useDeferredValue(query.trim().toLowerCase());
  const trimmedQuery = query.trim();
  // Immediate (non-deferred) lowercased query. The create row's visibility
  // (via hasExactMatch) and its label both read from the live query so they
  // can never disagree for a frame while the fuzzy filter catches up.
  const normalizedQuery = trimmedQuery.toLowerCase();

  const isForumMode = channelTypeFilter === "forum";
  const canCreate = Boolean(onCreateChannel);
  const createKind = isForumMode ? "forum" : "stream";
  const browseTitle = isForumMode ? "Add a forum" : "Browse channels";
  const searchPlaceholder = canCreate
    ? isForumMode
      ? "Search or create a forum"
      : "Search or create a channel"
    : isForumMode
      ? "Search forums by name or description"
      : "Search channels by name or description";
  const entityLabel = isForumMode ? "forum" : "channel";

  const noopCreate = React.useCallback(async () => {}, []);
  const createForm = useCreateChannelForm({
    channelKind: createKind,
    active: open && mode === "create",
    initialName: createInitialName,
    isCreating: isCreatingChannel,
    onCreate: onCreateChannel ?? noopCreate,
    onCreated: () => onOpenChange(false),
  });

  // Fuzzy match score per channel id for the current query, so both filtering
  // and relevance-ordering share one source of truth. Empty when no query.
  const matchScoreById = React.useMemo(() => {
    const scores = new Map<string, number>();
    if (deferredQuery.length === 0) return scores;
    for (const channel of channels) {
      const score = scoreChannelMatch(channel, deferredQuery);
      if (score !== null) scores.set(channel.id, score);
    }
    return scores;
  }, [channels, deferredQuery]);

  const matchingChannels = React.useMemo(() => {
    const filtered = channels.filter(
      (channel) =>
        channel.channelType !== "dm" &&
        (channel.archivedAt
          ? channel.isMember
          : channel.visibility === "open" || channel.isMember) &&
        (channelTypeFilter ? channel.channelType === channelTypeFilter : true),
    );

    if (deferredQuery.length === 0) {
      return filtered;
    }

    return filtered.filter((channel) => matchScoreById.has(channel.id));
  }, [channels, channelTypeFilter, deferredQuery, matchScoreById]);

  const currentChannels = React.useMemo(
    () => matchingChannels.filter((channel) => channel.archivedAt === null),
    [matchingChannels],
  );

  const joinedChannels = React.useMemo(
    () => currentChannels.filter((channel) => channel.isMember),
    [currentChannels],
  );

  const archivedChannels = React.useMemo(
    () => matchingChannels.filter((channel) => channel.archivedAt !== null),
    [matchingChannels],
  );

  const visibleChannels =
    activeTab === "archived"
      ? archivedChannels
      : activeTab === "joined"
        ? joinedChannels
        : matchingChannels;

  const isSearching = deferredQuery.length > 0;

  const orderedVisibleChannels = React.useMemo(() => {
    const sorted =
      sort === "members"
        ? [...visibleChannels].sort(
            (a, b) =>
              b.memberCount - a.memberCount ||
              a.name.localeCompare(b.name, undefined, {
                sensitivity: "base",
              }),
          )
        : sortChannelsForSidebar(visibleChannels, sort);

    if (!isSearching) return sorted;

    return sorted.sort(
      (a, b) =>
        (matchScoreById.get(a.id) ?? Number.POSITIVE_INFINITY) -
        (matchScoreById.get(b.id) ?? Number.POSITIVE_INFINITY),
    );
  }, [isSearching, matchScoreById, sort, visibleChannels]);

  const selectedSortLabel =
    CHANNEL_SORT_OPTIONS.find((option) => option.value === sort)?.label ??
    "Alphabetical";

  const allTabLabel = isForumMode ? "All forums" : "All channels";

  // Whether an exact name match already exists — if so we don't offer to
  // create a duplicate, mirroring how you'd never make two "#general"s.
  const hasExactMatch = React.useMemo(
    () =>
      channels.some(
        (channel) =>
          channel.channelType !== "dm" &&
          channel.name.toLowerCase() === normalizedQuery &&
          (channelTypeFilter
            ? channel.channelType === channelTypeFilter
            : true),
      ),
    [channels, channelTypeFilter, normalizedQuery],
  );

  // The pinned create row (Are.na style) appears for any non-empty query that
  // isn't already an exact channel name — covering both partial-match and
  // no-match cases, so a dedicated empty-state button would be redundant.
  // The create row is present from the moment the dialog opens (so it's clear
  // you can browse *or* create), then specializes to "Create «query»" as you
  // type. It only hides when the query is an exact match for an existing name
  // — creating a duplicate "#general" makes no sense.
  const showCreateRow = canCreate && !hasExactMatch;

  // The create row participates in keyboard navigation as a virtual item so
  // arrow keys reach it and Enter activates it — not just Tab. It's rendered
  // pinned at the top, so it takes nav index 0 and channels shift down by one,
  // keeping keyboard order identical to visual order.
  const channelNavOffset = showCreateRow ? 1 : 0;
  const createRowIndex = showCreateRow ? 0 : null;
  const navItemCount = orderedVisibleChannels.length + channelNavOffset;
  const isCreateRowSelected =
    createRowIndex !== null && selectedIndex === createRowIndex;

  const updateTabIndicator = React.useCallback(() => {
    const list = tabListRef.current;
    const trigger = tabTriggerRefs.current[activeTab];

    if (!open || mode !== "browse" || !list || !trigger) {
      return;
    }

    const nextIndicator = {
      left: trigger.offsetLeft,
      width: trigger.offsetWidth,
    };

    setTabIndicator((current) =>
      Math.abs(current.left - nextIndicator.left) < 0.5 &&
      Math.abs(current.width - nextIndicator.width) < 0.5
        ? current
        : nextIndicator,
    );
  }, [activeTab, mode, open]);

  React.useLayoutEffect(() => {
    updateTabIndicator();

    if (!open || mode !== "browse") {
      return;
    }

    let isCancelled = false;
    const updateIfActive = () => {
      if (!isCancelled) {
        updateTabIndicator();
      }
    };
    const frameId = window.requestAnimationFrame(updateIfActive);
    const observer = new ResizeObserver(updateTabIndicator);
    const list = tabListRef.current;

    void document.fonts.ready.then(updateIfActive);

    if (list) {
      observer.observe(list);
    }

    for (const trigger of Object.values(tabTriggerRefs.current)) {
      if (trigger) {
        observer.observe(trigger);
      }
    }

    return () => {
      isCancelled = true;
      window.cancelAnimationFrame(frameId);
      observer.disconnect();
    };
  }, [mode, open, updateTabIndicator]);

  React.useEffect(() => {
    if (!open) {
      setQuery("");
      setActiveTab("all");
      setSort("alpha");
      setSelectedIndex(null);
      setJoiningChannelId(null);
      setMode("browse");
      setCreateInitialName("");
      return;
    }
  }, [open]);

  React.useEffect(() => {
    setSelectedIndex((current) => {
      if (current === null || navItemCount === 0) {
        return null;
      }

      return Math.min(current, navItemCount - 1);
    });
  }, [navItemCount]);

  async function handleJoin(channelId: string) {
    setJoiningChannelId(channelId);

    try {
      await onJoinChannel(channelId);
      onOpenChange(false);
      onSelectChannel(channelId);
    } catch {
      setJoiningChannelId(null);
    }
  }

  function handleSelect(channel: Channel) {
    onOpenChange(false);
    onSelectChannel(channel.id);
  }

  function enterCreateMode(prefillName: string) {
    setCreateInitialName(prefillName);
    setMode("create");
  }

  function exitCreateMode() {
    setMode("browse");
    // Return focus to the search field so keyboard users stay oriented.
    window.requestAnimationFrame(() => {
      inputRef.current?.focus();
    });
  }

  // Map the flat nav index back to a channel, accounting for the create row
  // occupying index 0 when present.
  const selectedItem =
    selectedIndex !== null && !isCreateRowSelected
      ? orderedVisibleChannels[selectedIndex - channelNavOffset]
      : undefined;
  const emptyTitle =
    deferredQuery.length > 0
      ? `No ${entityLabel}s match your search`
      : activeTab === "archived"
        ? `No archived ${entityLabel}s`
        : activeTab === "joined"
          ? `No joined ${entityLabel}s`
          : `No ${entityLabel}s to browse`;
  const emptyDescription =
    deferredQuery.length > 0
      ? canCreate
        ? `No ${entityLabel} by that name yet — create it to get started.`
        : "Try a different name or keyword."
      : activeTab === "archived"
        ? `Archived ${entityLabel}s you have joined will appear here.`
        : activeTab === "joined"
          ? `${entityLabel[0].toUpperCase()}${entityLabel.slice(1)}s you join will appear here.`
          : `All open ${entityLabel}s are available in the sidebar. Create a new ${entityLabel} to get started.`;

  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent
        aria-describedby={undefined}
        className="gap-0 overflow-hidden border-0 px-6 pb-0 pt-6"
        data-testid={
          isForumMode ? "forum-browser-dialog" : "channel-browser-dialog"
        }
        onOpenAutoFocus={(event) => {
          event.preventDefault();
          if (mode === "browse") {
            inputRef.current?.focus({ preventScroll: true });
          }
        }}
        showCloseButton={false}
      >
        {mode === "create" ? (
          <ChannelCreateView
            entityLabel={entityLabel}
            form={createForm}
            onBack={exitCreateMode}
            onClose={() => onOpenChange(false)}
          />
        ) : (
          <>
            <DialogHeader className="space-y-0 pb-5">
              <div className="flex items-center justify-between gap-4">
                <DialogTitle>{browseTitle}</DialogTitle>
                <DialogClose className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors duration-150 ease-out hover:bg-accent hover:text-accent-foreground focus:outline-hidden focus:ring-1 focus:ring-ring">
                  <X className="h-4 w-4" />
                  <span className="sr-only">Close</span>
                </DialogClose>
              </div>
              <div className={MODAL_SEARCH_SHELL_CLASS}>
                <label
                  className="flex min-w-0 flex-1 cursor-text items-center gap-3"
                  htmlFor="channel-browser-search"
                >
                  <Search className="h-4 w-4 shrink-0 text-muted-foreground/55 transition-colors duration-150 ease-out group-hover/search:text-muted-foreground group-focus-within/search:text-foreground" />
                  <input
                    autoCapitalize="none"
                    autoCorrect="off"
                    className={MODAL_SEARCH_INPUT_CLASS}
                    data-testid="channel-browser-search"
                    id="channel-browser-search"
                    onChange={(event) => {
                      setQuery(event.target.value);
                      setSelectedIndex(null);
                    }}
                    onKeyDown={(event) => {
                      // Arrow keys traverse the pinned create row (index 0)
                      // and the channel list beneath it, in visual order.
                      if (event.key === "ArrowDown" && navItemCount > 0) {
                        event.preventDefault();
                        setSelectedIndex((current) =>
                          current === null
                            ? 0
                            : Math.min(current + 1, navItemCount - 1),
                        );
                        return;
                      }

                      if (event.key === "ArrowUp" && navItemCount > 0) {
                        event.preventDefault();
                        setSelectedIndex((current) =>
                          current === null
                            ? navItemCount - 1
                            : Math.max(current - 1, 0),
                        );
                        return;
                      }

                      if (
                        event.key === "Enter" &&
                        !event.nativeEvent.isComposing
                      ) {
                        // If the create row is highlighted — or it's the only
                        // actionable item (no channel matches) — Enter creates.
                        if (
                          showCreateRow &&
                          (isCreateRowSelected ||
                            orderedVisibleChannels.length === 0)
                        ) {
                          event.preventDefault();
                          enterCreateMode(trimmedQuery);
                          return;
                        }

                        if (orderedVisibleChannels.length > 0) {
                          event.preventDefault();
                          handleSelect(
                            selectedItem ?? orderedVisibleChannels[0],
                          );
                        }
                      }
                    }}
                    placeholder={searchPlaceholder}
                    ref={inputRef}
                    spellCheck={false}
                    type="text"
                    value={query}
                  />
                </label>
                <DropdownMenu>
                  <DropdownMenuTrigger asChild>
                    <Button
                      aria-label={`Sort ${entityLabel}s: ${selectedSortLabel}`}
                      data-testid="channel-browser-sort"
                      size="icon-xs"
                      type="button"
                      variant="ghost"
                    >
                      <ListSortDescending />
                    </Button>
                  </DropdownMenuTrigger>
                  <DropdownMenuContent align="end">
                    <DropdownMenuLabel>Sort by</DropdownMenuLabel>
                    <DropdownMenuRadioGroup
                      onValueChange={(value) => {
                        setSort(value as ChannelSort);
                        setSelectedIndex(null);
                      }}
                      value={sort}
                    >
                      {CHANNEL_SORT_OPTIONS.map((option) => (
                        <DropdownMenuRadioItem
                          data-testid={`channel-browser-sort-${option.value}`}
                          key={option.value}
                          value={option.value}
                        >
                          {option.label}
                        </DropdownMenuRadioItem>
                      ))}
                    </DropdownMenuRadioGroup>
                  </DropdownMenuContent>
                </DropdownMenu>
              </div>
            </DialogHeader>

            <div className="h-[min(60vh,30rem)] overflow-hidden">
              <div className="flex h-full flex-col">
                <Tabs
                  className="shrink-0"
                  onValueChange={(value) => {
                    setActiveTab(value as BrowserTab);
                    setSelectedIndex(null);
                  }}
                  value={activeTab}
                >
                  <TabsList
                    className="relative h-auto w-full justify-start gap-6 rounded-none border-b border-border/70 bg-transparent p-0 text-muted-foreground"
                    ref={tabListRef}
                  >
                    <span
                      aria-hidden="true"
                      className="pointer-events-none absolute bottom-[-1px] left-0 h-0.5 w-px origin-left rounded-full bg-foreground opacity-0 transition-[transform,opacity] duration-[180ms] ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none data-[ready=true]:opacity-100"
                      data-ready={tabIndicator.width > 0}
                      data-testid="channel-browser-tab-indicator"
                      style={{
                        transform: `translate3d(${tabIndicator.left}px, 0, 0) scaleX(${tabIndicator.width})`,
                      }}
                    />
                    <TabsTrigger
                      className="rounded-none border-b-2 border-transparent bg-transparent px-0 py-2 text-sm font-medium shadow-none transition-colors duration-150 ease-out data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none"
                      ref={(element) => {
                        tabTriggerRefs.current.all = element;
                      }}
                      value="all"
                    >
                      {allTabLabel}
                    </TabsTrigger>
                    <TabsTrigger
                      className="rounded-none border-b-2 border-transparent bg-transparent px-0 py-2 text-sm font-medium shadow-none transition-colors duration-150 ease-out data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none"
                      ref={(element) => {
                        tabTriggerRefs.current.joined = element;
                      }}
                      value="joined"
                    >
                      Joined
                    </TabsTrigger>
                    <TabsTrigger
                      className="rounded-none border-b-2 border-transparent bg-transparent px-0 py-2 text-sm font-medium shadow-none transition-colors duration-150 ease-out data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none"
                      ref={(element) => {
                        tabTriggerRefs.current.archived = element;
                      }}
                      value="archived"
                    >
                      Archived
                    </TabsTrigger>
                  </TabsList>
                </Tabs>

                <div className="min-h-0 flex-1 overflow-y-auto pb-6 pt-4">
                  {showCreateRow ? (
                    <div className="mb-3">
                      <CreateChannelRow
                        entityLabel={entityLabel}
                        isSelected={isCreateRowSelected}
                        onClick={() => enterCreateMode(trimmedQuery)}
                        query={trimmedQuery}
                      />
                    </div>
                  ) : null}

                  {orderedVisibleChannels.length === 0 ? (
                    <BrowseState
                      description={emptyDescription}
                      icon={deferredQuery.length > 0 ? Search : Compass}
                      title={emptyTitle}
                    />
                  ) : (
                    <div className="overflow-hidden rounded-xl border border-border/70 bg-background/70 shadow-xs divide-y divide-border/55">
                      {orderedVisibleChannels.map((channel, index) => (
                        <ChannelCard
                          channel={channel}
                          isJoining={joiningChannelId === channel.id}
                          isSelected={
                            index + channelNavOffset === selectedIndex
                          }
                          key={channel.id}
                          onJoin={
                            !channel.isMember
                              ? () => {
                                  void handleJoin(channel.id);
                                }
                              : undefined
                          }
                          onSelect={() => handleSelect(channel)}
                        />
                      ))}
                    </div>
                  )}
                </div>
              </div>
            </div>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}

function CreateChannelRow({
  entityLabel,
  isSelected,
  onClick,
  query,
}: {
  entityLabel: string;
  isSelected: boolean;
  onClick: () => void;
  query: string;
}) {
  const hasQuery = query.length > 0;
  return (
    <button
      className={
        isSelected
          ? "flex w-full items-center gap-3 rounded-xl border border-border/70 bg-muted/60 px-4 py-3 text-left transition-colors duration-150 ease-out focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
          : "flex w-full items-center gap-3 rounded-xl border border-border/70 bg-muted/30 px-4 py-3 text-left transition-colors duration-150 ease-out hover:bg-muted/60 focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
      }
      data-testid="channel-browser-create-row"
      data-selected={isSelected}
      onClick={onClick}
      type="button"
    >
      <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
        <Plus className="h-4 w-4" />
      </span>
      {hasQuery ? (
        <span className="min-w-0 text-sm">
          <span className="font-medium text-foreground">
            Create {entityLabel}{" "}
          </span>
          <span className="font-semibold text-foreground">“{query}”</span>
        </span>
      ) : (
        <span className="min-w-0 text-sm font-medium text-foreground">
          Create a new {entityLabel}
        </span>
      )}
    </button>
  );
}

function ChannelCreateView({
  entityLabel,
  form,
  onBack,
  onClose,
}: {
  entityLabel: string;
  form: ReturnType<typeof useCreateChannelForm>;
  onBack: () => void;
  onClose: () => void;
}) {
  return (
    <div className="flex h-[min(72vh,38rem)] flex-col">
      <DialogHeader className="space-y-0 pb-4">
        <div className="flex items-center justify-between gap-4">
          <div className="flex min-w-0 items-center gap-2">
            <button
              aria-label="Back to search"
              className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors duration-150 ease-out hover:bg-accent hover:text-accent-foreground focus:outline-hidden focus:ring-1 focus:ring-ring"
              data-testid="channel-browser-create-back"
              onClick={onBack}
              type="button"
            >
              <ArrowLeft className="h-4 w-4" />
            </button>
            <DialogTitle className="truncate">
              {`New ${entityLabel}`}
            </DialogTitle>
          </div>
          <button
            aria-label="Close"
            className="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors duration-150 ease-out hover:bg-accent hover:text-accent-foreground focus:outline-hidden focus:ring-1 focus:ring-ring"
            onClick={onClose}
            type="button"
          >
            <X className="h-4 w-4" />
            <span className="sr-only">Close</span>
          </button>
        </div>
      </DialogHeader>

      <div className="min-h-0 flex-1 overflow-y-auto pb-2">
        <form
          className="space-y-5"
          id={CREATE_CHANNEL_FORM_ID}
          onSubmit={form.handleSubmit}
        >
          <CreateChannelFormFields form={form} />
        </form>
      </div>

      <div className="shrink-0 pb-6 pt-4">
        <CreateChannelFormFooter form={form} />
      </div>
    </div>
  );
}

function ChannelCard({
  channel,
  isJoining,
  isSelected,
  onJoin,
  onSelect,
}: {
  channel: Channel;
  isJoining: boolean;
  isSelected: boolean;
  onJoin?: () => void;
  onSelect: () => void;
}) {
  const memberLabel = `${channel.memberCount} ${
    channel.memberCount === 1 ? "member" : "members"
  }`;

  return (
    <div
      className={
        isSelected
          ? "group/channel-row flex min-h-16 items-center gap-4 bg-muted/40 px-4 py-3 transition-colors duration-150 ease-out"
          : "group/channel-row flex min-h-16 items-center gap-4 px-4 py-3 transition-colors duration-150 ease-out hover:bg-muted/40"
      }
      data-testid={`browse-channel-${channel.name}`}
    >
      <button
        className="min-w-0 flex-1 border-0 bg-transparent p-0 text-left text-foreground outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
        onClick={(event) => {
          event.stopPropagation();
          onSelect();
        }}
        type="button"
      >
        <div className="min-w-0">
          <div className="flex min-w-0 items-center gap-1.5">
            <span className="shrink-0 text-sm font-normal text-muted-foreground">
              #
            </span>
            <p className="min-w-0 truncate text-base font-medium tracking-tight">
              {channel.name}
            </p>
            {channel.archivedAt ? (
              <Badge className="ml-1 shrink-0" variant="warning">
                archived
              </Badge>
            ) : null}
          </div>
          <p className="mt-1 truncate text-sm text-muted-foreground">
            <span>{memberLabel}</span>
            {channel.description ? (
              <>
                <span className="px-1.5">·</span>
                <span title={channel.description}>{channel.description}</span>
              </>
            ) : null}
          </p>
        </div>
      </button>

      {!channel.isMember && onJoin ? (
        <Button
          className={
            isJoining
              ? "shrink-0"
              : "shrink-0 opacity-0 transition-opacity duration-150 ease-out group-hover/channel-row:opacity-100 group-focus-within/channel-row:opacity-100"
          }
          disabled={isJoining}
          onClick={(event) => {
            event.stopPropagation();
            onJoin();
          }}
          size="sm"
          type="button"
          variant="default"
        >
          {isJoining ? "Joining..." : "Join"}
        </Button>
      ) : null}
    </div>
  );
}
