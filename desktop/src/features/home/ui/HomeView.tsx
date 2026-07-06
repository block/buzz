import * as React from "react";
import { RefreshCcw } from "lucide-react";

import { useAppShell } from "@/app/AppShellContext";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useChannelsQuery, useOpenDmMutation } from "@/features/channels/hooks";
import { RightAuxiliaryPane } from "@/features/channels/ui/RightAuxiliaryPane";
import { ChannelManagementSheet } from "@/features/channels/ui/ChannelManagementSheet";
import {
  type InboxFilter,
  type InboxContextMessage,
  type InboxReply,
  buildInboxItems,
  formatInboxFullTimestamp,
} from "@/features/home/lib/inbox";
import {
  getContextMessageDepth,
  getReactionTargetId,
  matchesInboxFilter,
} from "@/features/home/lib/inboxViewHelpers";
import { useHomeInboxReadState } from "@/features/home/useHomeInboxReadState";
import { useInboxThreadContext } from "@/features/home/useInboxThreadContext";
import {
  type ProfilePanelTab,
  type ProfilePanelView,
  UserProfilePanel,
} from "@/features/profile/ui/UserProfilePanel";
import {
  profilePanelTabFromSearch,
  profilePanelViewFromSearch,
} from "@/features/profile/ui/UserProfilePanelUtils";
import {
  INBOX_COLUMN_MIN_WIDTH_PX,
  INBOX_SINGLE_COLUMN_BREAKPOINT_PX,
  useResizableInboxListWidth,
} from "@/features/home/useResizableInboxListWidth";
import { HomeLoadingState } from "@/features/home/ui/HomeLoadingState";
import { InboxDetailPane } from "@/features/home/ui/InboxDetailPane";
import { InboxListPane } from "@/features/home/ui/InboxListPane";
import {
  useChannelMessagesQuery,
  useToggleReactionMutation,
} from "@/features/messages/hooks";
import {
  collectMessageMentionPubkeys,
  formatTimelineMessages,
} from "@/features/messages/lib/formatTimelineMessages";
import { formatTime } from "@/features/messages/lib/dateFormatters";
import { splitOutgoingTags } from "@/features/messages/lib/imetaMediaMarkdown";
import { getThreadReference } from "@/features/messages/lib/threading";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import {
  countDueReminders,
  useRemindersQuery,
} from "@/features/reminders/hooks";
import { useRemindLater } from "@/features/reminders/ui/RemindMeLaterProvider";
import { deleteMessage, sendChannelMessage } from "@/shared/api/tauri";
import type { HomeFeedResponse } from "@/shared/api/types";
import { KIND_REACTION } from "@/shared/constants/kinds";
import { topChromeInset } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { resolveMentionNames } from "@/shared/lib/resolveMentionNames";
import { useElementWidth } from "@/shared/hooks/use-mobile";
import { useThreadPanelWidth } from "@/shared/hooks/useThreadPanelWidth";
import { AUXILIARY_PANEL_SINGLE_COLUMN_BREAKPOINT_PX } from "@/shared/layout/AuxiliaryPanel";
import { useHistorySearchState } from "@/shared/hooks/useHistorySearchState";
import { ProfilePanelProvider } from "@/shared/context/ProfilePanelContext";
import { Button } from "@/shared/ui/button";

const INBOX_SEARCH_KEYS = [
  "item",
  "profile",
  "profileTab",
  "profileView",
] as const;

type HomeViewProps = {
  feed?: HomeFeedResponse;
  isLoading?: boolean;
  errorMessage?: string;
  currentPubkey?: string;
  availableChannelIds: ReadonlySet<string>;
  onOpenContext: (
    channelId: string,
    messageId: string,
    threadRootId?: string | null,
  ) => void;
  onRefresh: () => void;
};

export function HomeView({
  feed,
  isLoading = false,
  errorMessage,
  currentPubkey,
  availableChannelIds,
  onOpenContext,
  onRefresh,
}: HomeViewProps) {
  const [homeInboxRef, homeInboxWidthPx] = useElementWidth<HTMLDivElement>();
  const isNarrowHomeViewport =
    homeInboxWidthPx > 0 &&
    homeInboxWidthPx < INBOX_SINGLE_COLUMN_BREAKPOINT_PX;
  const [filter, setFilter] = React.useState<InboxFilter>("all");
  const [unreadOnly, setUnreadOnly] = React.useState(false);
  // Explicit selections are mirrored to the URL (`?item=`), so back/forward
  // restores the detail pane each history entry was showing and reloads
  // restore it from the URL. Default/automatic selection stays local-only —
  // background data loads must never trigger navigations.
  const { applyPatch: applyInboxSearchPatch, values: inboxSearchValues } =
    useHistorySearchState(INBOX_SEARCH_KEYS);
  const isReminders = filter === "reminders";
  const isDrafts = filter === "drafts";
  const isMessagesMode = !isReminders && !isDrafts;
  const remindersQuery = useRemindersQuery(currentPubkey);
  const dueReminderCount = countDueReminders(remindersQuery.data ?? []);
  // `?item=` is Messages-mode-only machinery: a reminder never enters the
  // FeedItem selection model, so reload while in Reminders mode keeps a stale
  // `?item=` unconsumed and does not snap back to a feed-item detail view.
  const urlSelectedItemId = isMessagesMode ? inboxSearchValues.item : null;
  const profilePanelPubkey = inboxSearchValues.profile;
  const profilePanelTab = profilePanelTabFromSearch(
    inboxSearchValues.profileTab,
  );
  const profilePanelView = profilePanelViewFromSearch(
    inboxSearchValues.profileView,
  );
  const [selectedItemId, setSelectedItemId] = React.useState<string | null>(
    urlSelectedItemId,
  );
  const [managedChannelId, setManagedChannelId] = React.useState<string | null>(
    null,
  );
  const { goChannel } = useAppNavigation();
  const openDmMutation = useOpenDmMutation();
  React.useEffect(() => {
    setSelectedItemId(urlSelectedItemId);
  }, [urlSelectedItemId]);
  const handleUserSelectItem = React.useCallback(
    (itemId: string | null) => {
      setSelectedItemId(itemId);
      applyInboxSearchPatch({ item: itemId });
    },
    [applyInboxSearchPatch],
  );
  const handleOpenProfilePanel = React.useCallback(
    (pubkey: string) => {
      setManagedChannelId(null);
      applyInboxSearchPatch({
        profile: pubkey,
        profileTab: null,
        profileView: null,
      });
    },
    [applyInboxSearchPatch],
  );
  const handleCloseProfilePanel = React.useCallback(() => {
    applyInboxSearchPatch({
      profile: null,
      profileTab: null,
      profileView: null,
    });
  }, [applyInboxSearchPatch]);
  const handleProfilePanelViewChange = React.useCallback(
    (view: ProfilePanelView, options?: { replace?: boolean }) =>
      applyInboxSearchPatch(
        { profileView: view === "summary" ? null : view },
        options,
      ),
    [applyInboxSearchPatch],
  );
  const handleProfilePanelTabChange = React.useCallback(
    (tab: ProfilePanelTab, options?: { replace?: boolean }) =>
      applyInboxSearchPatch(
        { profileTab: tab === "info" ? null : tab },
        options,
      ),
    [applyInboxSearchPatch],
  );
  const [isDeletingMessage, setIsDeletingMessage] = React.useState(false);
  const [isSendingReply, setIsSendingReply] = React.useState(false);
  const handleOpenDm = React.useCallback(
    async (pubkeys: string[]) => {
      const dm = await openDmMutation.mutateAsync({ pubkeys });
      await goChannel(dm.id);
    },
    [goChannel, openDmMutation],
  );
  const { activeReminderEventIds, openReminder } = useRemindLater();
  const [localRepliesByItemId, setLocalRepliesByItemId] = React.useState<
    Record<string, InboxReply[]>
  >({});
  const {
    canReset: canResetThreadPanelWidth,
    onResetWidth: handleThreadPanelWidthReset,
    onResizeStart: handleThreadPanelResizeStart,
    widthPx: threadPanelWidthPx,
  } = useThreadPanelWidth();
  const {
    canResetInboxListWidth,
    handleInboxListResizeStart,
    handleInboxListWidthReset,
    inboxListWidthPx,
  } = useResizableInboxListWidth();
  const {
    getChannelReadAt,
    getThreadReadAt,
    getMessageReadAt,
    feedItemState,
    markChannelRead,
    markThreadRead,
    readStateVersion,
  } = useAppShell();
  const { doneSet, markDone, markUnread, undoDone, undoUnread, unreadSet } =
    feedItemState;
  const feedItems = React.useMemo(
    () =>
      feed
        ? [
            ...feed.feed.mentions,
            ...feed.feed.needsAction,
            ...feed.feed.activity,
            ...feed.feed.agentActivity,
          ]
        : [],
    [feed],
  );
  const selectedFeedItem =
    feedItems.find((item) => item.id === selectedItemId) ?? null;

  const channelsQuery = useChannelsQuery();
  const channels = channelsQuery.data;
  const selectedChannelIdCandidate = React.useMemo(() => {
    return selectedFeedItem?.channelId ?? null;
  }, [selectedFeedItem]);
  const selectedChannel = React.useMemo(() => {
    if (!selectedChannelIdCandidate || !channels) return null;
    return (
      channels.find((channel) => channel.id === selectedChannelIdCandidate) ??
      null
    );
  }, [channels, selectedChannelIdCandidate]);
  const managedChannel = React.useMemo(() => {
    if (!managedChannelId || !channels) return null;
    return channels.find((channel) => channel.id === managedChannelId) ?? null;
  }, [channels, managedChannelId]);
  const isChannelManagementOpen = managedChannel !== null;
  const hasAuxiliaryPane =
    isChannelManagementOpen || profilePanelPubkey !== null;
  const isSinglePanelAuxiliaryView =
    hasAuxiliaryPane &&
    homeInboxWidthPx > 0 &&
    homeInboxWidthPx < AUXILIARY_PANEL_SINGLE_COLUMN_BREAKPOINT_PX;

  const channelMessagesQuery = useChannelMessagesQuery(selectedChannel);
  const toggleReactionMutation = useToggleReactionMutation();
  const channelMessages = channelMessagesQuery.data;
  const threadContext = useInboxThreadContext(
    selectedFeedItem,
    channelMessages,
  );

  const feedProfilePubkeys = React.useMemo(
    () => [
      ...new Set([
        ...feedItems.map((item) => item.pubkey),
        ...collectMessageMentionPubkeys(feedItems),
        ...threadContext.events.map((event) => event.pubkey),
        ...collectMessageMentionPubkeys(threadContext.events),
        ...(channelMessages ?? [])
          .filter((event) => event.kind === KIND_REACTION)
          .map((event) => event.pubkey),
        ...(currentPubkey ? [currentPubkey] : []),
      ]),
    ],
    [channelMessages, currentPubkey, feedItems, threadContext.events],
  );
  const feedProfilesQuery = useUsersBatchQuery(feedProfilePubkeys, {
    enabled: feedProfilePubkeys.length > 0,
  });
  const feedProfiles = feedProfilesQuery.data?.profiles;
  const inboxAgentPubkeys = React.useMemo(() => {
    const pubkeys = new Set<string>();

    for (const item of feed?.feed.agentActivity ?? []) {
      pubkeys.add(normalizePubkey(item.pubkey));
    }

    for (const [pubkey, profile] of Object.entries(feedProfiles ?? {})) {
      if (profile.isAgent) {
        pubkeys.add(normalizePubkey(pubkey));
      }
    }

    return pubkeys;
  }, [feed?.feed.agentActivity, feedProfiles]);
  const inboxItems = React.useMemo(
    () =>
      buildInboxItems({
        channels,
        currentPubkey,
        feed,
        profiles: feedProfiles,
      }),
    [channels, currentPubkey, feed, feedProfiles],
  );
  const { effectiveDoneSet, markItemRead, markItemUnread } =
    useHomeInboxReadState({
      items: inboxItems,
      getChannelReadAt,
      getThreadReadAt,
      getMessageReadAt,
      readStateVersion,
      localDoneSet: doneSet,
      localUnreadSet: unreadSet,
      markChannelRead,
      markThreadRead,
      markDoneLocal: markDone,
      markUnreadLocal: markUnread,
      undoDoneLocal: undoDone,
      undoUnreadLocal: undoUnread,
    });
  const filteredItems = React.useMemo(() => {
    return inboxItems.filter(
      (item) =>
        matchesInboxFilter(item, filter) &&
        (!unreadOnly ||
          !effectiveDoneSet.has(item.id) ||
          item.id === selectedItemId),
    );
  }, [effectiveDoneSet, filter, inboxItems, selectedItemId, unreadOnly]);
  const selectedItem =
    filteredItems.find((item) => item.id === selectedItemId) ?? null;
  const contextMessages = React.useMemo<InboxContextMessage[]>(() => {
    if (!selectedItem) {
      return [];
    }

    const eventById = new Map(
      threadContext.events.map((event) => [event.id, event]),
    );
    const contextEventIds = new Set(eventById.keys());
    const reactionEvents = (channelMessages ?? []).filter((event) => {
      if (event.kind !== KIND_REACTION) {
        return false;
      }

      const targetId = getReactionTargetId(event.tags);
      return Boolean(targetId && contextEventIds.has(targetId));
    });
    const currentUserAvatarUrl = currentPubkey
      ? (feedProfiles?.[currentPubkey.toLowerCase()]?.avatarUrl ?? null)
      : null;
    const timelineMessages = formatTimelineMessages(
      [...threadContext.events, ...reactionEvents],
      selectedChannel,
      currentPubkey,
      currentUserAvatarUrl,
      feedProfiles,
    );

    return timelineMessages.map((message) => {
      const event = eventById.get(message.id);
      const authorPubkey =
        message.pubkey ?? event?.pubkey ?? selectedItem.item.pubkey;
      return {
        id: message.id,
        authorLabel: message.author,
        authorPubkey,
        avatarUrl: message.avatarUrl ?? null,
        content: message.body,
        depth: event ? getContextMessageDepth(event, eventById) : message.depth,
        fullTimestampLabel: formatInboxFullTimestamp(message.createdAt),
        isSelected: message.id === selectedItem.id,
        mentionNames:
          resolveMentionNames(message.tags ?? [], feedProfiles) ?? [],
        reactions: message.reactions,
        tags: message.tags,
        timeLabel: message.time,
      };
    });
  }, [
    channelMessages,
    currentPubkey,
    feedProfiles,
    selectedChannel,
    selectedItem,
    threadContext.events,
  ]);
  const selectedItemReplies = React.useMemo<InboxReply[]>(() => {
    if (!selectedItem) return [];
    const localReplies = localRepliesByItemId[selectedItem.id] ?? [];
    const contextIds = new Set(contextMessages.map((message) => message.id));
    return localReplies.filter((reply) => !contextIds.has(reply.id));
  }, [contextMessages, localRepliesByItemId, selectedItem]);
  React.useEffect(() => {
    // Auto-selection is Messages-mode-only: in Reminders mode no FeedItem is
    // ever selected, so default-selecting one behind the reminders list would
    // be wasted work and could drive narrow-viewport detail off a stale feed
    // selection.
    if (!isMessagesMode) {
      return;
    }

    // While the feed is loading (e.g. a reload restoring `?item=` from the
    // URL) the selected item simply hasn't arrived yet — don't clobber it.
    if (isLoading || !feed) {
      return;
    }

    if (filteredItems.length === 0) {
      setSelectedItemId(null);
      return;
    }

    // Don't default-select before the width is measured: at width 0
    // isNarrowHomeViewport is false, so narrow Home would cold-load into detail.
    if (homeInboxWidthPx === 0) {
      return;
    }

    if (!filteredItems.some((item) => item.id === selectedItemId)) {
      setSelectedItemId(
        isNarrowHomeViewport ? null : (filteredItems[0]?.id ?? null),
      );
    }
  }, [
    feed,
    filteredItems,
    homeInboxWidthPx,
    isLoading,
    isMessagesMode,
    isNarrowHomeViewport,
    selectedItemId,
  ]);

  React.useEffect(() => {
    void selectedItemId;
    setIsDeletingMessage(false);
    setIsSendingReply(false);
  }, [selectedItemId]);

  if (isLoading && !feed) {
    return <HomeLoadingState />;
  }

  if (!feed) {
    return (
      <div className="flex-1 overflow-hidden px-4 pb-3 pt-4 sm:px-6">
        <div className="flex w-full max-w-3xl flex-col gap-4">
          <div className="rounded-md border border-destructive/30 bg-destructive/5 px-4 py-5">
            <p className="text-base font-semibold tracking-tight">
              Home feed unavailable
            </p>
            <p className="mt-2 text-sm text-muted-foreground">
              {errorMessage ?? "The relay did not return a feed response."}
            </p>
            <Button className="mt-5" onClick={onRefresh} type="button">
              <RefreshCcw className="h-4 w-4" />
              Try again
            </Button>
          </div>
        </div>
      </div>
    );
  }

  const canReact =
    selectedItem !== null &&
    selectedItem.item.channelId !== null &&
    availableChannelIds.has(selectedItem.item.channelId);
  const canReply =
    canReact &&
    selectedItem.item.kind !== 45001 &&
    selectedItem.item.kind !== 45003;
  const disabledReplyReason =
    canReply || !selectedItem
      ? null
      : selectedItem.item.channelId
        ? availableChannelIds.has(selectedItem.item.channelId)
          ? "This item does not support inline replies yet."
          : "Open the linked channel to reply."
        : "This inbox item does not have a reply target.";
  const canDelete =
    selectedItem !== null &&
    currentPubkey?.trim().toLowerCase() ===
      selectedItem.item.pubkey.trim().toLowerCase();
  const isSinglePanelDetailView =
    isMessagesMode &&
    isNarrowHomeViewport &&
    selectedItemId !== null &&
    !isSinglePanelAuxiliaryView;
  const showListPane = !isSinglePanelDetailView && !isSinglePanelAuxiliaryView;
  const showDetailPane =
    isMessagesMode &&
    !isSinglePanelAuxiliaryView &&
    (!isNarrowHomeViewport || isSinglePanelDetailView);
  const auxiliaryPaneWidthPx = isSinglePanelAuxiliaryView
    ? homeInboxWidthPx
    : threadPanelWidthPx;
  const maxEffectiveInboxListWidthPx =
    homeInboxWidthPx > 0
      ? Math.max(
          INBOX_COLUMN_MIN_WIDTH_PX,
          homeInboxWidthPx -
            INBOX_COLUMN_MIN_WIDTH_PX -
            (hasAuxiliaryPane ? auxiliaryPaneWidthPx : 0),
        )
      : undefined;
  const effectiveInboxListWidthPx =
    homeInboxWidthPx > 0
      ? Math.min(
          inboxListWidthPx,
          maxEffectiveInboxListWidthPx ?? inboxListWidthPx,
        )
      : inboxListWidthPx;

  return (
    <ProfilePanelProvider onOpenProfilePanel={handleOpenProfilePanel}>
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
        <div
          className={cn(
            "relative grid min-h-0 w-full flex-1",
            isSinglePanelAuxiliaryView
              ? "grid-cols-1"
              : showListPane && showDetailPane && hasAuxiliaryPane
                ? "grid-cols-[var(--home-inbox-list-width)_minmax(0,1fr)_var(--home-channel-management-width)]"
                : showListPane && showDetailPane
                  ? "grid-cols-[var(--home-inbox-list-width)_minmax(0,1fr)]"
                  : hasAuxiliaryPane
                    ? "grid-cols-[minmax(0,1fr)_var(--home-channel-management-width)]"
                    : "grid-cols-1",
          )}
          data-testid="home-inbox"
          ref={homeInboxRef}
          style={
            {
              "--home-channel-management-width": `${auxiliaryPaneWidthPx}px`,
              "--home-inbox-list-width": `${effectiveInboxListWidthPx}px`,
            } as React.CSSProperties
          }
        >
          {showListPane ? (
            <InboxListPane
              activeReminderEventIds={activeReminderEventIds}
              agentPubkeys={inboxAgentPubkeys}
              doneSet={effectiveDoneSet}
              dueReminderCount={dueReminderCount}
              filter={filter}
              items={filteredItems}
              onFilterChange={setFilter}
              onMarkRead={markItemRead}
              onMarkUnread={markItemUnread}
              onOpenDirect={(item) => {
                const channelId = item.item.channelId;
                if (!channelId) {
                  return;
                }
                onOpenContext(
                  channelId,
                  item.id,
                  getThreadReference(item.item.tags).rootId,
                );
              }}
              onRemindLater={(item) => {
                const channelId = item.item.channelId;
                if (!channelId) {
                  return;
                }
                openReminder({
                  authorPubkey: item.item.pubkey,
                  channelId,
                  eventId: item.id,
                  preview: item.preview.slice(0, 100),
                });
              }}
              onSelect={(itemId) => {
                handleUserSelectItem(itemId);
                markItemRead(itemId);
              }}
              onUnreadOnlyChange={setUnreadOnly}
              reminderPubkey={currentPubkey}
              selectedId={selectedItemId}
              showRightDivider={showListPane && showDetailPane}
              unreadOnly={unreadOnly}
            />
          ) : null}

          <button
            aria-label="Resize inbox list"
            className={cn(
              "group absolute bottom-0 z-40 w-3 -translate-x-1/2 cursor-col-resize",
              topChromeInset.top,
              showListPane && showDetailPane ? "block" : "hidden",
            )}
            data-testid="home-inbox-list-resize-handle"
            onDoubleClick={
              canResetInboxListWidth ? handleInboxListWidthReset : undefined
            }
            onPointerDown={handleInboxListResizeStart}
            style={{ left: `${effectiveInboxListWidthPx}px` }}
            title={
              canResetInboxListWidth
                ? "Drag to resize. Double-click to reset width."
                : "Drag to resize."
            }
            type="button"
          >
            <span className="absolute bottom-0 left-1/2 top-0 w-px -translate-x-1/2 bg-transparent transition-colors group-hover:bg-border/80 group-focus-visible:bg-border/80" />
          </button>

          {showDetailPane ? (
            <InboxDetailPane
              agentPubkeys={inboxAgentPubkeys}
              canDelete={canDelete}
              canOpenChannel={Boolean(
                selectedItem?.item.channelId &&
                  availableChannelIds.has(selectedItem.item.channelId),
              )}
              canReply={canReply}
              channel={selectedChannel}
              contextChannelName={selectedChannel?.name ?? null}
              currentPubkey={currentPubkey}
              disabledReplyReason={disabledReplyReason}
              isDeletingMessage={isDeletingMessage}
              isSendingReply={isSendingReply}
              isSinglePanelView={isSinglePanelDetailView}
              isThreadContextLoading={threadContext.isLoading}
              item={selectedItem}
              messages={contextMessages}
              onBack={
                isSinglePanelDetailView
                  ? () => {
                      handleUserSelectItem(null);
                    }
                  : undefined
              }
              onDelete={() => {
                if (!selectedItem || !canDelete) {
                  return;
                }
                const channelId = selectedItem.item.channelId;
                if (!channelId) {
                  return;
                }

                setIsDeletingMessage(true);
                void deleteMessage(channelId, selectedItem.id)
                  .then(() => {
                    onRefresh();
                  })
                  .finally(() => {
                    setIsDeletingMessage(false);
                  });
              }}
              onOpenChannel={(channelId) => {
                handleCloseProfilePanel();
                setManagedChannelId(channelId);
              }}
              onSendReply={async ({
                content,
                mediaTags,
                mentionPubkeys,
                parentEventId,
              }) => {
                const channelId = selectedItem?.item.channelId;
                if (!selectedItem || !channelId || !canReply) {
                  throw new Error("Replies are not available for this item.");
                }

                const itemToReply = selectedItem;
                setIsSendingReply(true);
                try {
                  const {
                    mediaTags: imetaTags,
                    emojiTags,
                    mentionTags,
                  } = splitOutgoingTags(mediaTags);
                  const result = await sendChannelMessage(
                    channelId,
                    content,
                    parentEventId,
                    imetaTags,
                    mentionPubkeys,
                    undefined,
                    emojiTags,
                    mentionTags,
                  );
                  const authorPubkey = currentPubkey ?? itemToReply.item.pubkey;
                  const reply: InboxReply = {
                    authorLabel: currentPubkey
                      ? resolveUserLabel({
                          currentPubkey,
                          profiles: feedProfiles,
                          pubkey: authorPubkey,
                        })
                      : "You",
                    authorPubkey,
                    avatarUrl:
                      currentPubkey && feedProfiles
                        ? (feedProfiles[currentPubkey.trim().toLowerCase()]
                            ?.avatarUrl ?? null)
                        : null,
                    content,
                    depth: result.depth,
                    fullTimestampLabel: formatInboxFullTimestamp(
                      result.createdAt,
                    ),
                    id: result.eventId,
                    parentId: result.parentEventId,
                    rootId: result.rootEventId,
                    tags: emojiTags,
                    timeLabel: formatTime(result.createdAt),
                  };
                  setLocalRepliesByItemId((current) => ({
                    ...current,
                    [itemToReply.id]: [
                      ...(current[itemToReply.id] ?? []),
                      reply,
                    ],
                  }));
                  onRefresh();
                } finally {
                  setIsSendingReply(false);
                }
              }}
              onToggleReaction={
                canReact
                  ? async (message, emoji, remove) => {
                      await toggleReactionMutation.mutateAsync({
                        emoji,
                        eventId: message.id,
                        remove,
                      });
                      await channelMessagesQuery.refetch();
                      onRefresh();
                    }
                  : undefined
              }
              replies={selectedItemReplies}
            />
          ) : null}
          {profilePanelPubkey ? (
            <RightAuxiliaryPane
              canResetWidth={canResetThreadPanelWidth}
              constrainToAvailableSpace={false}
              onResetWidth={handleThreadPanelWidthReset}
              onResizeStart={handleThreadPanelResizeStart}
              testId="home-user-profile-panel"
              widthPx={auxiliaryPaneWidthPx}
            >
              <UserProfilePanel
                currentPubkey={currentPubkey}
                isSinglePanelView={isSinglePanelAuxiliaryView}
                layout="split"
                onClose={handleCloseProfilePanel}
                onOpenDm={handleOpenDm}
                onOpenProfile={handleOpenProfilePanel}
                onTabChange={handleProfilePanelTabChange}
                onViewChange={handleProfilePanelViewChange}
                pubkey={profilePanelPubkey}
                splitPaneClamp
                tab={profilePanelTab}
                transparentChrome
                view={profilePanelView}
                widthPx={auxiliaryPaneWidthPx}
              />
            </RightAuxiliaryPane>
          ) : isChannelManagementOpen ? (
            <RightAuxiliaryPane
              canResetWidth={canResetThreadPanelWidth}
              constrainToAvailableSpace={false}
              onResetWidth={handleThreadPanelWidthReset}
              onResizeStart={handleThreadPanelResizeStart}
              testId="home-channel-management-auxiliary-pane"
              widthPx={auxiliaryPaneWidthPx}
            >
              <ChannelManagementSheet
                channel={managedChannel}
                currentPubkey={currentPubkey}
                layout="split"
                onOpenChange={(nextOpen) => {
                  if (!nextOpen) {
                    setManagedChannelId(null);
                  }
                }}
                open={true}
              />
            </RightAuxiliaryPane>
          ) : null}
        </div>
      </div>
    </ProfilePanelProvider>
  );
}
