import { MessageCircle } from "lucide-react";
import * as React from "react";
import { useSyncExternalStore } from "react";

import { useChannelsQuery } from "@/features/channels/hooks";
import {
  useChannelMessagesQuery,
  useChannelSubscription,
  useSendMessageMutation,
} from "@/features/messages/hooks";
import { isChannelCreatedSystemMessage } from "@/features/channels/ui/ChannelPane.helpers";
import { formatTimelineMessages } from "@/features/messages/lib/formatTimelineMessages";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { MessageThreadTranscript } from "@/features/messages/ui/MessageThreadTranscript";
import { useProfileQuery, useUsersBatchQuery } from "@/features/profile/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import {
  CHANNEL_TIMELINE_CONTENT_KINDS,
  KIND_SYSTEM_MESSAGE,
} from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { Button } from "@/shared/ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "@/shared/ui/popover";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import {
  setHuddleDockChatOpen,
  subscribeHuddleShareUiState,
  getHuddleDockChatOpen,
} from "../lib/huddleShareExpandStore";

const CONTENT_KIND_SET = new Set<number>(CHANNEL_TIMELINE_CONTENT_KINDS);

function formatUnreadBadge(count: number): string {
  return count > 99 ? "99+" : String(count);
}

type HuddleDockChatControlProps = {
  channelId: string | null;
  /** Only render while screen share is expanded over the transcript. */
  visible: boolean;
};

/**
 * Dock Chat control for expand-share mode: badge = unread while the stage
 * covers the normal transcript; popover hosts messages + composer.
 */
export function HuddleDockChatControl({
  channelId,
  visible,
}: HuddleDockChatControlProps) {
  const [open, setOpen] = React.useState(false);
  const identityQuery = useIdentityQuery();
  const profileQuery = useProfileQuery();
  const channelsQuery = useChannelsQuery({ enabled: visible && !!channelId });
  const channel = React.useMemo(
    () =>
      channelId
        ? ((channelsQuery.data ?? []).find((entry) => entry.id === channelId) ??
          null)
        : null,
    [channelId, channelsQuery.data],
  );

  useChannelSubscription(visible ? channel : null);
  const messagesQuery = useChannelMessagesQuery(visible ? channel : null);
  const sendMessageMutation = useSendMessageMutation(
    channel,
    identityQuery.data,
  );

  const contentEvents = React.useMemo(
    () =>
      (messagesQuery.data ?? []).filter((event) => {
        if (!CONTENT_KIND_SET.has(event.kind)) return false;
        // Don't badge or attribute the relay-owner channel_created row.
        if (event.kind !== KIND_SYSTEM_MESSAGE) return true;
        try {
          return (
            (JSON.parse(event.content) as { type?: string }).type !==
            "channel_created"
          );
        } catch {
          return true;
        }
      }),
    [messagesQuery.data],
  );

  const authorPubkeys = React.useMemo(
    () => [
      ...new Set(
        contentEvents
          .map((event) => normalizePubkey(event.pubkey))
          .filter(Boolean),
      ),
    ],
    [contentEvents],
  );
  const profilesQuery = useUsersBatchQuery(authorPubkeys, {
    enabled: visible && authorPubkeys.length > 0,
  });

  const timelineMessages = React.useMemo(() => {
    const formatted = formatTimelineMessages(
      messagesQuery.data ?? [],
      channel,
      identityQuery.data?.pubkey,
      profileQuery.data?.avatarUrl ?? null,
      profilesQuery.data?.profiles,
    );
    // Huddle create always posts kind:40099 channel_created from the relay
    // owner. Dock chat uses MessageThreadRow (not SystemMessageRow), so without
    // this filter the raw JSON shows as a chat bubble every huddle.
    return formatted.filter(
      (message) => !isChannelCreatedSystemMessage(message),
    );
  }, [
    channel,
    identityQuery.data?.pubkey,
    messagesQuery.data,
    profileQuery.data?.avatarUrl,
    profilesQuery.data?.profiles,
  ]);

  // Watermark unread while the stage covers chat and the popover is closed.
  const seenThroughRef = React.useRef<number>(0);
  const [unreadCount, setUnreadCount] = React.useState(0);
  const latestCreatedAt = contentEvents.reduce(
    (max, event) => Math.max(max, event.created_at),
    0,
  );

  React.useEffect(() => {
    if (!visible) {
      seenThroughRef.current = 0;
      setUnreadCount(0);
      setOpen(false);
      return;
    }
    // First paint after expand: seed watermark so existing history is not
    // counted as unread; only messages that arrive while covered badge up.
    if (seenThroughRef.current === 0 && latestCreatedAt > 0) {
      seenThroughRef.current = latestCreatedAt;
    }
  }, [latestCreatedAt, visible]);

  React.useEffect(() => {
    if (!visible || open) return;
    if (seenThroughRef.current === 0) return;
    const unread = contentEvents.filter(
      (event) => event.created_at > seenThroughRef.current,
    ).length;
    setUnreadCount(unread);
  }, [contentEvents, open, visible]);

  React.useEffect(() => {
    if (!visible) {
      setHuddleDockChatOpen(false);
      return;
    }
    setHuddleDockChatOpen(open);
    return () => setHuddleDockChatOpen(false);
  }, [open, visible]);

  React.useEffect(() => {
    if (!open) return;
    if (latestCreatedAt > 0) seenThroughRef.current = latestCreatedAt;
    setUnreadCount(0);
  }, [latestCreatedAt, open]);

  const scrollRef = React.useRef<HTMLDivElement | null>(null);
  React.useEffect(() => {
    if (!open) return;
    const node = scrollRef.current;
    if (!node) return;
    // Depend on timelineMessages so newly arrived rows stay in view.
    if (timelineMessages.length >= 0) {
      node.scrollTop = node.scrollHeight;
    }
  }, [open, timelineMessages]);

  const handleOpenChange = React.useCallback((next: boolean) => {
    setOpen(next);
  }, []);

  const handleSend = React.useCallback(
    async (
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
    ) => {
      if (!channelId) return;
      await sendMessageMutation.mutateAsync({
        channelId,
        content,
        mentionPubkeys,
        mediaTags,
      });
    },
    [channelId, sendMessageMutation],
  );

  // Keep store subscription warm so tests / read-marker can observe dock chat.
  useSyncExternalStore(
    subscribeHuddleShareUiState,
    getHuddleDockChatOpen,
    () => false,
  );

  if (!visible) return null;

  return (
    <Popover onOpenChange={handleOpenChange} open={open}>
      <Tooltip>
        <TooltipTrigger asChild>
          <PopoverTrigger asChild>
            <Button
              aria-label={
                unreadCount > 0
                  ? `Huddle chat, ${unreadCount} unread`
                  : "Huddle chat"
              }
              aria-pressed={open}
              className={cn(
                "buzz-huddle-control-button relative h-12 w-12 shrink-0 rounded-md",
                open && "text-foreground",
              )}
              data-testid="huddle-dock-chat-button"
              size="icon"
              type="button"
              variant={open ? "secondary" : "ghost"}
            >
              <MessageCircle className="h-4 w-4" />
              {unreadCount > 0 ? (
                <span
                  className="absolute -right-1 -top-1 flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-[10px] font-semibold leading-none text-primary-foreground tabular-nums"
                  data-testid="huddle-dock-chat-unread"
                >
                  {formatUnreadBadge(unreadCount)}
                  <span className="sr-only"> unread</span>
                </span>
              ) : null}
            </Button>
          </PopoverTrigger>
        </TooltipTrigger>
        <TooltipContent className="buzz-huddle-tooltip" side="top">
          Chat
        </TooltipContent>
      </Tooltip>
      <PopoverContent
        align="start"
        className="buzz-huddle-popover flex h-[min(28rem,70vh)] w-[min(24rem,92vw)] flex-col gap-0 overflow-hidden p-0"
        data-testid="huddle-dock-chat-popover"
        side="top"
        sideOffset={10}
      >
        <div className="shrink-0 border-b border-border/60 px-3 py-2">
          <p className="text-sm font-medium text-foreground">Huddle chat</p>
          <p className="text-xs text-muted-foreground">
            Messages while screen share is expanded
          </p>
        </div>
        <div
          className="min-h-0 flex-1 overflow-y-auto px-1 py-2"
          data-testid="huddle-dock-chat-messages"
          ref={scrollRef}
        >
          {channel && timelineMessages.length > 0 ? (
            <MessageThreadTranscript
              channelId={channel.id}
              currentPubkey={identityQuery.data?.pubkey}
              messages={timelineMessages}
              profiles={profilesQuery.data?.profiles}
              testId="huddle-dock-chat-transcript"
            />
          ) : (
            <div className="flex h-full min-h-32 flex-col items-center justify-center gap-1 px-4 text-center">
              <p className="text-sm font-medium text-foreground">
                No messages yet
              </p>
              <p className="text-xs text-muted-foreground">
                Say something to the huddle from here.
              </p>
            </div>
          )}
        </div>
        <div className="shrink-0 border-t border-border/60">
          <MessageComposer
            channelId={channelId}
            channelName="huddle"
            channelType={channel?.channelType ?? null}
            containerClassName="px-2 pb-2 pt-1"
            disabled={!channelId || sendMessageMutation.isPending}
            draftKey={channelId ? `huddle-dock-chat:${channelId}` : undefined}
            isSending={sendMessageMutation.isPending}
            layoutMode="standalone"
            onSend={handleSend}
            placeholder="Message the huddle"
            profiles={profilesQuery.data?.profiles}
            showBackgroundUploadProgress={false}
            showTopBorder={false}
          />
        </div>
      </PopoverContent>
    </Popover>
  );
}
