import { ArrowLeft, Hash, Mail, MoreHorizontal, Trash2 } from "lucide-react";
import * as React from "react";

import type {
  InboxContextMessage,
  InboxItem,
  InboxReply,
} from "@/features/home/lib/inbox";
import { ChannelMembersBar } from "@/features/channels/ui/ChannelMembersBar";
import { formatInboxTypeLabel } from "@/features/home/lib/inbox";
import {
  type InboxDisplayMessage,
  InboxMessageRow,
} from "@/features/home/ui/InboxMessageRow";
import type { TimelineMessage } from "@/features/messages/types";
import { formatTime } from "@/features/messages/lib/dateFormatters";
import {
  hasSameMessageAuthor,
  isWithinGroupingWindow,
} from "@/features/messages/lib/messageGrouping";
import { orderMentionPubkeysByText } from "@/features/messages/lib/orderMentionPubkeys";
import { getThreadReference } from "@/features/messages/lib/threading";
import { normalizePubkey } from "@/shared/lib/pubkey";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { useAnchoredScroll } from "@/features/messages/ui/useAnchoredScroll";
import { UpdateIndicator } from "@/features/settings/UpdateIndicator";
import type { Channel, UserProfileSummary } from "@/shared/api/types";
import { resolveMentionProps } from "@/shared/lib/resolveMentionNames";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/shared/ui/tooltip";

const MembersSidebar = React.lazy(async () => {
  const module = await import("@/features/channels/ui/MembersSidebar");
  return { default: module.MembersSidebar };
});

type InboxDetailPaneProps = {
  agentPubkeys?: ReadonlySet<string>;
  canDelete: boolean;
  canOpenChannel: boolean;
  canReply: boolean;
  disabledReplyReason?: string | null;
  isDeletingMessage?: boolean;
  isSendingReply?: boolean;
  isSinglePanelView?: boolean;
  isThreadContextLoading?: boolean;
  item: InboxItem | null;
  messages?: InboxContextMessage[];
  profiles?: Record<string, UserProfileSummary>;
  replies?: InboxReply[];
  channel: Channel | null;
  contextChannelName?: string | null;
  currentPubkey?: string;
  /**
   * The event anchor: the specific event ID the user selected or navigated to
   * via `?item=`. Used for message highlighting and as the stable identity for
   * scroll/focus effects. Does NOT change when a live reply advances the
   * representative `item.id`.
   */
  selectedEventId: string | null;
  /**
   * The default reply-parent event ID derived from the latched anchor's tags
   * in HomeView (`parentId ?? anchor.id`). Populated once the anchor is found
   * in feedItems and held until a new anchor is selected. Used as fallback
   * when the anchor event has been displaced from the current `groupItems`
   * (e.g. a very old anchor evicted by a newer representative).
   */
  latchedDefaultParentId?: string | null;
  onBack?: () => void;
  onDelete: () => void;
  onManageChannel: (channelId: string) => void;
  onOpenContext: (
    channelId: string,
    messageId: string,
    threadRootId?: string | null,
  ) => void;
  onSendReply: (input: {
    content: string;
    mediaTags?: string[][];
    mentionPubkeys: string[];
    parentEventId: string;
  }) => Promise<void>;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
};

export function InboxDetailPane({
  agentPubkeys,
  canDelete,
  canOpenChannel,
  canReply,
  disabledReplyReason,
  isDeletingMessage = false,
  isSendingReply = false,
  isSinglePanelView = false,
  isThreadContextLoading = false,
  item,
  messages = [],
  profiles,
  replies = [],
  channel,
  contextChannelName = null,
  currentPubkey,
  selectedEventId,
  latchedDefaultParentId = null,
  onBack,
  onDelete,
  onManageChannel,
  onOpenContext,
  onSendReply,
  onToggleReaction,
}: InboxDetailPaneProps) {
  const detailPaneRef = React.useRef<HTMLElement | null>(null);
  // Refs for the shared anchored-scroll hook's container and content roots.
  const scrollContainerRef = React.useRef<HTMLDivElement | null>(null);
  const contentRef = React.useRef<HTMLDivElement | null>(null);
  const [replyTargetId, setReplyTargetId] = React.useState<string | null>(null);
  const [isFocusHighlightVisible, setIsFocusHighlightVisible] =
    React.useState(true);
  const [isMembersSidebarOpen, setIsMembersSidebarOpen] = React.useState(false);
  // The stable conversation ID: does not change when the representative latest
  // event advances. All lifecycle effects (reply target reset, focus highlight,
  // scroll centering) key on this.
  const conversationId = item?.conversationId ?? null;
  const selectedChannelId = item?.item.channelId ?? null;
  // Build the plain, non-virtualized timeline the shared hook anchors against.
  // Live arrivals rerun its layout compensation without changing the target.

  const selectedMessage = messages.find((message) => message.isSelected);
  // A latest reply can represent an Inbox conversation. Resolve the actual
  // root from loaded context or the complete feed group; never treat an
  // unresolved root/profile lookup as an authoritative empty audience.
  const contextRoot = messages.find((message) => message.id === conversationId);
  const feedRoot = item
    ? [item.item, ...item.groupItems].find(
        (groupItem) => groupItem.id === conversationId,
      )
    : undefined;
  const rootMessage = contextRoot
    ? {
        authorPubkey: contextRoot.authorPubkey,
        content: contextRoot.content,
        mentionPubkeysByName: contextRoot.mentionPubkeysByName,
      }
    : feedRoot && profiles
      ? {
          authorPubkey: feedRoot.pubkey,
          content: feedRoot.content,
          mentionPubkeysByName: resolveMentionProps(feedRoot.tags, profiles)
            .mentionPubkeysByName,
        }
      : null;
  const initialAgentPubkeys = rootMessage
    ? currentPubkey &&
      normalizePubkey(rootMessage.authorPubkey) ===
        normalizePubkey(currentPubkey)
      ? orderMentionPubkeysByText(
          rootMessage.content,
          rootMessage.mentionPubkeysByName,
          (pubkey) => agentPubkeys?.has(pubkey) === true,
        )
      : []
    : undefined;
  const pendingReplyMessages: InboxDisplayMessage[] = replies.map((reply) => ({
    ...reply,
    depth: reply.depth ?? (selectedMessage?.depth ?? 0) + 1,
    isSelected: false,
    mentionNames: [],
  }));
  const displayMessages: InboxDisplayMessage[] =
    messages.length > 0
      ? [...messages, ...pendingReplyMessages]
      : item
        ? [
            {
              authorLabel: item.senderLabel,
              authorPubkey: item.item.pubkey,
              avatarUrl: item.avatarUrl,
              content: item.preview,
              createdAt: item.item.createdAt,
              depth: 0,
              fullTimestampLabel: item.fullTimestampLabel,
              id: item.id,
              isSelected: true,
              mentionNames: item.mentionNames,
              mentionPubkeysByName: item.mentionPubkeysByName,
              timeLabel: formatTime(item.item.createdAt),
            },
            ...pendingReplyMessages,
          ]
        : pendingReplyMessages;
  const { onScroll } = useAnchoredScroll({
    channelId: conversationId,
    contentRef,
    isLoading: isThreadContextLoading,
    messages: displayMessages,
    pinTargetCentered: true,
    scrollContainerRef,
    targetMessageId: selectedEventId,
  });

  const focusComposer = React.useCallback(() => {
    window.requestAnimationFrame(() => {
      const textarea =
        detailPaneRef.current?.querySelector<HTMLTextAreaElement>(
          '[data-testid="message-input"]',
        );
      textarea?.focus();
    });
  }, []);

  React.useEffect(() => {
    void conversationId;
    setReplyTargetId(null);
  }, [conversationId]);

  React.useEffect(() => {
    void selectedChannelId;
    setIsMembersSidebarOpen(false);
  }, [selectedChannelId]);

  React.useEffect(() => {
    void conversationId;
    setIsFocusHighlightVisible(true);
    const timeoutId = window.setTimeout(() => {
      setIsFocusHighlightVisible(false);
    }, 1_200);

    return () => {
      window.clearTimeout(timeoutId);
    };
  }, [conversationId]);

  // Capture the default composer reply parent from the selected-event anchor
  // when the conversation first opens (or when the user explicitly navigates
  // to a different event anchor). Reset only when conversationId/selectedEventId
  // changes so that a live incoming message does not silently retarget the
  // in-progress reply, preserving PR #1714 same-depth semantics.
  //
  // Design: no render-phase ref mutations. `item` and `latchedDefaultParentId`
  // are explicit deps. A committed ref (`parentCapturedRef`) written only inside
  // effects prevents live-update re-runs from overwriting a value that was
  // already captured for the current (conversationId, selectedEventId) pair.
  const [capturedDefaultParentId, setCapturedDefaultParentId] = React.useState<
    string | null
  >(null);
  // Written only inside committed effects — never during render.
  const parentCapturedRef = React.useRef(false);

  // Reset when the user navigates to a different conversation or event anchor.
  // biome-ignore lint/correctness/useExhaustiveDependencies: parentCapturedRef is a ref (not a reactive value); conversationId and selectedEventId are the intentional reset triggers
  React.useEffect(() => {
    parentCapturedRef.current = false;
    setCapturedDefaultParentId(null);
  }, [conversationId, selectedEventId]);

  // Capture the default parent once per (conversation, anchor) pair. The effect
  // also fires when `item` or `latchedDefaultParentId` changes, but the
  // `parentCapturedRef` guard prevents overwriting a value that was already
  // resolved for the current anchor. The one exception: when the anchor is not
  // in groupItems and `latchedDefaultParentId` was null on the first run, we
  // defer capture until the latch arrives (parentCapturedRef stays false so
  // the null→resolved transition of latchedDefaultParentId triggers re-capture).
  // biome-ignore lint/correctness/useExhaustiveDependencies: conversationId is derived from item but listed explicitly as a self-documenting reset signal; parentCapturedRef is a ref
  React.useEffect(() => {
    if (parentCapturedRef.current) {
      return;
    }
    if (!item) {
      setCapturedDefaultParentId(null);
      return;
    }
    // Look for the anchored event inside groupItems first (it may be an older
    // non-representative event), then fall back to the representative item.
    const anchoredEvent =
      selectedEventId != null
        ? item.groupItems.find((gi) => gi.id === selectedEventId)
        : null;
    if (anchoredEvent) {
      // Anchor found in groupItems — derive parent from its tags. Mark as
      // captured so live feed advances don't retarget the reply.
      const defaultParent =
        getThreadReference(anchoredEvent.tags).parentId ?? anchoredEvent.id;
      setCapturedDefaultParentId(defaultParent);
      parentCapturedRef.current = true;
      return;
    }
    // Anchor is not in groupItems (evicted from feed window). Use the latched
    // default parent from HomeView, which was captured when the event was still
    // present in feedItems. If the latch is not yet available (null), use the
    // representative fallback but do NOT mark as captured — the null→resolved
    // transition of latchedDefaultParentId will fire this effect again with the
    // correct value.
    if (latchedDefaultParentId != null) {
      setCapturedDefaultParentId(latchedDefaultParentId);
      parentCapturedRef.current = true;
      return;
    }
    // Latch not yet available; install the representative fallback without
    // marking as captured so the true latch value replaces it when it arrives.
    const fallback =
      getThreadReference(item.item.tags ?? []).parentId ?? item.id;
    setCapturedDefaultParentId(fallback);
  }, [conversationId, selectedEventId, item, latchedDefaultParentId]);

  if (!item) {
    return (
      <section
        className="flex min-h-0 min-w-0 items-center justify-center bg-background/60 px-6 py-10 pt-20 text-center"
        data-testid="home-inbox-detail-empty"
      >
        <div className="max-w-sm">
          <div className="mx-auto flex h-14 w-14 items-center justify-center rounded-full bg-muted text-muted-foreground">
            <Mail className="h-6 w-6" />
          </div>
          <p className="mt-4 text-base font-semibold">Select a message</p>
          <p className="mt-1 text-sm text-muted-foreground">
            Pick an inbox item to see the full message and react to it.
          </p>
        </div>
      </section>
    );
  }

  const replyTarget =
    displayMessages.find((message) => message.id === replyTargetId) ?? null;
  // Explicit sub-message reply wins. Otherwise use the captured default parent
  // (derived from the selected-event anchor at conversation entry), which does
  // not change when a live incoming message advances the representative item.
  const composerParentEventId =
    replyTarget?.id ?? capturedDefaultParentId ?? item.id;
  const composerReplyTarget =
    replyTarget && replyTarget.id !== item.id
      ? {
          author: replyTarget.authorLabel,
          body: replyTarget.content,
          id: replyTarget.id,
        }
      : null;
  const channelContextName = contextChannelName ?? item.channelLabel;
  const composerChannelType =
    item.item.channelType === "dm" ||
    item.item.channelType === "stream" ||
    item.item.channelType === "forum"
      ? item.item.channelType
      : null;
  const contextLabel = channelContextName ?? formatInboxTypeLabel(item);
  const hasChannelContext = Boolean(channelContextName);
  const contextChannelId = item.item.channelId;
  const contextThreadRootId = getThreadReference(item.item.tags).rootId;

  const handleSelectReplyTarget = (message: InboxDisplayMessage) => {
    setReplyTargetId((currentReplyTargetId) =>
      currentReplyTargetId === message.id ? null : message.id,
    );
    focusComposer();
  };

  return (
    <section
      className="flex min-h-0 min-w-0 flex-col overflow-hidden bg-background/60"
      data-testid="home-inbox-detail"
      ref={detailPaneRef}
    >
      <div className="relative flex min-h-0 flex-1 flex-col overflow-hidden">
        <TopChromeInsetHeader flush>
          <div className="px-5 py-2">
            <div className="flex min-h-9 min-w-0 items-center justify-between gap-3">
              <div
                className={cn(
                  "flex min-w-0 items-center",
                  isSinglePanelView ? "gap-[4px]" : "gap-1",
                )}
              >
                {onBack ? (
                  <Button
                    aria-label="Back to inbox list"
                    className="rounded-full text-muted-foreground hover:bg-muted/60 hover:text-foreground"
                    onClick={onBack}
                    size="icon"
                    type="button"
                    variant="ghost"
                  >
                    <ArrowLeft />
                  </Button>
                ) : null}
                <div className="min-w-0">
                  {canOpenChannel && contextChannelId ? (
                    <button
                      className="flex min-w-0 items-center gap-[4px] text-left text-sm font-semibold leading-5 tracking-tight text-foreground hover:underline focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
                      onClick={() =>
                        onOpenContext(
                          contextChannelId,
                          item.id,
                          contextThreadRootId,
                        )
                      }
                      title={item.fullTimestampLabel}
                      type="button"
                    >
                      {hasChannelContext ? (
                        <Hash className="h-4 w-4 shrink-0" color="gray" />
                      ) : null}
                      <span className="min-w-0 translate-y-px truncate">
                        {contextLabel}
                      </span>
                    </button>
                  ) : (
                    <h2
                      className="flex min-w-0 items-center gap-[4px] text-sm font-semibold leading-5 tracking-tight text-foreground"
                      title={item.fullTimestampLabel}
                    >
                      {hasChannelContext ? (
                        <Hash className="h-4 w-4 shrink-0" color="gray" />
                      ) : null}
                      <span className="min-w-0 translate-y-px truncate">
                        {contextLabel}
                      </span>
                    </h2>
                  )}
                </div>
              </div>

              <TooltipProvider delayDuration={200}>
                <div className="flex shrink-0 items-center gap-1">
                  <UpdateIndicator />
                  {channel ? (
                    <ChannelMembersBar
                      channel={channel}
                      currentPubkey={currentPubkey}
                      onManageChannel={() => {
                        if (contextChannelId) {
                          onManageChannel(contextChannelId);
                        }
                      }}
                      onToggleMembers={() =>
                        setIsMembersSidebarOpen((open) => !open)
                      }
                    />
                  ) : null}
                  {canDelete ? (
                    <HeaderMoreMenu
                      isDeletingMessage={isDeletingMessage}
                      onDelete={onDelete}
                    />
                  ) : null}
                </div>
              </TooltipProvider>
            </div>
          </div>
        </TopChromeInsetHeader>

        <div
          aria-busy={isThreadContextLoading}
          className="min-h-0 flex-1 overflow-y-auto overscroll-contain pb-32 [overflow-anchor:none]"
          onScroll={onScroll}
          ref={scrollContainerRef}
        >
          <div ref={contentRef}>
            {displayMessages.map((message, index) => {
              const isAfterSeparator = index === 1;
              const previousMessage = displayMessages[index - 1];
              const isContinuation =
                !isAfterSeparator &&
                hasSameMessageAuthor(
                  { pubkey: previousMessage?.authorPubkey },
                  { pubkey: message.authorPubkey },
                ) &&
                isWithinGroupingWindow(
                  previousMessage?.createdAt,
                  message.createdAt,
                );

              return (
                <React.Fragment key={message.id}>
                  {isAfterSeparator ? (
                    <div className="mx-6 my-3 border-t border-border/60" />
                  ) : null}
                  <InboxMessageRow
                    agentPubkeys={agentPubkeys}
                    canReply={canReply}
                    channelId={item.item.channelId}
                    isContinuation={isContinuation}
                    isFocusHighlightVisible={isFocusHighlightVisible}
                    message={message}
                    onSelectReplyTarget={handleSelectReplyTarget}
                    onToggleReaction={onToggleReaction}
                  />
                </React.Fragment>
              );
            })}
          </div>
        </div>

        <div className="pointer-events-none absolute inset-x-0 bottom-0 z-10">
          <div className="pointer-events-auto">
            <MessageComposer
              audienceContext={{
                type: "thread",
                threadRootId: item.conversationId,
                initialAgentPubkeys,
              }}
              channelId={item.item.channelId}
              channelName={item.channelLabel ?? "channel"}
              channelType={composerChannelType}
              containerClassName="px-4 pb-4 sm:px-4"
              disabled={!canReply}
              draftKey={`thread:${item.conversationId}`}
              isSending={isSendingReply}
              onCancelReply={
                composerReplyTarget ? () => setReplyTargetId(null) : undefined
              }
              onSend={(content, mentionPubkeys, mediaTags) =>
                onSendReply({
                  content,
                  mediaTags,
                  mentionPubkeys,
                  parentEventId: composerParentEventId,
                })
              }
              placeholder={
                canReply
                  ? `Send reply to ${item.channelLabel ? `#${item.channelLabel} thread` : "channel thread"}`
                  : (disabledReplyReason ??
                    "Replies are not available for this item.")
              }
              replyTarget={composerReplyTarget}
            />
          </div>
        </div>
      </div>

      {channel ? (
        <React.Suspense fallback={null}>
          <MembersSidebar
            channel={channel}
            currentPubkey={currentPubkey}
            onOpenChange={setIsMembersSidebarOpen}
            open={isMembersSidebarOpen}
          />
        </React.Suspense>
      ) : null}
    </section>
  );
}

function HeaderMoreMenu({
  isDeletingMessage,
  onDelete,
}: {
  isDeletingMessage: boolean;
  onDelete: () => void;
}) {
  const trigger = (
    <Button
      aria-label="More actions"
      className="rounded-full text-muted-foreground"
      size="icon"
      type="button"
      variant="ghost"
    >
      <MoreHorizontal />
    </Button>
  );

  return (
    <DropdownMenu modal={false}>
      <Tooltip>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>{trigger}</DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent>More actions</TooltipContent>
      </Tooltip>
      <DropdownMenuContent align="end">
        <DropdownMenuItem
          className="text-destructive focus:text-destructive"
          disabled={isDeletingMessage}
          onClick={onDelete}
        >
          <Trash2 className="h-4 w-4" />
          Delete message
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
