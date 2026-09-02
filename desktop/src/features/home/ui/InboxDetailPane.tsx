import {
  AlertCircle,
  ArrowLeft,
  ExternalLink,
  LoaderCircle,
  Mail,
  MoreHorizontal,
  Trash2,
} from "lucide-react";
import * as React from "react";

import type {
  InboxContextMessage,
  InboxItem,
  InboxReply,
} from "@/features/home/lib/inbox";
import { getProjectInboxReference } from "@/features/home/lib/projectInbox";
import { ProjectInboxDetail } from "@/features/home/ui/ProjectInboxDetail";
import { ChannelMembersBar } from "@/features/channels/ui/ChannelMembersBar";
import { useCommunities } from "@/features/communities/useCommunities";
import { formatInboxTypeLabel } from "@/features/home/lib/inbox";
import {
  hasInboxThreadContext,
  toTimelineMessage,
} from "@/features/home/lib/inboxViewHelpers";
import {
  type InboxDisplayMessage,
  InboxMessageRow,
} from "@/features/home/ui/InboxMessageRow";
import type { TimelineMessage } from "@/features/messages/types";
import { formatTime } from "@/features/messages/lib/dateFormatters";
import {
  hasSameMessageAuthor,
  isWithinGroupingWindow,
  startsNewMessageGroup,
} from "@/features/messages/lib/messageGrouping";
import { canManageMessageForCurrentUser } from "@/features/messages/lib/canManageMessage";
import { buildEditMentionState } from "@/features/messages/lib/draftMentionRefs";
import { imetaMediaFromTags } from "@/features/messages/lib/imetaMediaMarkdown";
import {
  buildVideoReviewPresentationByMessageId,
  hasRenderedVideoAttachment,
} from "@/features/messages/lib/videoReviewContext";
import { getThreadReference } from "@/features/messages/lib/threading";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { useAnchoredScroll } from "@/features/messages/ui/useAnchoredScroll";
import { useComposerHeightPadding } from "@/features/messages/ui/useComposerHeightPadding";
import { UpdateIndicator } from "@/features/settings/UpdateIndicator";
import type { Channel, UserProfileSummary } from "@/shared/api/types";
import { TopChromeInsetHeader } from "@/shared/layout/TopChromeInsetHeader";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { VideoReviewNavigationProvider } from "@/shared/ui/VideoReviewNavigation";
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

const EMPTY_CONTEXT_MESSAGES: InboxContextMessage[] = [];
const EMPTY_REPLIES: InboxReply[] = [];

type InboxDetailPaneProps = {
  agentPubkeys?: ReadonlySet<string>;
  canDelete: boolean;
  canOpenChannel: boolean;
  canReply: boolean;
  disabledReplyReason?: string | null;
  isDeletingMessage?: boolean;
  isEditingMessage?: boolean;
  isSendingReply?: boolean;
  editTargetId: string | null;
  isSinglePanelView?: boolean;
  hasThreadContextLoadError?: boolean;
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
  unreadBoundaryEventId?: string | null;
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
  onDeleteMessage: (eventId: string) => void;
  onEditTargetChange: React.Dispatch<React.SetStateAction<string | null>>;
  onEditSave: (input: {
    content: string;
    eventId: string;
    mediaTags?: string[][];
    mentionPubkeys?: string[];
  }) => Promise<void>;
  onRequestEmptyEditDelete: (eventId: string) => void;
  onManageChannel: (channelId: string) => void;
  onOpenContext: (
    channelId: string,
    messageId: string,
    threadRootId?: string | null,
  ) => void;
  /** True while the selected hidden DM is being reopened on the relay. */
  reopenPending?: boolean;
  /** True when the last reopen of the selected hidden DM failed. */
  reopenErrored?: boolean;
  onSendReply: (input: {
    content: string;
    mediaTags?: string[][];
    mentionPubkeys: string[];
    parentEventId: string | null;
  }) => Promise<void>;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
};

/** Routes Inbox selections to their canonical message or Buzz Git detail. */
export function InboxDetailPane(props: InboxDetailPaneProps) {
  if (props.item && getProjectInboxReference(props.item.item)) {
    return (
      <ProjectInboxDetail
        isSinglePanelView={props.isSinglePanelView}
        item={props.item}
        onBack={props.onBack}
        profiles={props.profiles}
      />
    );
  }

  return (
    <VideoReviewNavigationProvider>
      <InboxMessageDetailPane {...props} />
    </VideoReviewNavigationProvider>
  );
}

function InboxMessageDetailPane({
  agentPubkeys,
  canDelete,
  canOpenChannel,
  canReply,
  disabledReplyReason,
  editTargetId,
  isDeletingMessage = false,
  isEditingMessage = false,
  isSendingReply = false,
  isSinglePanelView = false,
  hasThreadContextLoadError = false,
  isThreadContextLoading = false,
  item,
  messages = EMPTY_CONTEXT_MESSAGES,
  profiles,
  replies = EMPTY_REPLIES,
  channel,
  contextChannelName = null,
  currentPubkey,
  selectedEventId,
  unreadBoundaryEventId = null,
  latchedDefaultParentId = null,
  onBack,
  onDelete,
  onDeleteMessage,
  onEditTargetChange,
  onEditSave,
  onRequestEmptyEditDelete,
  onManageChannel,
  onOpenContext,
  reopenPending = false,
  reopenErrored = false,
  onSendReply,
  onToggleReaction,
}: InboxDetailPaneProps) {
  const detailPaneRef = React.useRef<HTMLElement | null>(null);
  const { activeCommunity } = useCommunities();
  // Refs for the shared anchored-scroll hook's container and content roots.
  const scrollContainerRef = React.useRef<HTMLDivElement | null>(null);
  const contentRef = React.useRef<HTMLDivElement | null>(null);
  const composerWrapperRef = React.useRef<HTMLDivElement | null>(null);
  const [replyTargetId, setReplyTargetId] = React.useState<string | null>(null);
  const [isFocusHighlightVisible, setIsFocusHighlightVisible] =
    React.useState(true);
  const [isMembersSidebarOpen, setIsMembersSidebarOpen] = React.useState(false);
  // The stable conversation ID: does not change when the representative latest
  // event advances. All lifecycle effects (reply target reset, focus highlight,
  // scroll centering) key on this.
  const conversationId = item?.conversationId ?? null;
  const selectedChannelId = item?.item.channelId ?? null;
  const isDirectMessage = item?.item.channelType === "dm";
  // Build the plain, non-virtualized timeline the shared hook anchors against.
  // Live arrivals rerun its layout compensation without changing the target.

  const displayMessages = React.useMemo<InboxDisplayMessage[]>(() => {
    const selectedMessage = messages.find((message) => message.isSelected);
    const pendingReplyMessages: InboxDisplayMessage[] = replies.map(
      (reply) => ({
        ...reply,
        depth: reply.depth ?? (selectedMessage?.depth ?? 0) + 1,
        isSelected: false,
        mentionNames: [],
      }),
    );

    if (messages.length > 0) {
      return [...messages, ...pendingReplyMessages];
    }
    if (!item) return pendingReplyMessages;

    const threadReference = getThreadReference(item.item.tags);
    return [
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
        kind: item.item.kind,
        parentId: threadReference.parentId,
        rootId: threadReference.rootId,
        tags: item.item.tags,
        timeLabel: formatTime(item.item.createdAt),
      },
      ...pendingReplyMessages,
    ];
  }, [item, messages, replies]);
  const videoReviewMessages = React.useMemo(
    () => displayMessages.map(toTimelineMessage),
    [displayMessages],
  );
  const videoReviewChannelType =
    item?.item.channelType === "dm" ||
    item?.item.channelType === "stream" ||
    item?.item.channelType === "forum"
      ? item.item.channelType
      : null;
  const handleSendVideoReviewComment = React.useCallback(
    (
      message: TimelineMessage,
      content: string,
      mentionPubkeys: string[],
      mediaTags?: string[][],
      parentEventId?: string,
    ) =>
      onSendReply({
        content,
        mediaTags,
        mentionPubkeys,
        parentEventId: parentEventId ?? message.id,
      }),
    [onSendReply],
  );
  const videoReviewPresentation = React.useMemo(
    () =>
      buildVideoReviewPresentationByMessageId(
        {
          channelId: item?.item.channelId,
          channelName: contextChannelName ?? item?.channelLabel ?? undefined,
          channelType: videoReviewChannelType,
          isSendingVideoReviewComment: isSendingReply,
          messages: videoReviewMessages,
          onSendVideoReviewComment: canReply
            ? handleSendVideoReviewComment
            : undefined,
          onToggleReaction,
          profiles,
        },
        hasRenderedVideoAttachment,
      ),
    [
      canReply,
      contextChannelName,
      handleSendVideoReviewComment,
      isSendingReply,
      item,
      onToggleReaction,
      profiles,
      videoReviewChannelType,
      videoReviewMessages,
    ],
  );
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

  useComposerHeightPadding(
    scrollContainerRef,
    composerWrapperRef,
    conversationId,
  );

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
  const editTarget =
    displayMessages.find((message) => message.id === editTargetId) ?? null;
  const editMentionState = editTarget
    ? buildEditMentionState(
        editTarget.content,
        editTarget.tags,
        profiles,
        (pubkey) => agentPubkeys?.has(pubkey) === true,
      )
    : null;
  const composerEditTarget = editTarget
    ? {
        author: editTarget.authorLabel,
        body: editTarget.content,
        id: editTarget.id,
        isThreadReply: false,
        imetaMedia: imetaMediaFromTags(editTarget.tags),
        ...editMentionState,
      }
    : null;
  // Explicit sub-message reply wins. Otherwise use the captured default parent
  // (derived from the selected-event anchor at conversation entry), which does
  // not change when a live incoming message advances the representative item.
  const composerParentEventId =
    replyTarget?.id ??
    (isDirectMessage ? null : (capturedDefaultParentId ?? item.id));
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
  const isThreadContext =
    !isDirectMessage && hasInboxThreadContext(item, messages);
  const threadRootTags = isThreadContext
    ? (displayMessages.find((message) => message.id === item.conversationId)
        ?.tags ?? [])
    : [];
  const contextLabel = isThreadContext
    ? isDirectMessage
      ? `Thread with ${item.senderLabel}`
      : channelContextName
        ? `Thread in #${channelContextName}`
        : "Thread"
    : isDirectMessage
      ? `DM with ${item.senderLabel}`
      : channelContextName
        ? `Message in #${channelContextName}`
        : formatInboxTypeLabel(item);
  const contextChannelId = item.item.channelId;
  const sourceEventId = selectedEventId ?? item.id;
  const contextThreadRootId = isThreadContext ? item.conversationId : null;
  const openContextLabel = isThreadContext
    ? "Open full thread"
    : isDirectMessage
      ? "Open conversation"
      : "Open in channel";

  const handleSelectReplyTarget = (message: InboxDisplayMessage) => {
    setReplyTargetId((currentReplyTargetId) =>
      currentReplyTargetId === message.id ? null : message.id,
    );
    onEditTargetChange(null);
    focusComposer();
  };
  const handleSelectEditTarget = (message: InboxDisplayMessage) => {
    onEditTargetChange((currentEditTargetId) =>
      currentEditTargetId === message.id ? null : message.id,
    );
    setReplyTargetId(null);
    focusComposer();
  };

  return (
    <section
      className="flex min-h-0 min-w-0 flex-col overflow-hidden bg-background/60"
      data-testid="home-inbox-detail"
      ref={detailPaneRef}
    >
      <div className="relative flex min-h-0 flex-1 flex-col overflow-hidden">
        <TopChromeInsetHeader flush transparent>
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
                    <h2 className="min-w-0">
                      <button
                        className="block min-w-0 text-left text-sm font-semibold leading-5 tracking-tight text-foreground hover:underline focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
                        data-testid="home-inbox-context-title"
                        onClick={() =>
                          onOpenContext(
                            contextChannelId,
                            sourceEventId,
                            contextThreadRootId,
                          )
                        }
                        title={openContextLabel}
                        type="button"
                      >
                        <span className="block min-w-0 translate-y-px truncate">
                          {contextLabel}
                        </span>
                      </button>
                    </h2>
                  ) : (
                    <h2
                      className="min-w-0 text-sm font-semibold leading-5 tracking-tight text-foreground"
                      title={item.fullTimestampLabel}
                    >
                      <span className="block min-w-0 translate-y-px truncate">
                        {contextLabel}
                      </span>
                    </h2>
                  )}
                </div>
              </div>

              <TooltipProvider>
                <div className="flex shrink-0 items-center gap-1">
                  <UpdateIndicator />
                  {reopenPending || reopenErrored ? (
                    <div
                      aria-live="polite"
                      className={cn(
                        "flex items-center gap-1.5 rounded-full px-2 py-1 text-xs font-medium",
                        reopenErrored
                          ? "bg-destructive/10 text-destructive"
                          : "bg-muted/60 text-muted-foreground",
                      )}
                      data-testid="home-inbox-reopen-status"
                      role="status"
                    >
                      {reopenPending ? (
                        <>
                          <LoaderCircle className="h-3.5 w-3.5 shrink-0 animate-spin" />
                          <span>Reopening…</span>
                        </>
                      ) : (
                        <>
                          <AlertCircle className="h-3.5 w-3.5 shrink-0" />
                          <span>Couldn’t reopen</span>
                          {contextChannelId ? (
                            <button
                              className="ml-0.5 rounded font-semibold underline underline-offset-2 hover:no-underline focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
                              data-testid="home-inbox-reopen-retry"
                              onClick={() =>
                                onOpenContext(
                                  contextChannelId,
                                  sourceEventId,
                                  contextThreadRootId,
                                )
                              }
                              type="button"
                            >
                              Retry
                            </button>
                          ) : null}
                        </>
                      )}
                    </div>
                  ) : null}
                  {canOpenChannel && contextChannelId ? (
                    <Tooltip>
                      <TooltipTrigger asChild>
                        <Button
                          aria-label={openContextLabel}
                          className="rounded-full text-muted-foreground"
                          data-testid="home-inbox-open-context"
                          onClick={() =>
                            onOpenContext(
                              contextChannelId,
                              sourceEventId,
                              contextThreadRootId,
                            )
                          }
                          size="icon"
                          type="button"
                          variant="ghost"
                        >
                          <ExternalLink />
                        </Button>
                      </TooltipTrigger>
                      <TooltipContent>{openContextLabel}</TooltipContent>
                    </Tooltip>
                  ) : null}
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
          className="-mt-13 min-h-0 flex-1 overflow-y-auto overscroll-contain pb-32 pt-13 [overflow-anchor:none]"
          data-testid="home-inbox-detail-scroll"
          onScroll={onScroll}
          ref={scrollContainerRef}
        >
          <div ref={contentRef}>
            {isThreadContextLoading && displayMessages.length <= 1 ? (
              <div
                className="mx-4 mb-2 flex items-center gap-2 rounded-md bg-muted/50 px-3 py-2 text-sm text-muted-foreground"
                data-testid="home-inbox-context-loading"
              >
                <LoaderCircle className="h-4 w-4 shrink-0 animate-spin" />
                <span>Loading surrounding context...</span>
              </div>
            ) : null}
            {hasThreadContextLoadError ? (
              <div
                className="mx-4 mb-2 flex items-center gap-2 rounded-md bg-destructive/10 px-3 py-2 text-sm text-destructive"
                data-testid="home-inbox-context-error"
              >
                <AlertCircle className="h-4 w-4 shrink-0" />
                <span>Some message context could not be loaded.</span>
              </div>
            ) : null}
            {displayMessages.map((message, index) => {
              const hasUnreadBoundary = message.id === unreadBoundaryEventId;
              const isAfterSeparator = index === 1 || hasUnreadBoundary;
              const previousMessage = displayMessages[index - 1];
              const isContinuation =
                !isAfterSeparator &&
                !startsNewMessageGroup(message) &&
                hasSameMessageAuthor(
                  { pubkey: previousMessage?.authorPubkey },
                  { pubkey: message.authorPubkey },
                ) &&
                isWithinGroupingWindow(
                  previousMessage?.createdAt,
                  message.createdAt,
                );

              const canManageMessage = canManageMessageForCurrentUser(
                {
                  id: message.id,
                  author: message.authorLabel,
                  body: message.content,
                  createdAt: message.createdAt,
                  depth: message.depth,
                  kind: message.kind,
                  pubkey: message.authorPubkey,
                  time: message.timeLabel ?? message.fullTimestampLabel,
                },
                currentPubkey,
                profiles,
              );

              const canEditMessage =
                channel?.archivedAt === null && canManageMessage;

              return (
                <InboxMessageRow
                  agentPubkeys={agentPubkeys}
                  canReply={canReply}
                  channelId={item.item.channelId}
                  isContinuation={isContinuation}
                  isFirst={index === 0}
                  isFocusHighlightVisible={isFocusHighlightVisible}
                  key={message.id}
                  message={message}
                  onDelete={
                    canEditMessage
                      ? () => onDeleteMessage(message.id)
                      : undefined
                  }
                  onEdit={canEditMessage ? handleSelectEditTarget : undefined}
                  onSelectReplyTarget={handleSelectReplyTarget}
                  onToggleReaction={onToggleReaction}
                  showUnreadBoundary={hasUnreadBoundary}
                  videoReviewCommentRootId={videoReviewPresentation.commentRootIdsByMessageId.get(
                    message.id,
                  )}
                  videoReviewContext={videoReviewPresentation.contextsByMessageId.get(
                    message.id,
                  )}
                />
              );
            })}
          </div>
        </div>

        <div
          className="pointer-events-none absolute inset-x-0 bottom-0 z-40 isolate before:absolute before:inset-x-0 before:bottom-0 before:h-12 before:bg-gradient-to-b before:from-transparent before:to-background before:content-[''] after:absolute after:inset-x-0 after:bottom-0 after:h-4 after:bg-background after:content-['']"
          data-testid="home-inbox-detail-composer-overlay"
          ref={composerWrapperRef}
        >
          <span
            aria-hidden="true"
            className="absolute bottom-4 left-4 h-4 w-4 bg-background"
            style={{
              maskImage:
                "radial-gradient(circle at top right, transparent 0 1rem, black calc(1rem + 0.5px))",
              WebkitMaskImage:
                "radial-gradient(circle at top right, transparent 0 1rem, black calc(1rem + 0.5px))",
            }}
          />
          <span
            aria-hidden="true"
            className="absolute bottom-4 right-4 h-4 w-4 bg-background"
            style={{
              maskImage:
                "radial-gradient(circle at top left, transparent 0 1rem, black calc(1rem + 0.5px))",
              WebkitMaskImage:
                "radial-gradient(circle at top left, transparent 0 1rem, black calc(1rem + 0.5px))",
            }}
          />
          <div className="pointer-events-auto">
            <MessageComposer
              audienceContext={
                isDirectMessage
                  ? null
                  : {
                      type: "thread",
                      rootTags: threadRootTags,
                    }
              }
              channelId={item.item.channelId}
              channelName={item.channelLabel ?? "channel"}
              channelType={composerChannelType}
              containerClassName="px-4 pb-4 sm:px-4"
              disabled={!canReply && !composerEditTarget}
              draftKey={
                isDirectMessage
                  ? (item.item.channelId ?? item.conversationId)
                  : `thread:${item.conversationId}`
              }
              editTarget={composerEditTarget}
              isSending={isSendingReply || isEditingMessage}
              onCancelEdit={
                composerEditTarget ? () => onEditTargetChange(null) : undefined
              }
              onCancelReply={
                composerReplyTarget ? () => setReplyTargetId(null) : undefined
              }
              onEditSave={async (content, mediaTags, mentionPubkeys) => {
                if (!composerEditTarget) {
                  return;
                }
                // Empty edits are delete shorthand. Keep edit mode active while
                // confirmation is open so Cancel returns to the editor.
                const isEmptyDeletion =
                  content.trim().length === 0 &&
                  (mediaTags === undefined || mediaTags.length === 0);
                if (isEmptyDeletion) {
                  onRequestEmptyEditDelete(composerEditTarget.id);
                  return;
                }
                await onEditSave({
                  content,
                  eventId: composerEditTarget.id,
                  mediaTags,
                  mentionPubkeys,
                });
                onEditTargetChange(null);
              }}
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
                  ? isDirectMessage
                    ? `Message ${item.senderLabel}`
                    : `Send reply to ${item.channelLabel ? `#${item.channelLabel} thread` : "channel thread"}`
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
            relayUrl={activeCommunity?.relayUrl}
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
