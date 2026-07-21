import * as React from "react";
import { RefreshCcw } from "lucide-react";

import { useAppShell } from "@/app/AppShellContext";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useKnownAgentPubkeys } from "@/features/agents/useKnownAgentPubkeys";
import { useChannelsQuery, useOpenDmMutation } from "@/features/channels/hooks";
import { RightAuxiliaryPane } from "@/features/channels/ui/RightAuxiliaryPane";
import { ChannelManagementSheet } from "@/features/channels/ui/ChannelManagementSheet";
import {
  type InboxContextMessage,
  type InboxItem,
  type InboxReply,
  buildInboxItems,
  formatInboxFullTimestamp,
  getInboxConversationId,
} from "@/features/home/lib/inbox";
import { useInboxSelectionAnchor } from "@/features/home/useInboxSelectionAnchor";
import { useActivityInboxFilter } from "@/features/home/useActivityInboxFilter";
import { useOwnedAgentPubkeys } from "@/features/home/useOwnedAgentPubkeys";
import {
  filterActivityInboxItems,
  getReactionTargetId,
  matchesInboxFilter,
  toInboxContextMessage,
} from "@/features/home/lib/inboxViewHelpers";
import { useHomeInboxReadState } from "@/features/home/useHomeInboxReadState";
import { useHomePersonalActivity } from "@/features/home/useHomePersonalActivity";
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
  INBOX_SINGLE_COLUMN_BREAKPOINT_PX,
  useResizableInboxListWidth,
} from "@/features/home/useResizableInboxListWidth";
import { getHomePaneLayout } from "@/features/home/lib/homePaneLayout";
import { getHomeMessageCapabilities } from "@/features/home/lib/homeMessageCapabilities";
import { HomeLoadingState } from "@/features/home/ui/HomeLoadingState";
import { InboxDetailPane } from "@/features/home/ui/InboxDetailPane";
import { InboxListPane } from "@/features/home/ui/InboxListPane";
import { HomePersonalActivityDetail } from "@/features/home/ui/HomePersonalActivityDetail";
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
import { useRelaySelfQuery } from "@/features/moderation/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { useRemindLater } from "@/features/reminders/ui/RemindMeLaterProvider";
import { deleteMessage, sendChannelMessage } from "@/shared/api/tauri";
import type { HomeFeedResponse } from "@/shared/api/types";
import { KIND_REACTION } from "@/shared/constants/kinds";
import { topChromeInset } from "@/shared/layout/chromeLayout";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
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

/**
 * Finds the InboxItem whose stable conversation contains the given event ID.
 * Checks `item.id` (the current representative/latest event) first, then
 * falls back to `item.groupItems` so that a deep-linked or URL-anchored event
 * that is no longer the representative still resolves to its row.
 */
function findItemByEventId(
  items: readonly InboxItem[],
  eventId: string,
): InboxItem | null {
  // Fast path: representative event matches (the common case).
  const direct = items.find((item) => item.id === eventId);
  if (direct) return direct;
  // Slow path: event is a non-representative group member (e.g. original
  // mention that was later superseded by a newer reply as the representative).
  return (
    items.find((item) => item.groupItems.some((gi) => gi.id === eventId)) ??
    null
  );
}

type HomeViewProps = {
  activityEnabled: boolean;
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
  activityEnabled,
  feed,
  isLoading = false,
  errorMessage,
  currentPubkey,
  availableChannelIds,
  onOpenContext,
  onRefresh,
}: HomeViewProps) {
  const relaySelfPubkey = useRelaySelfQuery().data;
  const [homeInboxRef, homeInboxWidthPx] = useElementWidth<HTMLDivElement>();
  const isNarrowHomeViewport =
    homeInboxWidthPx > 0 &&
    homeInboxWidthPx < INBOX_SINGLE_COLUMN_BREAKPOINT_PX;
  const [filter, setFilter] = useActivityInboxFilter(activityEnabled);
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
  const {
    drafts: {
      activeCount: activeDraftCount,
      deleteDraft: handleDeleteDraft,
      items: draftItems,
      selectedItem: selectedDraftItem,
      selectedKey: selectedDraftKey,
      selectDraft: setSelectedDraftKey,
    },
    dueReminderCount,
    pendingReminders,
    reminders: {
      selectedId: selectedReminderId,
      selectedItem: selectedReminder,
      select: setSelectedReminderId,
    },
  } = useHomePersonalActivity({
    activityEnabled,
    currentPubkey,
    isDrafts,
    isNarrowHomeViewport,
    isReminders,
    viewportWidthPx: homeInboxWidthPx,
  });
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
  // Selection state — two-tier design so explicit and automatic selections
  // have distinct ownership:
  //
  //   urlSelectedItemId  — explicit/user anchor, URL-authoritative.  Written
  //     only by handleUserSelectItem (via applyInboxSearchPatch) and by
  //     back/forward navigation.  Never touched by background data loads.
  //
  //   autoSelectedEventId — default desktop selection when the URL carries no
  //     explicit anchor.  Written only by the auto-selection effect.  Never
  //     triggers a history push.
  //
  //   selectedEventId — the effective anchor used everywhere below: the URL
  //     anchor when present, otherwise the auto-selected fallback.  Derived
  //     synchronously, no separate state — so there is no mirror-revert race.
  const [autoSelectedEventId, setAutoSelectedEventId] = React.useState<
    string | null
  >(null);
  const selectedEventId = urlSelectedItemId ?? autoSelectedEventId;
  const [managedChannelId, setManagedChannelId] = React.useState<string | null>(
    null,
  );
  const { goChannel } = useAppNavigation();
  const openDmMutation = useOpenDmMutation();
  // handleUserSelectItem: explicit selection — only patches the URL.
  // No local setSelectedEventId call; the URL patch triggers a TanStack Router
  // navigation which updates urlSelectedItemId, which becomes selectedEventId
  // on the next render.  This avoids the mirror-revert race where
  // useEffect([urlSelectedItemId]) would fire before navigation commits and
  // overwrite the optimistically-set local state with the stale URL null.
  const handleUserSelectItem = React.useCallback(
    (itemId: string | null) => {
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
  const { feedItems, activeLatchedItem, coldResolutionPending } =
    useInboxSelectionAnchor({
      feed,
      selectedEventId,
      availableChannelIds,
    });

  const threadContextFeedItem = activeLatchedItem;
  // Derive the default composer parent from the active anchor's own tags so
  // that InboxDetailPane can recover the original reply target even when the
  // anchor event has been displaced from the current groupItems. This is null
  // until the active item is resolved (anchor not yet found in feedItems and
  // no matching committed latch).
  const latchedDefaultParentId =
    activeLatchedItem !== null
      ? (getThreadReference(activeLatchedItem.tags).parentId ??
        activeLatchedItem.id)
      : null;

  const channelsQuery = useChannelsQuery();
  const channels = channelsQuery.data;
  const selectedChannelIdCandidate = React.useMemo(() => {
    return threadContextFeedItem?.channelId ?? null;
  }, [threadContextFeedItem]);
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
    threadContextFeedItem,
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
  const ownedAgentPubkeys = useOwnedAgentPubkeys(
    activityEnabled,
    feedProfiles,
    currentPubkey,
  );
  const feedOwnerPubkeys = React.useMemo(
    () => [
      ...new Set(
        Object.values(feedProfiles ?? {})
          .map((profile) => profile.ownerPubkey)
          .filter((pubkey): pubkey is string => Boolean(pubkey)),
      ),
    ],
    [feedProfiles],
  );
  const feedOwnerProfilesQuery = useUsersBatchQuery(feedOwnerPubkeys, {
    enabled: feedOwnerPubkeys.length > 0,
  });
  const feedOwnerProfiles = feedOwnerProfilesQuery.data?.profiles;
  // Agent set for the inbox list/detail bot badges: the community-scoped
  // baseline widened with this surface's profile lookup.
  const communityAgentPubkeys = useKnownAgentPubkeys();
  const inboxAgentPubkeys = React.useMemo(() => {
    const pubkeys = new Set(communityAgentPubkeys);

    for (const [pubkey, profile] of Object.entries(feedProfiles ?? {})) {
      if (profile.isAgent) {
        pubkeys.add(normalizePubkey(pubkey));
      }
    }

    return pubkeys;
  }, [feedProfiles, communityAgentPubkeys]);
  const inboxItems = React.useMemo(() => {
    const items = buildInboxItems({
      channels,
      currentPubkey,
      feed,
      profiles: feedProfiles,
    });
    return filterActivityInboxItems(items, activityEnabled);
  }, [activityEnabled, channels, currentPubkey, feed, feedProfiles]);
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
  // Resolve the selected row and stable conversation ID from inboxItems
  // (unfiltered). We need conversationId before filtering so we can keep the
  // selected item visible when unreadOnly is on. The event anchor may point to
  // any event in the group (representative or older member), so search both.
  const selectedItemFromAll = React.useMemo(
    () =>
      selectedEventId
        ? (findItemByEventId(inboxItems, selectedEventId) ?? null)
        : null,
    [inboxItems, selectedEventId],
  );
  // selectedConversationId: prefer the InboxItem-derived conversationId (stable
  // group key). Fall back to deriving it from the latched FeedItem when the
  // anchored event is no longer present in any group's items — this keeps the
  // correct row selected (by conversationId) even after the anchor event has
  // been displaced from groupItems by a newer representative.
  const latchedConversationId = activeLatchedItem
    ? getInboxConversationId(activeLatchedItem.tags, activeLatchedItem.id)
    : null;
  const selectedConversationId =
    selectedItemFromAll?.conversationId ?? latchedConversationId;

  const filteredItems = React.useMemo(() => {
    return inboxItems.filter(
      (item) =>
        matchesInboxFilter(
          item,
          filter,
          activityEnabled ? ownedAgentPubkeys : undefined,
        ) &&
        (!unreadOnly ||
          !effectiveDoneSet.has(item.id) ||
          item.conversationId === selectedConversationId),
    );
  }, [
    effectiveDoneSet,
    activityEnabled,
    filter,
    inboxItems,
    ownedAgentPubkeys,
    selectedConversationId,
    unreadOnly,
  ]);
  // Prefer the filtered view for the selected item so that filter/unread
  // changes can still dismiss it, but fall back to the unfiltered row so a
  // live representative-event change (which keeps the conversation in the
  // filter) does not make selectedItem go null mid-session.
  const selectedItem = React.useMemo(() => {
    if (!selectedEventId) return null;
    // Primary: find by event anchor in the filtered view.
    const fromFiltered = findItemByEventId(filteredItems, selectedEventId);
    if (fromFiltered) return fromFiltered;
    // Secondary: event anchor is in an unfiltered row (e.g., dismissed item).
    if (selectedItemFromAll) return selectedItemFromAll;
    // Tertiary: anchor has been displaced from all groupItems (e.g., a very old
    // event that fell off the feed window). Resolve by conversationId so the
    // correct row stays selected and the auto-selection effect doesn't replace
    // the anchor with a different conversation.
    if (selectedConversationId) {
      return (
        filteredItems.find(
          (item) => item.conversationId === selectedConversationId,
        ) ??
        inboxItems.find(
          (item) => item.conversationId === selectedConversationId,
        ) ??
        null
      );
    }
    return null;
  }, [
    filteredItems,
    inboxItems,
    selectedConversationId,
    selectedEventId,
    selectedItemFromAll,
  ]);
  const contextMessages = React.useMemo<InboxContextMessage[]>(() => {
    if (!selectedItem) {
      return [];
    }

    const eventById = new Map(
      threadContext.events.map((event) => [event.id, event]),
    );
    const contextEventIds = new Set(eventById.keys());
    const reactionEvents = [
      ...(channelMessages ?? []),
      ...threadContext.reactionEvents,
    ].filter((event) => {
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
      undefined,
      undefined,
      undefined,
      relaySelfPubkey,
      feedOwnerProfiles,
    );

    return timelineMessages.map((message) =>
      toInboxContextMessage(message, {
        eventById,
        fallbackAuthorPubkey: selectedItem.item.pubkey,
        profiles: feedProfiles,
        selectedItemId: selectedEventId ?? selectedItem.id,
      }),
    );
  }, [
    channelMessages,
    currentPubkey,
    feedProfiles,
    feedOwnerProfiles,
    relaySelfPubkey,
    selectedChannel,
    selectedEventId,
    selectedItem,
    threadContext.events,
    threadContext.reactionEvents,
  ]);
  const selectedItemReplies = React.useMemo<InboxReply[]>(() => {
    if (!selectedItem) return [];
    const localReplies =
      localRepliesByItemId[selectedItem.conversationId] ?? [];
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

    // The URL carries an explicit anchor — auto-selection must not overwrite
    // it. Clear any stale auto fallback so it cannot reappear if back later
    // returns to a no-item entry.
    if (urlSelectedItemId !== null) {
      setAutoSelectedEventId(null);
      return;
    }

    // While the feed is loading (e.g. a reload restoring `?item=` from the
    // URL) the selected item simply hasn't arrived yet — don't clobber it.
    if (isLoading || !feed) {
      return;
    }

    if (filteredItems.length === 0) {
      setAutoSelectedEventId(null);
      return;
    }

    // Don't default-select before the width is measured: at width 0
    // isNarrowHomeViewport is false, so narrow Home would cold-load into detail.
    if (homeInboxWidthPx === 0) {
      return;
    }

    // The event anchor is still valid if the conversation it belongs to is
    // still present in the filtered list. A live representative-event change
    // does NOT invalidate the anchor (the same conversationId is still there).
    if (
      selectedConversationId !== null &&
      filteredItems.some(
        (item) => item.conversationId === selectedConversationId,
      )
    ) {
      return;
    }

    // A cold URL anchor is being resolved via getEventById — the user navigated
    // to a specific event that is not yet in the inbox list. Do not overwrite
    // selectedEventId; wait for cold recovery to commit before auto-selecting.
    if (coldResolutionPending) {
      return;
    }

    setAutoSelectedEventId(
      isNarrowHomeViewport ? null : (filteredItems[0]?.id ?? null),
    );
  }, [
    coldResolutionPending,
    feed,
    filteredItems,
    homeInboxWidthPx,
    isLoading,
    isMessagesMode,
    isNarrowHomeViewport,
    selectedConversationId,
    urlSelectedItemId,
  ]);

  React.useEffect(() => {
    void selectedConversationId;
    setIsDeletingMessage(false);
    setIsSendingReply(false);
  }, [selectedConversationId]);

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

  const { canDelete, canReact, canReply, disabledReplyReason } =
    getHomeMessageCapabilities(
      selectedItem,
      currentPubkey,
      availableChannelIds,
    );
  const {
    auxiliaryPaneWidthPx,
    effectiveInboxListWidthPx,
    isSinglePanelDetailView,
    isSinglePanelDraftDetailView,
    isSinglePanelReminderDetailView,
    showDetailPane,
    showListPane,
  } = getHomePaneLayout({
    activityEnabled,
    hasAuxiliaryPane,
    homeWidthPx: homeInboxWidthPx,
    inboxListWidthPx,
    isDrafts,
    isMessagesMode,
    isNarrow: isNarrowHomeViewport,
    isReminders,
    isSinglePanelAuxiliaryView,
    selectedDraft: selectedDraftItem !== null,
    selectedEvent: selectedEventId !== null,
    selectedReminder: selectedReminder !== null,
    threadPanelWidthPx,
  });

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
          {showListPane || showDetailPane ? (
            <div
              aria-hidden="true"
              className="pointer-events-none absolute inset-x-0 top-0 z-30 h-13 bg-background/80 backdrop-blur-md supports-backdrop-filter:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-backdrop-filter:bg-background/55"
              data-testid="home-inbox-shared-header-backdrop"
            />
          ) : null}

          {showListPane ? (
            <InboxListPane
              activityEnabled={activityEnabled}
              activeReminderEventIds={activeReminderEventIds}
              agentPubkeys={inboxAgentPubkeys}
              activeDraftCount={activeDraftCount}
              draftItems={draftItems}
              doneSet={effectiveDoneSet}
              dueReminderCount={dueReminderCount}
              filter={filter}
              items={filteredItems}
              onDeleteDraft={handleDeleteDraft}
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
              onSelectDraft={setSelectedDraftKey}
              onSelectReminder={setSelectedReminderId}
              onUnreadOnlyChange={setUnreadOnly}
              reminderPubkey={currentPubkey}
              reminders={pendingReminders}
              selectedConversationId={selectedConversationId}
              selectedDraftKey={selectedDraftKey}
              selectedReminderId={selectedReminderId}
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

          {showDetailPane && isMessagesMode ? (
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
              latchedDefaultParentId={latchedDefaultParentId}
              messages={contextMessages}
              profiles={feedProfiles}
              selectedEventId={selectedEventId}
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
              onManageChannel={(channelId) => {
                handleCloseProfilePanel();
                setManagedChannelId(channelId);
              }}
              onOpenContext={onOpenContext}
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
                    createdAt: result.createdAt,
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
                    [itemToReply.conversationId]: [
                      ...(current[itemToReply.conversationId] ?? []),
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
                      await threadContext.refreshReactions();
                      await channelMessagesQuery.refetch();
                      onRefresh();
                    }
                  : undefined
              }
              replies={selectedItemReplies}
            />
          ) : null}
          {showDetailPane && (isDrafts || isReminders) ? (
            <HomePersonalActivityDetail
              currentPubkey={currentPubkey}
              draftItem={selectedDraftItem}
              mode={isDrafts ? "drafts" : "reminders"}
              onBack={
                isSinglePanelDraftDetailView
                  ? () => setSelectedDraftKey(null)
                  : isSinglePanelReminderDetailView
                    ? () => setSelectedReminderId(null)
                    : undefined
              }
              onDeleteDraft={handleDeleteDraft}
              reminder={selectedReminder}
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
