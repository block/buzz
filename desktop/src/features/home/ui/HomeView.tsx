import * as React from "react";
import { RefreshCcw } from "lucide-react";

import { useAppShell } from "@/app/AppShellContext";
import { useChannelsQuery } from "@/features/channels/hooks";
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
import { useFeedItemState } from "@/features/home/useFeedItemState";
import { useHomeInboxReadState } from "@/features/home/useHomeInboxReadState";
import { useInboxThreadContext } from "@/features/home/useInboxThreadContext";
import {
  INBOX_COLUMN_MIN_WIDTH_PX,
  INBOX_SINGLE_COLUMN_BREAKPOINT_PX,
  useResizableInboxListWidth,
} from "@/features/home/useResizableInboxListWidth";
import { HomeChannelActions } from "@/features/home/ui/HomeChannelActions";
import { HomeLoadingState } from "@/features/home/ui/HomeLoadingState";
import { InboxDetailPane } from "@/features/home/ui/InboxDetailPane";
import { InboxListPane } from "@/features/home/ui/InboxListPane";
import {
  useChannelMessagesQuery,
  useToggleReactionMutation,
} from "@/features/messages/hooks";
import { formatTimelineMessages } from "@/features/messages/lib/formatTimelineMessages";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { deleteMessage, sendChannelMessage } from "@/shared/api/tauri";
import type { HomeFeedResponse } from "@/shared/api/types";
import { KIND_REACTION } from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import { resolveMentionNames } from "@/shared/lib/resolveMentionNames";
import { useElementWidth } from "@/shared/hooks/use-mobile";
import { Button } from "@/shared/ui/button";

type HomeViewProps = {
  feed?: HomeFeedResponse;
  isLoading?: boolean;
  errorMessage?: string;
  currentPubkey?: string;
  availableChannelIds: ReadonlySet<string>;
  onOpenChannel: (channelId: string) => void;
  onOpenContext: (channelId: string, messageId: string) => void;
  onRefresh: () => void;
};

export function HomeView({
  feed,
  isLoading = false,
  errorMessage,
  currentPubkey,
  availableChannelIds,
  onOpenChannel,
  onOpenContext,
  onRefresh,
}: HomeViewProps) {
  const [homeInboxRef, homeInboxWidthPx] = useElementWidth<HTMLDivElement>();
  const isNarrowHomeViewport =
    homeInboxWidthPx > 0 &&
    homeInboxWidthPx < INBOX_SINGLE_COLUMN_BREAKPOINT_PX;
  const [filter, setFilter] = React.useState<InboxFilter>("all");
  const [selectedItemId, setSelectedItemId] = React.useState<string | null>(
    null,
  );
  const [isDeletingMessage, setIsDeletingMessage] = React.useState(false);
  const [isSendingReply, setIsSendingReply] = React.useState(false);
  const [localRepliesByItemId, setLocalRepliesByItemId] = React.useState<
    Record<string, InboxReply[]>
  >({});
  const {
    canResetInboxListWidth,
    handleInboxListResizeStart,
    handleInboxListWidthReset,
    inboxListWidthPx,
  } = useResizableInboxListWidth();
  const { doneSet, markDone, undoDone } = useFeedItemState(currentPubkey);
  const {
    getChannelReadAt,
    markChannelRead,
    markChannelUnread,
    readStateVersion,
  } = useAppShell();
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
        ...threadContext.events.map((event) => event.pubkey),
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
  const inboxItems = React.useMemo(
    () =>
      buildInboxItems({
        currentPubkey,
        feed,
        profiles: feedProfiles,
      }),
    [currentPubkey, feed, feedProfiles],
  );
  const { effectiveDoneSet, markItemRead, markItemUnread } =
    useHomeInboxReadState({
      items: inboxItems,
      getChannelReadAt,
      readStateVersion,
      localDoneSet: doneSet,
      markChannelRead,
      markChannelUnread,
      markDoneLocal: markDone,
      undoDoneLocal: undoDone,
    });
  const filteredItems = React.useMemo(() => {
    return inboxItems.filter((item) => matchesInboxFilter(item, filter));
  }, [filter, inboxItems]);
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
      return {
        id: message.id,
        authorLabel: message.author,
        avatarUrl: message.avatarUrl ?? null,
        content: message.body,
        depth: event ? getContextMessageDepth(event, eventById) : message.depth,
        fullTimestampLabel: formatInboxFullTimestamp(message.createdAt),
        isSelected: message.id === selectedItem.id,
        mentionNames:
          resolveMentionNames(message.tags ?? [], feedProfiles) ?? [],
        reactions: message.reactions,
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
  }, [filteredItems, homeInboxWidthPx, isNarrowHomeViewport, selectedItemId]);

  React.useEffect(() => {
    void selectedItemId;
    setIsDeletingMessage(false);
    setIsSendingReply(false);
  }, [selectedItemId]);

  const handleToggleDone = React.useCallback(
    (itemId: string) => {
      if (effectiveDoneSet.has(itemId)) {
        markItemUnread(itemId);
        return;
      }

      markItemRead(itemId);
    },
    [effectiveDoneSet, markItemRead, markItemUnread],
  );

  if (isLoading && !feed) {
    return <HomeLoadingState />;
  }

  if (!feed) {
    return (
      <div className="flex-1 overflow-hidden px-4 pb-3 pt-14 sm:px-6">
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
    isNarrowHomeViewport && selectedItemId !== null;
  const showListPane = !isSinglePanelDetailView;
  const showDetailPane = !isNarrowHomeViewport || isSinglePanelDetailView;
  const maxEffectiveInboxListWidthPx =
    homeInboxWidthPx > 0
      ? Math.max(
          INBOX_COLUMN_MIN_WIDTH_PX,
          homeInboxWidthPx - INBOX_COLUMN_MIN_WIDTH_PX,
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
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <HomeChannelActions
        channel={selectedChannel}
        currentPubkey={currentPubkey}
        onOpenChannel={onOpenChannel}
      />
      <div
        className={cn(
          "relative grid min-h-0 w-full flex-1",
          showListPane && showDetailPane
            ? "grid-cols-[var(--home-inbox-list-width)_minmax(0,1fr)]"
            : "grid-cols-1",
        )}
        data-testid="home-inbox"
        ref={homeInboxRef}
        style={
          {
            "--home-inbox-list-width": `${effectiveInboxListWidthPx}px`,
          } as React.CSSProperties
        }
      >
        {showListPane ? (
          <InboxListPane
            doneSet={effectiveDoneSet}
            filter={filter}
            items={filteredItems}
            onFilterChange={setFilter}
            onSelect={(itemId) => {
              setSelectedItemId(itemId);
              markItemRead(itemId);
            }}
            selectedId={selectedItemId}
            showRightDivider={showListPane && showDetailPane}
          />
        ) : null}

        <button
          aria-label="Resize inbox list"
          className={cn(
            "group absolute inset-y-0 z-40 w-3 -translate-x-1/2 cursor-col-resize",
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
          <span className="absolute bottom-0 left-1/2 top-10 w-px -translate-x-1/2 bg-transparent transition-colors group-hover:bg-border/80 group-focus-visible:bg-border/80" />
        </button>

        {showDetailPane ? (
          <InboxDetailPane
            canDelete={canDelete}
            canOpenChannel={Boolean(
              selectedItem?.item.channelId &&
                availableChannelIds.has(selectedItem.item.channelId),
            )}
            canReply={canReply}
            contextChannelName={selectedChannel?.name ?? null}
            disabledReplyReason={disabledReplyReason}
            isDeletingMessage={isDeletingMessage}
            isDone={
              selectedItem ? effectiveDoneSet.has(selectedItem.id) : false
            }
            isSendingReply={isSendingReply}
            isSinglePanelView={isSinglePanelDetailView}
            isThreadContextLoading={threadContext.isLoading}
            item={selectedItem}
            messages={contextMessages}
            onBack={
              isSinglePanelDetailView
                ? () => {
                    setSelectedItemId(null);
                  }
                : undefined
            }
            onDelete={() => {
              if (!selectedItem || !canDelete) {
                return;
              }

              setIsDeletingMessage(true);
              void deleteMessage(selectedItem.id)
                .then(() => {
                  onRefresh();
                })
                .finally(() => {
                  setIsDeletingMessage(false);
                });
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
                const result = await sendChannelMessage(
                  channelId,
                  content,
                  parentEventId,
                  mediaTags,
                  mentionPubkeys,
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
                };
                setLocalRepliesByItemId((current) => ({
                  ...current,
                  [itemToReply.id]: [...(current[itemToReply.id] ?? []), reply],
                }));
                onRefresh();
              } finally {
                setIsSendingReply(false);
              }
            }}
            onToggleDone={() => {
              if (selectedItem) {
                handleToggleDone(selectedItem.id);
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
      </div>
    </div>
  );
}
