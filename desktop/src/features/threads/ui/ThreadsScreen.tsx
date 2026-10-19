import { MessagesSquare, Pin } from "lucide-react";
import { AnimatePresence } from "motion/react";
import { useLocation } from "@tanstack/react-router";
import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  type ActiveThreadTurnSummary,
  useActiveAgentTurnsByThread,
} from "@/features/agents/activeAgentTurnsStore";
import { formatElapsed } from "@/features/agents/ui/agentSessionUtils";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useThreadViewMode } from "@/features/channels/lib/threadViewModePreference";
import {
  mergeOpenChannelDirectory,
  useOpenChannelDirectoryQuery,
} from "@/features/channels/openChannelDirectory";
import { useFocusDrawerPresence } from "@/features/channels/ui/useFocusDrawerPresence";
import { formatRelativeTime } from "@/features/forum/lib/time";
import { getThreadReference } from "@/features/messages/lib/threading";
import type { TimelineMessage } from "@/features/messages/types";
import {
  type ConversationUnread,
  ProjectConversationPanel,
} from "@/features/projects/ui/ProjectConversationPanel";
import { resolveChannelDisplayLabel } from "@/features/sidebar/lib/channelLabels";
import { formatWorkingTooltip } from "@/features/sidebar/ui/SidebarSection";
import {
  useMarkOpenThreadRead,
  useThreadTitlesQuery,
  useTitledThreadUnreadCounts,
} from "@/features/threads/hooks";
import {
  effectiveChannelFilter,
  filterByChannel,
  threadChannelOptions,
} from "@/features/threads/lib/threadChannelFilter";
import { unreadReplyIdsForRoot } from "@/features/threads/lib/threadPaneUnread";
import { partitionByPin } from "@/features/threads/lib/threadPins";
import { useThreadPins } from "@/features/threads/lib/useThreadPins";
import { ThreadsChannelFilterMenu } from "@/features/threads/ui/ThreadsChannelFilterMenu";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useRelayOrigin } from "@/shared/lib/useRelayOrigin";
import type { ThreadTitle } from "@/shared/api/tauriThreadTitles";
import type { Channel } from "@/shared/api/types";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { useThreadPanelWidth } from "@/shared/hooks/useThreadPanelWidth";
import { cn } from "@/shared/lib/cn";
import { useNow } from "@/shared/lib/useNow";
import { Button } from "@/shared/ui/button";
import { Skeleton } from "@/shared/ui/skeleton";

const CHANNEL_FILTER_SESSION_KEY = "buzz.desktop.threads-channel-filter";

const FOCUS_DRAWER = { backLabel: "Back to Threads" };

type OpenThread = {
  channelId: string;
  channelName: string;
  rootId: string;
  /** Replies unread as the thread opened, before the pane reads them. */
  unreadAtOpen: ReadonlySet<string>;
};

function readSessionFilter(): string | null {
  try {
    return window.sessionStorage.getItem(CHANNEL_FILTER_SESSION_KEY);
  } catch {
    return null;
  }
}

function writeSessionFilter(channelId: string | null) {
  try {
    if (channelId === null) {
      window.sessionStorage.removeItem(CHANNEL_FILTER_SESSION_KEY);
    } else {
      window.sessionStorage.setItem(CHANNEL_FILTER_SESSION_KEY, channelId);
    }
  } catch {
    // Storage can be unavailable; the filter still works for this visit.
  }
}

function channelLabel(
  channel: Channel | undefined,
  currentPubkey: string | undefined,
): string {
  if (!channel) {
    return "Unknown channel";
  }
  const label = resolveChannelDisplayLabel(channel, currentPubkey, undefined);
  return channel.channelType === "dm" ? label : `#${label}`;
}

/** Live agent-working timer, the thread-scoped twin of the sidebar's badge. */
function ThreadWorkingBadge({ summary }: { summary: ActiveThreadTurnSummary }) {
  const now = useNow(1000);
  const elapsed = formatElapsed(now - summary.anchorAt);
  return (
    <span
      className="inline-flex shrink-0 rounded-full bg-primary/10 px-1.5 py-0.5 text-2xs font-medium leading-none tabular-nums text-primary motion-safe:animate-pulse"
      data-testid="threads-view-row-working"
      title={formatWorkingTooltip(summary)}
    >
      {summary.agentCount > 1 ? `${elapsed} (${summary.agentCount})` : elapsed}
    </span>
  );
}

function ThreadRow({
  channel,
  currentPubkey,
  entry,
  onOpen,
  onTogglePin,
  pinned,
  selected,
  unreadCount,
  working,
}: {
  channel: Channel | undefined;
  currentPubkey: string | undefined;
  entry: ThreadTitle;
  onOpen: (entry: ThreadTitle, channel: Channel | undefined) => void;
  onTogglePin: ((rootId: string) => void) | undefined;
  pinned: boolean;
  selected: boolean;
  unreadCount: number;
  working: ActiveThreadTurnSummary | undefined;
}) {
  const hasUnread = unreadCount > 0;
  return (
    <div
      className={cn(
        "group/thread-row relative flex min-w-0 items-center rounded-lg hover:bg-accent focus-within:bg-accent",
        selected && "bg-accent",
      )}
    >
      <button
        aria-current={selected ? "true" : undefined}
        className="flex min-w-0 flex-1 flex-col items-start gap-0.5 rounded-lg px-3 py-2 text-left focus-visible:outline-none"
        data-pinned={pinned ? "true" : undefined}
        data-selected={selected ? "true" : undefined}
        data-testid="threads-view-row"
        data-unread={hasUnread ? "true" : undefined}
        onClick={() => onOpen(entry, channel)}
        type="button"
      >
        <span className="flex w-full min-w-0 items-center gap-2">
          <span
            className={cn(
              "min-w-0 flex-1 truncate text-sm",
              hasUnread
                ? "font-bold text-foreground"
                : "font-normal text-foreground/80",
            )}
          >
            {entry.title}
          </span>
          {working ? <ThreadWorkingBadge summary={working} /> : null}
          {hasUnread ? (
            <span
              className="flex shrink-0 items-center gap-1.5 text-xs font-medium text-primary"
              data-testid="threads-view-row-unread"
            >
              <span
                aria-hidden="true"
                className="h-2 w-2 rounded-full bg-primary"
              />
              {unreadCount} new
            </span>
          ) : null}
        </span>
        <span className="flex w-full min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
          <span className="min-w-0 truncate">
            {channelLabel(channel, currentPubkey)}
          </span>
          <span aria-hidden="true">·</span>
          <span className="shrink-0">
            {formatRelativeTime(entry.lastActivityAt)}
          </span>
        </span>
      </button>
      {onTogglePin ? (
        <button
          aria-label={pinned ? "Unpin thread" : "Pin thread"}
          aria-pressed={pinned}
          className={cn(
            "mr-2 flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted-foreground hover:bg-background hover:text-foreground focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
            pinned
              ? "text-primary"
              : "opacity-0 group-hover/thread-row:opacity-100 group-focus-within/thread-row:opacity-100",
          )}
          data-testid="threads-view-row-pin"
          onClick={() => onTogglePin(entry.rootId)}
          title={pinned ? "Unpin thread" : "Pin thread"}
          type="button"
        >
          <Pin className={cn("h-3.5 w-3.5", pinned && "fill-current")} />
        </button>
      ) : null}
    </div>
  );
}

function ThreadsBody({
  channelFilter,
  channelsById,
  currentPubkey,
  onOpen,
  selectedRootId,
  titlesQuery,
}: {
  channelFilter: string | null;
  channelsById: ReadonlyMap<string, Channel>;
  currentPubkey: string | undefined;
  onOpen: (entry: ThreadTitle, channel: Channel | undefined) => void;
  selectedRootId: string | null;
  titlesQuery: ReturnType<typeof useThreadTitlesQuery>;
}) {
  const unreadCounts = useTitledThreadUnreadCounts();
  const workingThreads = useActiveAgentTurnsByThread();
  const relayOrigin = useRelayOrigin();
  const { pinned: pinnedRoots, toggle: handleTogglePin } = useThreadPins(
    relayOrigin,
    currentPubkey,
  );

  if (titlesQuery.isPending) {
    return (
      <div className="space-y-3 px-3 py-2">
        {[1, 2, 3, 4].map((i) => (
          <div className="space-y-1.5" key={i}>
            <Skeleton className="h-4 w-64" />
            <Skeleton className="h-3 w-32" />
          </div>
        ))}
      </div>
    );
  }

  if (titlesQuery.isError) {
    return (
      <div className="flex flex-col items-center gap-3 px-4 py-12 text-center">
        <p className="text-sm text-muted-foreground">
          Couldn't load thread titles.
        </p>
        <Button
          onClick={() => void titlesQuery.refetch()}
          size="sm"
          variant="outline"
        >
          Retry
        </Button>
      </div>
    );
  }

  if (titlesQuery.data.length === 0) {
    return (
      <div className="flex flex-col items-center gap-2 px-4 py-12 text-center">
        <MessagesSquare className="h-6 w-6 text-muted-foreground" />
        <p className="text-sm font-medium text-foreground">No named threads</p>
        <p className="max-w-sm text-sm text-muted-foreground">
          Open a thread and use the pencil next to its title to name it. Named
          threads show up here, most recently active first.
        </p>
      </div>
    );
  }

  const { pinned, unpinned } = partitionByPin(
    filterByChannel(titlesQuery.data, channelFilter),
    pinnedRoots,
  );
  const renderRows = (entries: ThreadTitle[], isPinned: boolean) =>
    entries.map((entry) => (
      <li key={`${entry.channelId}:${entry.rootId}`}>
        <ThreadRow
          channel={channelsById.get(entry.channelId)}
          currentPubkey={currentPubkey}
          entry={entry}
          onOpen={onOpen}
          onTogglePin={handleTogglePin}
          pinned={isPinned}
          selected={entry.rootId === selectedRootId}
          unreadCount={unreadCounts.get(entry.rootId) ?? 0}
          working={workingThreads.get(entry.rootId)}
        />
      </li>
    ));

  if (pinned.length === 0) {
    return (
      <ul aria-label="Named threads" className="flex flex-col">
        {renderRows(unpinned, false)}
      </ul>
    );
  }

  return (
    <div className="flex flex-col gap-3">
      <section aria-labelledby="threads-view-pinned-heading">
        <h2
          className="px-3 pb-1 text-xs font-medium text-muted-foreground"
          id="threads-view-pinned-heading"
        >
          Pinned
        </h2>
        <ul
          aria-label="Pinned threads"
          className="flex flex-col"
          data-testid="threads-view-pinned"
        >
          {renderRows(pinned, true)}
        </ul>
      </section>
      {unpinned.length > 0 ? (
        <section aria-labelledby="threads-view-recent-heading">
          <h2
            className="px-3 pb-1 text-xs font-medium text-muted-foreground"
            id="threads-view-recent-heading"
          >
            Recent
          </h2>
          <ul aria-label="Named threads" className="flex flex-col">
            {renderRows(unpinned, false)}
          </ul>
        </section>
      ) : null}
    </div>
  );
}

/**
 * Named threads across every readable channel, most recently active first.
 * Threads in joined chat channels open beside the list so it keeps its place.
 */
export function ThreadsScreen() {
  const titlesQuery = useThreadTitlesQuery({ refetchInterval: true });
  const channelsQuery = useChannelsQuery();
  const currentPubkey = useIdentityQuery().data?.pubkey;
  const { goChannel, goForumPost } = useAppNavigation();
  const panelWidth = useThreadPanelWidth();
  const [openThread, setOpenThread] = React.useState<OpenThread | null>(null);
  const [selectedFilter, setSelectedFilter] = React.useState(readSessionFilter);
  const { feedItemState, markMessageRead, unreadThreadFeedItems } =
    useAppShell();
  const unreadFeedReplies = React.useMemo(
    () =>
      unreadThreadFeedItems.map((item) => ({
        id: item.id,
        rootId: getThreadReference(item.tags).rootId,
      })),
    [unreadThreadFeedItems],
  );
  // After "Mark unread" the open thread stops reading itself, or the replies
  // would be read again at once; opening any thread (or closing) resumes it.
  const [heldUnreadThread, setHeldUnreadThread] =
    React.useState<OpenThread | null>(null);
  useMarkOpenThreadRead(
    openThread && openThread !== heldUnreadThread ? openThread.rootId : null,
  );
  const markOpenThreadUnread = React.useCallback(
    (replyIds: string[]) => {
      for (const id of replyIds) feedItemState.markUnread(id);
      setHeldUnreadThread(openThread);
    },
    [feedItemState.markUnread, openThread],
  );
  const openRootId = openThread?.rootId ?? null;
  const currentUnread = React.useMemo(
    () => unreadReplyIdsForRoot(unreadFeedReplies, openRootId),
    [openRootId, unreadFeedReplies],
  );
  const markOpenRepliesRead = React.useCallback(
    (replies: TimelineMessage[]) => {
      for (const reply of replies) {
        feedItemState.undoUnread(reply.id);
        markMessageRead(reply.id, reply.createdAt);
      }
      // Once nothing marked unread is left, the open thread reads itself again.
      if ([...currentUnread].every((id) => replies.some((r) => r.id === id))) {
        setHeldUnreadThread(null);
      }
    },
    [currentUnread, feedItemState.undoUnread, markMessageRead],
  );
  const unreadAtOpen = openThread?.unreadAtOpen;
  const paneUnread = React.useMemo<ConversationUnread | undefined>(
    () =>
      unreadAtOpen
        ? {
            atOpen: unreadAtOpen,
            current: currentUnread,
            onMarkRead: markOpenRepliesRead,
          }
        : undefined,
    [currentUnread, markOpenRepliesRead, unreadAtOpen],
  );

  // Threads follow the channel thread layout preference: Focus opens the
  // large drawer over the list, Split keeps the list beside the thread.
  const threadViewMode = useThreadViewMode();
  const isOverlay = useIsThreadPanelOverlay();
  const useFocusDrawer = threadViewMode === "focus" && !isOverlay;
  const closeThread = React.useCallback(() => setOpenThread(null), []);
  const { channelIsCovered: listIsCovered, markExitComplete } =
    useFocusDrawerPresence(useFocusDrawer && openThread !== null, closeThread);

  // Clicking Threads in the sidebar again returns to the list.
  const threadsResetId = useLocation({
    select: (location) =>
      (location.state as { threadsResetId?: unknown } | undefined)
        ?.threadsResetId,
  });
  const lastResetIdRef = React.useRef(threadsResetId);
  React.useEffect(() => {
    if (threadsResetId === lastResetIdRef.current) return;
    lastResetIdRef.current = threadsResetId;
    closeThread();
  }, [closeThread, threadsResetId]);

  // Titles also come from open channels the user hasn't joined; only then pay
  // for the all-open directory scan (React Query dedups it across surfaces).
  const needsDirectory =
    channelsQuery.isSuccess &&
    (titlesQuery.data ?? []).some(
      (entry) =>
        !channelsQuery.data.some((channel) => channel.id === entry.channelId),
    );
  const openDirectoryQuery = useOpenChannelDirectoryQuery({
    enabled: needsDirectory,
  });
  const channelsById = React.useMemo(
    () =>
      new Map(
        mergeOpenChannelDirectory(
          channelsQuery.data ?? [],
          openDirectoryQuery.data,
        ).map((channel) => [channel.id, channel]),
      ),
    [channelsQuery.data, openDirectoryQuery.data],
  );
  const channelOptions = React.useMemo(
    () =>
      threadChannelOptions(titlesQuery.data ?? [], (channelId) =>
        channelLabel(channelsById.get(channelId), currentPubkey),
      ),
    [channelsById, currentPubkey, titlesQuery.data],
  );
  const channelFilter = titlesQuery.isSuccess
    ? effectiveChannelFilter(selectedFilter, channelOptions)
    : selectedFilter;

  const handleFilterChange = React.useCallback((channelId: string | null) => {
    setSelectedFilter(channelId);
    writeSessionFilter(channelId);
  }, []);

  const handleOpen = React.useCallback(
    (entry: ThreadTitle, channel: Channel | undefined) => {
      if (channel?.channelType === "forum") {
        void goForumPost(entry.channelId, entry.rootId);
        return;
      }
      // The pane resolves its channel from the joined list; anything else
      // (an open channel the user never joined) still navigates.
      const joined = channelsQuery.data?.find(
        (candidate) => candidate.id === entry.channelId,
      );
      if (!joined) {
        void goChannel(entry.channelId, {
          messageId: entry.rootId,
          threadRootId: entry.rootId,
        });
        return;
      }
      setOpenThread({
        channelId: joined.id,
        channelName: joined.name,
        rootId: entry.rootId,
        unreadAtOpen: unreadReplyIdsForRoot(unreadFeedReplies, entry.rootId),
      });
    },
    [channelsQuery.data, goChannel, goForumPost, unreadFeedReplies],
  );

  return (
    <div className="relative flex min-h-0 min-w-0 flex-1 flex-row overflow-hidden">
      <div
        className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden"
        inert={listIsCovered ? true : undefined}
      >
        <TopChromeInsetHeader flush transparent>
          <div className="px-5 py-2">
            <div className="flex min-h-9 items-center gap-2">
              <MessagesSquare className="h-4 w-4 text-muted-foreground" />
              <h1 className="text-sm font-semibold">Threads</h1>
              {channelOptions.length > 1 ? (
                <div className="ml-auto flex min-w-0">
                  <ThreadsChannelFilterMenu
                    onChange={handleFilterChange}
                    options={channelOptions}
                    selected={channelFilter}
                    totalCount={titlesQuery.data?.length ?? 0}
                  />
                </div>
              ) : null}
            </div>
          </div>
        </TopChromeInsetHeader>
        <p
          className="border-b border-border/60 px-5 pb-3 text-sm text-muted-foreground"
          data-testid="threads-view-subheader"
        >
          Named threads, most recent activity first. Name any thread from its
          header.
        </p>
        <div
          className="min-h-0 flex-1 overflow-y-auto"
          data-testid="threads-view"
        >
          <div className="w-full max-w-3xl px-2 pb-10 pt-2">
            <ThreadsBody
              channelFilter={channelFilter}
              channelsById={channelsById}
              currentPubkey={currentPubkey}
              onOpen={handleOpen}
              selectedRootId={openThread?.rootId ?? null}
              titlesQuery={titlesQuery}
            />
          </div>
        </div>
      </div>
      <AnimatePresence onExitComplete={markExitComplete}>
        {openThread ? (
          <ProjectConversationPanel
            canResetWidth={panelWidth.canReset}
            focusDrawer={useFocusDrawer ? FOCUS_DRAWER : undefined}
            hit={{
              channelId: openThread.channelId,
              channelName: openThread.channelName,
              eventId: openThread.rootId,
              threadRootId: openThread.rootId,
            }}
            key="threads-conversation"
            onClose={closeThread}
            onMarkUnread={markOpenThreadUnread}
            onResetWidth={panelWidth.onResetWidth}
            onResizeStart={panelWidth.onResizeStart}
            // Switching layouts swaps the drawer out without an exit animation.
            onThreadViewModeChange={markExitComplete}
            titleOpensThread
            unread={paneUnread}
            widthPx={panelWidth.widthPx}
          />
        ) : null}
      </AnimatePresence>
    </div>
  );
}
