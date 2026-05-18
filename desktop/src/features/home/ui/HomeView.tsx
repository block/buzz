import * as React from "react";
import { RefreshCcw } from "lucide-react";

import { useChannelsQuery } from "@/features/channels/hooks";
import {
  type InboxFilter,
  type InboxContextMessage,
  type InboxReply,
  buildInboxItems,
  formatInboxFullTimestamp,
} from "@/features/home/lib/inbox";
import { useFeedItemState } from "@/features/home/useFeedItemState";
import { useInboxThreadContext } from "@/features/home/useInboxThreadContext";
import { useResizableInboxListWidth } from "@/features/home/useResizableInboxListWidth";
import { InboxDetailPane } from "@/features/home/ui/InboxDetailPane";
import { InboxListPane } from "@/features/home/ui/InboxListPane";
import {
  useChannelMessagesQuery,
  useToggleReactionMutation,
} from "@/features/messages/hooks";
import { formatTimelineMessages } from "@/features/messages/lib/formatTimelineMessages";
import { getThreadReference } from "@/features/messages/lib/threading";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { resolveUserLabel } from "@/features/profile/lib/identity";
import { deleteMessage, sendChannelMessage } from "@/shared/api/tauri";
import type { HomeFeedResponse, RelayEvent } from "@/shared/api/types";
import { KIND_REACTION } from "@/shared/constants/kinds";
import { resolveMentionNames } from "@/shared/lib/resolveMentionNames";
import { Button } from "@/shared/ui/button";
import { Skeleton } from "@/shared/ui/skeleton";

function matchesInboxFilter(
  item: { categories: InboxFilter[] },
  filter: InboxFilter,
) {
  if (filter === "all") {
    return item.categories.some((category) => category !== "activity");
  }

  return item.categories.includes(filter);
}

function HomeLoadingState() {
  return (
    <div className="flex-1 overflow-hidden">
      <div className="grid h-full min-h-0 w-full lg:grid-cols-[320px_minmax(0,1fr)]">
        <div className="overflow-hidden border-r border-border/70 bg-background/60">
          <div className="border-b border-border/70 px-4 pb-4 pt-14">
            <Skeleton className="h-4 w-20" />
            <Skeleton className="mt-2 h-4 w-28" />
            <Skeleton className="mt-4 h-10 rounded-md" />
          </div>
          <div className="space-y-3 px-4 py-4">
            {["a", "b", "c", "d"].map((row) => (
              <Skeleton className="h-20 rounded-md" key={row} />
            ))}
          </div>
        </div>

        <div className="overflow-hidden bg-background/60">
          <div className="border-b border-border/70 px-5 pb-4 pt-14">
            <Skeleton className="h-5 w-48" />
            <Skeleton className="mt-3 h-8 w-72" />
          </div>
          <div className="px-5 py-5">
            <Skeleton className="h-64 rounded-md" />
          </div>
        </div>
      </div>
    </div>
  );
}

function getContextMessageDepth(
  event: RelayEvent,
  eventById: ReadonlyMap<string, RelayEvent>,
): number {
  let depth = 0;
  let parentId = getThreadReference(event.tags).parentId;
  const seen = new Set<string>([event.id]);

  while (parentId && eventById.has(parentId) && !seen.has(parentId)) {
    depth += 1;
    seen.add(parentId);
    parentId = getThreadReference(eventById.get(parentId)?.tags ?? []).parentId;
  }

  return depth;
}

function getReactionTargetId(tags: string[][]) {
  for (let index = tags.length - 1; index >= 0; index -= 1) {
    const tag = tags[index];
    if (tag?.[0] === "e" && typeof tag[1] === "string") {
      return tag[1];
    }
  }

  return null;
}

type HomeViewProps = {
  feed?: HomeFeedResponse;
  isLoading?: boolean;
  errorMessage?: string;
  currentPubkey?: string;
  availableChannelIds: ReadonlySet<string>;
  onOpenContext: (channelId: string, messageId: string) => void;
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

    if (!filteredItems.some((item) => item.id === selectedItemId)) {
      setSelectedItemId(filteredItems[0]?.id ?? null);
    }
  }, [filteredItems, selectedItemId]);

  React.useEffect(() => {
    void selectedItemId;
    setIsDeletingMessage(false);
    setIsSendingReply(false);
  }, [selectedItemId]);

  const handleToggleDone = React.useCallback(
    (itemId: string) => {
      if (doneSet.has(itemId)) {
        undoDone(itemId);
        return;
      }

      markDone(itemId);
    },
    [doneSet, markDone, undoDone],
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

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <div
        className="relative grid min-h-0 flex-1 w-full lg:grid-cols-[var(--home-inbox-list-width)_minmax(0,1fr)]"
        data-testid="home-inbox"
        style={
          {
            "--home-inbox-list-width": `${inboxListWidthPx}px`,
          } as React.CSSProperties
        }
      >
        <InboxListPane
          doneSet={doneSet}
          filter={filter}
          items={filteredItems}
          onFilterChange={setFilter}
          onSelect={(itemId) => {
            setSelectedItemId(itemId);
            markDone(itemId);
          }}
          selectedId={selectedItemId}
        />

        <button
          aria-label="Resize inbox list"
          className="group absolute inset-y-0 z-20 hidden w-3 -translate-x-1/2 cursor-col-resize lg:block"
          data-testid="home-inbox-list-resize-handle"
          onDoubleClick={
            canResetInboxListWidth ? handleInboxListWidthReset : undefined
          }
          onPointerDown={handleInboxListResizeStart}
          style={{ left: `${inboxListWidthPx}px` }}
          title={
            canResetInboxListWidth
              ? "Drag to resize. Double-click to reset width."
              : "Drag to resize."
          }
          type="button"
        >
          <span className="absolute inset-y-0 left-1/2 w-px -translate-x-1/2 bg-transparent transition-colors group-hover:bg-border/80 group-focus-visible:bg-border/80" />
        </button>

        <InboxDetailPane
          canDelete={canDelete}
          canOpenChannel={Boolean(
            selectedItem?.item.channelId &&
              availableChannelIds.has(selectedItem.item.channelId),
          )}
          canReply={canReply}
          disabledReplyReason={disabledReplyReason}
          isDone={selectedItem ? doneSet.has(selectedItem.id) : false}
          isDeletingMessage={isDeletingMessage}
          isSendingReply={isSendingReply}
          isThreadContextLoading={threadContext.isLoading}
          item={selectedItem}
          messages={contextMessages}
          replies={selectedItemReplies}
          contextChannelName={selectedChannel?.name ?? null}
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
                fullTimestampLabel: formatInboxFullTimestamp(result.createdAt),
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
          onToggleDone={() => {
            if (selectedItem) {
              handleToggleDone(selectedItem.id);
            }
          }}
        />
      </div>
    </div>
  );
}
