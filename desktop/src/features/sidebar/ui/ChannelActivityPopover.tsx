import * as React from "react";
import { Clock, Loader2, MailOpen } from "lucide-react";

import { useAppShell } from "@/app/AppShellContext";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import type { ActiveChannelTurnSummary } from "@/features/agents/activeAgentTurnsStore";
import { formatElapsed } from "@/features/agents/ui/agentSessionUtils";
import { useOpenAgentActivity } from "@/features/agents/useOpenAgentActivity";
import { buildInboxItems, type InboxItem } from "@/features/home/lib/inbox";
import { getGroupedInboxItemIds } from "@/features/home/useHomeInboxReadState";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { useRemindLater } from "@/features/reminders/ui/RemindMeLaterProvider";
import { useIdentityQuery } from "@/shared/api/hooks";
import type { Channel, FeedItem, HomeFeedResponse } from "@/shared/api/types";
import { normalizePubkey, truncatePubkey } from "@/shared/lib/pubkey";
import { useNow } from "@/shared/lib/useNow";
import { Markdown } from "@/shared/ui/markdown";
import {
  DEFAULT_POPOVER_HOVER_OPEN_DELAY_MS,
  Popover,
  PopoverAnchor,
  PopoverContent,
} from "@/shared/ui/popover";
import { UserAvatar } from "@/shared/ui/UserAvatar";

const HOVER_CLOSE_DELAY_MS = 180;
const ACTIVITY_POPOVER_MOTION_STYLE = {
  "--tw-enter-scale": "1",
  "--tw-exit-scale": "1",
} as React.CSSProperties;

function buildChannelActivityFeed(items: FeedItem[]): HomeFeedResponse {
  return {
    feed: {
      mentions: items.filter((item) => item.category === "mention"),
      needsAction: items.filter((item) => item.category === "needs_action"),
      activity: items.filter(
        (item) =>
          item.category !== "mention" &&
          item.category !== "needs_action" &&
          item.category !== "agent_activity",
      ),
      agentActivity: items.filter((item) => item.category === "agent_activity"),
    },
    meta: {
      since: 0,
      total: items.length,
      generatedAt: 0,
    },
  };
}

function RowActionButton({
  children,
  label,
  onClick,
}: {
  children: React.ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      aria-label={label}
      className="inline-flex size-7 items-center justify-center rounded-full text-muted-foreground transition-colors hover:bg-background/80 hover:text-foreground focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring [&>svg]:size-4"
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        onClick();
      }}
      title={label}
      type="button"
    >
      {children}
    </button>
  );
}

function ThreadPreviewRow({
  isAgent,
  item,
  onMarkRead,
  onOpen,
  onRemindLater,
}: {
  isAgent: boolean;
  item: InboxItem;
  onMarkRead: () => void;
  onOpen: () => void;
  onRemindLater: () => void;
}) {
  return (
    <div
      className="group/activity-row relative border-t border-border/50 first:border-t-0"
      data-testid={`channel-activity-item-${item.conversationId}`}
    >
      <button
        aria-label={`Open thread from ${item.senderLabel}`}
        className="absolute inset-0 z-0 w-full text-left"
        onClick={onOpen}
        type="button"
      />
      <div className="pointer-events-none relative z-10 flex min-w-0 items-start gap-2.5 px-3 py-3 transition-colors group-hover/activity-row:bg-muted/50 group-focus-within/activity-row:bg-muted/50">
        <UserAvatar
          avatarUrl={item.avatarUrl}
          className="h-9 w-9 shrink-0"
          displayName={item.senderLabel}
          shape={isAgent ? "squircle" : "circle"}
          size="md"
        />
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-start gap-2">
            <span className="min-w-0 flex-1 truncate text-sm font-semibold leading-4 text-foreground">
              {item.senderLabel}
            </span>
            <span className="shrink-0 text-xs leading-4 text-muted-foreground/70 transition-opacity group-hover/activity-row:opacity-0 group-focus-within/activity-row:opacity-0">
              {item.timestampLabel}
            </span>
          </div>
          <div className="mt-0.5 flex items-center gap-1.5 text-xs leading-4 text-muted-foreground">
            <span>Thread</span>
            {item.unreadCount > 1 ? (
              <>
                <span aria-hidden="true">·</span>
                <span>{item.unreadCount} unread</span>
              </>
            ) : null}
          </div>
          <Markdown
            className="inbox-preview-markdown mt-1.5 line-clamp-2 text-sm font-medium leading-5 text-foreground"
            content={item.preview}
            interactive={false}
            mentionNames={item.mentionNames}
          />
        </div>
      </div>
      <div className="pointer-events-none absolute right-2 top-2 z-20 flex items-center gap-0.5 rounded-full bg-muted/95 p-0.5 opacity-0 shadow-xs transition-opacity group-hover/activity-row:pointer-events-auto group-hover/activity-row:opacity-100 group-focus-within/activity-row:pointer-events-auto group-focus-within/activity-row:opacity-100">
        <RowActionButton label="Mark as read" onClick={onMarkRead}>
          <MailOpen />
        </RowActionButton>
        <RowActionButton label="Remind me later" onClick={onRemindLater}>
          <Clock />
        </RowActionButton>
      </div>
    </div>
  );
}

function WorkingAgentRow({
  avatarUrl,
  elapsed,
  name,
  onOpen,
  pubkey,
}: {
  avatarUrl: string | null;
  elapsed: string;
  name: string;
  onOpen: () => void;
  pubkey: string;
}) {
  return (
    <button
      className="flex w-full min-w-0 items-start gap-2.5 border-t border-border/50 px-3 py-3 text-left transition-colors first:border-t-0 hover:bg-muted/50 focus-visible:bg-muted/50 focus-visible:outline-hidden"
      data-testid={`channel-activity-agent-${pubkey}`}
      onClick={onOpen}
      type="button"
    >
      <UserAvatar
        avatarUrl={avatarUrl}
        className="h-9 w-9 shrink-0"
        displayName={name}
        shape="squircle"
        size="md"
      />
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-start gap-2">
          <span className="min-w-0 flex-1 truncate text-sm font-semibold leading-4 text-foreground">
            {name}
          </span>
          <span className="shrink-0 text-xs leading-4 text-muted-foreground/70">
            {elapsed}
          </span>
        </div>
        <span className="mt-0.5 flex items-center gap-1.5 text-xs leading-4 text-muted-foreground">
          <Loader2 className="h-3.5 w-3.5 animate-spin text-primary/70" />
          Working
        </span>
      </div>
    </button>
  );
}

function WorkingAgentRows({
  activeWorking,
  channelId,
  onOpen,
  profiles,
}: {
  activeWorking: ActiveChannelTurnSummary;
  channelId: string;
  onOpen: (pubkey: string, channelId: string) => void;
  profiles?: UserProfileLookup;
}) {
  const now = useNow(1000);
  const elapsed = formatElapsed(now - activeWorking.anchorAt);
  const alignedAgentNames =
    activeWorking.agentNames?.length === activeWorking.agentPubkeys.length
      ? activeWorking.agentNames
      : null;

  return activeWorking.agentPubkeys.map((pubkey, index) => {
    const profile = profiles?.[normalizePubkey(pubkey)];
    const name =
      profile?.displayName?.trim() ||
      alignedAgentNames?.[index] ||
      `Agent ${truncatePubkey(pubkey)}`;
    return (
      <WorkingAgentRow
        avatarUrl={profile?.avatarUrl ?? null}
        elapsed={elapsed}
        key={pubkey}
        name={name}
        onOpen={() => onOpen(pubkey, channelId)}
        pubkey={pubkey}
      />
    );
  });
}

export function ChannelActivityPopover({
  activeWorking,
  channel,
  children,
}: {
  activeWorking?: ActiveChannelTurnSummary;
  channel: Channel;
  children: React.ReactNode;
}) {
  const [open, setOpen] = React.useState(false);
  const hoverTimerRef = React.useRef<ReturnType<typeof setTimeout> | null>(
    null,
  );
  const {
    clearChannelUnreadSource,
    getChannelActivityItemReadAt,
    locallyUnreadFeedItems,
    markMessageRead,
    markThreadRead,
    unreadThreadFeedItems,
    feedItemState,
  } = useAppShell();
  const { undoUnread } = feedItemState;
  const identityQuery = useIdentityQuery();
  const { goChannel } = useAppNavigation();
  const { openAgentActivity } = useOpenAgentActivity();
  const { openReminder } = useRemindLater();

  const unreadChannelFeedItems = React.useMemo(() => {
    return unreadThreadFeedItems.filter(
      (item) => item.channelId === channel.id,
    );
  }, [channel.id, unreadThreadFeedItems]);
  const profilePubkeys = React.useMemo(
    () => [
      ...new Set([
        ...unreadChannelFeedItems.map((item) => item.pubkey),
        ...(activeWorking?.agentPubkeys ?? []),
      ]),
    ],
    [activeWorking?.agentPubkeys, unreadChannelFeedItems],
  );
  const profilesQuery = useUsersBatchQuery(open ? profilePubkeys : [], {
    enabled: open,
  });
  const profiles = profilesQuery.data?.profiles;
  const activityReadAtByMessageId = React.useMemo(
    () =>
      new Map(
        unreadChannelFeedItems.map((item) => [
          item.id,
          getChannelActivityItemReadAt(item),
        ]),
      ),
    [getChannelActivityItemReadAt, unreadChannelFeedItems],
  );
  const activityItems = React.useMemo(() => {
    if (!open) return [];
    return buildInboxItems({
      channels: [channel],
      currentPubkey: identityQuery.data?.pubkey,
      feed: buildChannelActivityFeed(unreadChannelFeedItems),
      getMessageReadAt: (messageId) =>
        activityReadAtByMessageId.get(messageId) ?? null,
      profiles,
    });
  }, [
    channel,
    activityReadAtByMessageId,
    identityQuery.data?.pubkey,
    open,
    profiles,
    unreadChannelFeedItems,
  ]);
  const hasContent =
    unreadChannelFeedItems.length > 0 ||
    (activeWorking?.agentPubkeys.length ?? 0) > 0;

  const clearHoverTimer = React.useCallback(() => {
    if (hoverTimerRef.current !== null) {
      clearTimeout(hoverTimerRef.current);
      hoverTimerRef.current = null;
    }
  }, []);
  const openWithDelay = React.useCallback(() => {
    if (!hasContent) return;
    clearHoverTimer();
    hoverTimerRef.current = setTimeout(() => {
      setOpen(true);
    }, DEFAULT_POPOVER_HOVER_OPEN_DELAY_MS);
  }, [clearHoverTimer, hasContent]);
  const openImmediately = React.useCallback(() => {
    if (!hasContent) return;
    clearHoverTimer();
    setOpen(true);
  }, [clearHoverTimer, hasContent]);
  const closeWithDelay = React.useCallback(() => {
    clearHoverTimer();
    hoverTimerRef.current = setTimeout(() => {
      setOpen(false);
    }, HOVER_CLOSE_DELAY_MS);
  }, [clearHoverTimer]);
  const keepOpen = React.useCallback(() => {
    clearHoverTimer();
  }, [clearHoverTimer]);

  React.useEffect(() => () => clearHoverTimer(), [clearHoverTimer]);
  React.useEffect(() => {
    if (!hasContent) {
      setOpen(false);
    }
  }, [hasContent]);

  const clearUnreadOverride = React.useCallback(
    (item: InboxItem) => {
      const clearedItemIds = new Set(getGroupedInboxItemIds(item));
      for (const itemId of clearedItemIds) {
        undoUnread(itemId);
      }
      const channelId = item.item.channelId ?? null;
      const hasAnotherChannelOverride = locallyUnreadFeedItems.some(
        (feedItem) =>
          feedItem.channelId === channelId && !clearedItemIds.has(feedItem.id),
      );
      if (channelId && !hasAnotherChannelOverride) {
        clearChannelUnreadSource(channelId, "inbox");
      }
    },
    [clearChannelUnreadSource, locallyUnreadFeedItems, undoUnread],
  );

  const handleMarkRead = React.useCallback(
    (item: InboxItem) => {
      clearUnreadOverride(item);
      for (const reply of item.groupItems) {
        markMessageRead(reply.id, reply.createdAt);
      }
      markThreadRead(item.conversationId, item.latestActivityAt);
    },
    [clearUnreadOverride, markMessageRead, markThreadRead],
  );

  if (!hasContent) {
    return children;
  }

  return (
    <Popover onOpenChange={setOpen} open={open}>
      {/* biome-ignore lint/a11y/noStaticElementInteractions: hover/focus events bubble from the nested channel button while the wrapper keeps the preview interactive. */}
      <div
        className="w-full min-w-0"
        onBlur={closeWithDelay}
        onContextMenu={() => setOpen(false)}
        onFocus={openImmediately}
        onMouseEnter={openWithDelay}
        onMouseLeave={closeWithDelay}
      >
        <PopoverAnchor asChild>{children}</PopoverAnchor>
      </div>
      <PopoverContent
        align="start"
        className="w-96 overflow-hidden p-0"
        data-testid={`channel-activity-popover-${channel.name}`}
        onFocusCapture={keepOpen}
        onMouseEnter={keepOpen}
        onMouseLeave={closeWithDelay}
        onOpenAutoFocus={(event) => event.preventDefault()}
        side="right"
        sideOffset={0}
        style={ACTIVITY_POPOVER_MOTION_STYLE}
      >
        <section
          aria-label="Channel activity"
          className="flex max-h-96 min-h-0 flex-col overflow-hidden"
        >
          <h3
            className="relative z-20 shrink-0 border-b border-border/70 bg-background/95 px-3 py-2 text-sm font-semibold text-foreground backdrop-blur-md supports-[backdrop-filter]:bg-background/90"
            data-testid="channel-activity-header"
          >
            Channel activity
          </h3>
          <div
            className="buzz-channel-activity-scrollbar min-h-0 overflow-y-auto overscroll-contain"
            data-testid="channel-activity-scroll"
          >
            {activeWorking?.agentPubkeys.length ? (
              <WorkingAgentRows
                activeWorking={activeWorking}
                channelId={channel.id}
                onOpen={(pubkey, channelId) => {
                  setOpen(false);
                  openAgentActivity(pubkey, { channelId });
                }}
                profiles={profiles}
              />
            ) : null}
            {activityItems.length > 0
              ? activityItems.map((item) => (
                  <ThreadPreviewRow
                    isAgent={
                      profiles?.[normalizePubkey(item.item.pubkey)]?.isAgent ===
                      true
                    }
                    item={item}
                    key={item.conversationId}
                    onMarkRead={() => handleMarkRead(item)}
                    onOpen={() => {
                      clearUnreadOverride(item);
                      setOpen(false);
                      void goChannel(channel.id, {
                        messageId: item.id,
                        threadRootId: item.conversationId,
                      });
                    }}
                    onRemindLater={() => {
                      setOpen(false);
                      openReminder({
                        authorPubkey: item.item.pubkey,
                        channelId: channel.id,
                        eventId: item.id,
                        preview: item.preview.slice(0, 100),
                      });
                    }}
                  />
                ))
              : null}
          </div>
        </section>
      </PopoverContent>
    </Popover>
  );
}
