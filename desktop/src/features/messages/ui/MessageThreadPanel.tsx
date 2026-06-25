import * as React from "react";
import { ArrowDown, ArrowLeft, X } from "lucide-react";

import type { AgentConversationMarker } from "@/features/agents/agentConversations";
import type { TranscriptItem } from "@/features/agents/ui/agentSessionTypes";
import { useAgentTranscript } from "@/features/agents/ui/useObserverEvents";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import type { ImetaMedia } from "@/features/messages/lib/imetaMediaMarkdown";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { Channel } from "@/shared/api/types";
import { useEscapeKey } from "@/shared/hooks/useEscapeKey";
import { useIsThreadPanelOverlay } from "@/shared/hooks/use-mobile";
import { THREAD_PANEL_MIN_WIDTH_PX } from "@/shared/hooks/useThreadPanelWidth";
import { cn } from "@/shared/lib/cn";
import {
  AuxiliaryPanelHeader,
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelTitle,
  auxiliaryPanelContentPaddingClass,
} from "@/shared/layout/AuxiliaryPanelHeader";
import { Button } from "@/shared/ui/button";
import {
  OverlayPanelBackdrop,
  PANEL_ENTER_BASE_CLASS,
  PANEL_OVERLAY_CLASS,
  PANEL_SINGLE_COLUMN_HEADER_LAYER_CLASS,
} from "@/shared/ui/OverlayPanelBackdrop";
import { Shimmer } from "@/shared/ui/Shimmer";
import { Skeleton } from "@/shared/ui/skeleton";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import type { VideoReviewContext } from "@/shared/ui/VideoPlayer";
import { AgentConversationMarkerRow } from "./AgentConversationMarkerRow";
import { MessageAuthorText, MessageHeaderRow } from "./MessageHeader";
import { MessageComposer } from "./MessageComposer";
import { MessageRow } from "./MessageRow";
import { TypingIndicatorRow } from "./TypingIndicatorRow";
import { UnreadDivider } from "./UnreadDivider";
import { useComposerHeightPadding } from "./useComposerHeightPadding";
import { useAnchoredScroll } from "./useAnchoredScroll";
import { selectDeferredListRenderState } from "@/features/messages/lib/timelineSnapshot";

type MessageThreadPanelProps = {
  agentConversationMarkers?: readonly AgentConversationMarker[];
  agentPubkeys?: ReadonlySet<string>;
  channel: Channel | null;
  channelId: string | null;
  channelName: string;
  currentPubkey?: string;
  disabled?: boolean;
  firstUnreadReplyId?: string | null;
  layout?: "standalone" | "split";
  editTarget?: {
    author: string;
    body: string;
    id: string;
    imetaMedia?: ImetaMedia[];
  } | null;
  isSending: boolean;
  isSinglePanelView?: boolean;
  onCancelEdit?: () => void;
  onCancelReply: () => void;
  onClose: () => void;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onEditLastOwnMessage?: () => boolean;
  onEditSave?: (content: string, mediaTags?: string[][]) => Promise<void>;
  onMarkUnread?: (message: TimelineMessage) => void;
  onMarkRead?: (message: TimelineMessage) => void;
  onOpenAgentConversation?: (
    message: TimelineMessage,
    options?: { publishMarker?: boolean },
  ) => void;
  onExpandReplies: (message: TimelineMessage) => void;
  onScrollTargetResolved: () => void;
  onSelectReplyTarget: (message: TimelineMessage) => void;
  onSend: (
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
  ) => Promise<void>;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
  profiles?: UserProfileLookup;
  replyTargetMessage: TimelineMessage | null;
  scrollTargetId: string | null;
  threadHead: TimelineMessage | null;
  threadReplies: MainTimelineEntry[];
  threadUnreadCount?: number;
  threadReplyUnreadCounts?: ReadonlyMap<string, number>;
  threadActivityAgents?: readonly ThreadActivityAgent[];
  threadTypingPubkeys: string[];
  threadHeadVideoReviewContext?: VideoReviewContext;
  widthPx: number;
  isFollowingThread?: boolean;
  isMessageUnreadById?: (messageId: string) => boolean;
  onFollowThread?: () => void;
  onUnfollowThread?: () => void;
};

type ThreadActivityAgent = {
  name: string;
  pubkey: string;
};

/** Stable `useDeferredValue` initial value; mirrors `EMPTY_MESSAGES`. */
const EMPTY_THREAD_REPLIES: MainTimelineEntry[] = [];
const THREAD_PANEL_MESSAGE_GUTTER_CLASS = "px-2";
const THREAD_PANEL_COMPOSER_GUTTER_CLASS = "px-5";

type MessageThreadPanelSkeletonProps = {
  isSinglePanelView?: boolean;
  layout?: "standalone" | "split";
  onClose: () => void;
  widthPx: number;
};

function canManageMessage(
  message: TimelineMessage,
  currentPubkey: string | undefined,
): boolean {
  return Boolean(
    currentPubkey &&
      message.pubkey &&
      currentPubkey.toLowerCase() === message.pubkey.toLowerCase(),
  );
}

function normalizeActivityText(value: string) {
  return value.toLowerCase().replace(/[_-]+/g, " ").replace(/\s+/g, " ").trim();
}

function getActivityLabel(item: TranscriptItem): string {
  if (item.type === "message") {
    return item.role === "assistant" ? "Responding..." : "Thinking...";
  }

  if (item.type !== "tool") {
    return "Thinking...";
  }

  const activityText = normalizeActivityText(
    [item.buzzToolName, item.toolName, item.title]
      .filter((value): value is string => Boolean(value))
      .join(" "),
  );

  if (/\b(send message|send)\b/.test(activityText)) {
    return "Responding...";
  }

  if (
    /\b(review|diff|compare|pull request|pr|changes?|patch)\b/.test(
      activityText,
    )
  ) {
    return "Reviewing...";
  }

  if (
    /\b(edit|write|update|create|delete|set|add|remove|join|leave|archive|unarchive|publish|trigger|approve|vote)\b/.test(
      activityText,
    )
  ) {
    return "Editing...";
  }

  if (
    /\b(search|find|lookup|query|fetch|get|list|read|retrieve|history|thread|channel|user|feed|canvas|presence)\b/.test(
      activityText,
    )
  ) {
    return "Searching...";
  }

  return "Thinking...";
}

function ThreadAgentActivityRow({
  agent,
  channelId,
  profiles,
}: {
  agent: ThreadActivityAgent;
  channelId: string | null;
  profiles?: UserProfileLookup;
}) {
  const transcript = useAgentTranscript(true, agent.pubkey);
  const activityLabel = React.useMemo(() => {
    const scopedTranscript = channelId
      ? transcript.filter((item) => item.channelId === channelId)
      : transcript;

    const latestActivity = scopedTranscript[scopedTranscript.length - 1];
    return latestActivity ? getActivityLabel(latestActivity) : "Thinking...";
  }, [channelId, transcript]);
  const profile = profiles?.[agent.pubkey.toLowerCase()];

  return (
    <article
      aria-live="polite"
      className="group/message relative z-10 mx-1 flex items-start gap-2.5 rounded-2xl px-2 py-2 transition-colors hover:bg-muted/50 focus-within:bg-muted/50"
      data-testid="message-thread-agent-activity-row"
    >
      <UserAvatar
        accent
        avatarUrl={profile?.avatarUrl ?? null}
        className="!h-10 !w-10 shrink-0"
        displayName={profile?.displayName || agent.name}
        testId="message-avatar"
      />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <MessageHeaderRow>
          <MessageAuthorText>
            {profile?.displayName || agent.name}
          </MessageAuthorText>
        </MessageHeaderRow>
        <p className="-mt-0.5 max-w-full truncate text-sm text-foreground">
          <Shimmer className="align-baseline">{activityLabel}</Shimmer>
        </p>
      </div>
    </article>
  );
}

function ThreadMessageSkeleton({ isHead = false }: { isHead?: boolean }) {
  return (
    <article className="relative flex items-start gap-2.5 rounded-2xl px-3 py-2">
      <Skeleton className="h-9 w-9 shrink-0 rounded-full" />
      <div className="-mt-1 min-w-0 flex-1">
        <div className="flex min-w-0 flex-wrap items-baseline gap-x-2 gap-y-0">
          <Skeleton className="h-[15px] w-28" />
          <Skeleton className="h-3 w-16" />
        </div>
        <div className="mt-1 space-y-1.5 pb-2">
          <Skeleton className="h-4 w-full" />
          <Skeleton className={isHead ? "h-4 w-4/5" : "h-4 w-2/3"} />
        </div>
        <div className="flex items-center gap-4">
          <Skeleton className="h-4 w-8 rounded-full" />
          <Skeleton className="h-4 w-8 rounded-full" />
          <Skeleton className="h-4 w-8 rounded-full" />
        </div>
      </div>
    </article>
  );
}

function ThreadComposerSkeleton() {
  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-0 z-10">
      <div className="pointer-events-auto">
        <div
          className={cn(
            "relative z-10 shrink-0 bg-transparent pb-2 pt-0",
            THREAD_PANEL_COMPOSER_GUTTER_CLASS,
          )}
        >
          <div className="relative isolate rounded-2xl border border-border/50 bg-background/80 px-3 pb-2 pt-3 shadow-none backdrop-blur-md sm:px-4">
            <Skeleton className="h-5 w-48 max-w-full" />
            <div className="mt-4 flex items-center gap-2">
              <Skeleton className="h-8 w-8 rounded-lg" />
              <Skeleton className="h-8 w-8 rounded-lg" />
              <Skeleton className="ml-auto h-8 w-20 rounded-full" />
            </div>
          </div>
        </div>
        <div
          className={cn(
            "h-7 bg-background pb-1 pt-0",
            THREAD_PANEL_COMPOSER_GUTTER_CLASS,
          )}
        />
      </div>
    </div>
  );
}

export function MessageThreadPanelSkeleton({
  isSinglePanelView = false,
  layout = "standalone",
  onClose,
  widthPx,
}: MessageThreadPanelSkeletonProps) {
  const isOverlay = useIsThreadPanelOverlay();
  const isFloatingOverlay = isOverlay && !isSinglePanelView;
  const isSplitLayout = layout === "split";
  useEscapeKey(onClose, isOverlay || isSinglePanelView);

  const threadHeaderContent = (
    <>
      <AuxiliaryPanelHeaderGroup>
        {isSinglePanelView ? (
          <Button
            aria-label="Back to conversation"
            className="shrink-0"
            onClick={onClose}
            size="icon"
            type="button"
            variant="outline"
          >
            <ArrowLeft />
          </Button>
        ) : null}
        <AuxiliaryPanelTitle>Thread</AuxiliaryPanelTitle>
      </AuxiliaryPanelHeaderGroup>
      <Button
        aria-label="Close thread"
        className="ml-auto"
        onClick={onClose}
        size="icon"
        type="button"
        variant="ghost"
      >
        <X />
      </Button>
    </>
  );

  const threadBody = (
    <div
      className={cn(
        "min-h-0 flex-1 overflow-y-auto overflow-x-hidden overscroll-contain pb-24 [overflow-anchor:none]",
        isSplitLayout && auxiliaryPanelContentPaddingClass,
        !isSplitLayout && !isFloatingOverlay && "pt-[3.25rem]",
      )}
      data-testid="message-thread-loading"
    >
      <div
        className={cn(THREAD_PANEL_MESSAGE_GUTTER_CLASS, "pb-1 pt-0")}
        data-testid="message-thread-head-loading"
      >
        <ThreadMessageSkeleton isHead />
      </div>
      <div
        className={cn(
          "space-y-2.5 pb-3 pt-1",
          THREAD_PANEL_MESSAGE_GUTTER_CLASS,
        )}
      >
        <ThreadMessageSkeleton />
        <ThreadMessageSkeleton />
        <div className="ml-[58px] flex items-center gap-1.5 pt-0.5">
          <Skeleton className="h-7 w-7 rounded-full" />
          <Skeleton className="h-7 w-7 rounded-full" />
          <Skeleton className="h-4 w-28 rounded-full" />
        </div>
      </div>
    </div>
  );

  if (isSplitLayout) {
    return (
      <div className="relative flex min-h-0 flex-1 flex-col">
        <AuxiliaryPanelHeader>{threadHeaderContent}</AuxiliaryPanelHeader>
        {threadBody}
        <ThreadComposerSkeleton />
      </div>
    );
  }

  return (
    <>
      {isFloatingOverlay && <OverlayPanelBackdrop onClose={onClose} />}
      <aside
        className={cn(
          PANEL_ENTER_BASE_CLASS,
          isSinglePanelView && "border-l-0",
          isFloatingOverlay && PANEL_OVERLAY_CLASS,
        )}
        data-testid="message-thread-panel"
        style={{
          width: isSinglePanelView
            ? "100%"
            : `min(${widthPx}px, calc(100% - ${THREAD_PANEL_MIN_WIDTH_PX}px))`,
        }}
      >
        <div
          className={cn(
            "flex cursor-default select-none items-center",
            isSinglePanelView
              ? `relative ${PANEL_SINGLE_COLUMN_HEADER_LAYER_CLASS} -mb-[3.25rem] min-h-[3.25rem] shrink-0 gap-2.5 bg-background/80 px-4 py-2 backdrop-blur-md supports-[backdrop-filter]:bg-background/70 sm:pr-3 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55`
              : "relative z-50 min-h-[3.25rem] shrink-0 gap-3 bg-background/80 px-5 py-2 backdrop-blur-md supports-[backdrop-filter]:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55",
          )}
          data-tauri-drag-region
        >
          {threadHeaderContent}
        </div>

        {threadBody}
        <ThreadComposerSkeleton />
      </aside>
    </>
  );
}

export function MessageThreadPanel({
  agentConversationMarkers,
  agentPubkeys,
  channel,
  channelId,
  channelName,
  currentPubkey,
  disabled = false,
  firstUnreadReplyId,
  layout = "standalone",
  editTarget,
  isSending,
  isSinglePanelView = false,
  isFollowingThread,
  isMessageUnreadById,
  onCancelEdit,
  onCancelReply,
  onClose,
  onDelete,
  onEdit,
  onEditLastOwnMessage,
  onEditSave,
  onFollowThread,
  onMarkUnread,
  onMarkRead,
  onOpenAgentConversation,
  onScrollTargetResolved,
  onSelectReplyTarget,
  onSend,
  onToggleReaction,
  onUnfollowThread,
  profiles,
  replyTargetMessage,
  scrollTargetId,
  threadHead,
  threadHeadVideoReviewContext,
  threadActivityAgents = [],
  threadReplies,
  threadTypingPubkeys,
  widthPx,
}: MessageThreadPanelProps) {
  const threadBodyRef = React.useRef<HTMLDivElement>(null);
  const threadContentRef = React.useRef<HTMLDivElement>(null);
  const threadComposerWrapperRef = React.useRef<HTMLDivElement>(null);
  const isOverlay = useIsThreadPanelOverlay();
  const isFloatingOverlay = isOverlay && !isSinglePanelView;
  const isSplitLayout = layout === "split";
  const threadHeadId = threadHead?.id ?? null;
  useEscapeKey(onClose, isOverlay || isSinglePanelView);
  useComposerHeightPadding(
    threadBodyRef,
    threadComposerWrapperRef,
    isSinglePanelView,
  );

  const composerReplyTarget =
    replyTargetMessage && threadHead && replyTargetMessage.id !== threadHead.id
      ? {
          author: replyTargetMessage.author,
          body: replyTargetMessage.body,
          id: replyTargetMessage.id,
        }
      : null;

  const deferredThreadReplies = React.useDeferredValue(
    threadReplies,
    EMPTY_THREAD_REPLIES,
  );
  const isRepliesPending = deferredThreadReplies !== threadReplies;

  // Which of the three states the reply region paints this frame. Delegated to
  // a pure helper so the "don't flash empty over an incoming list" rule is
  // covered in the lib test suite (see selectDeferredListRenderState).
  const repliesRenderState = selectDeferredListRenderState(
    deferredThreadReplies.length,
    threadReplies.length,
  );
  const threadMessages = React.useMemo(
    () => deferredThreadReplies.map((entry) => entry.message),
    [deferredThreadReplies],
  );
  const flatThreadReplyEntries = React.useMemo(
    () =>
      deferredThreadReplies.map((entry) => ({
        ...entry,
        message:
          entry.message.depth === 0
            ? entry.message
            : { ...entry.message, depth: 0 },
      })),
    [deferredThreadReplies],
  );
  const agentConversationMarkerByMessageId = React.useMemo(
    () =>
      new Map(
        (agentConversationMarkers ?? []).map((marker) => [
          marker.agentReplyId,
          marker,
        ]),
      ),
    [agentConversationMarkers],
  );

  const { isAtBottom, newMessageCount, onScroll, scrollToBottom } =
    useAnchoredScroll({
      channelId: threadHeadId,
      contentRef: threadContentRef,
      isLoading: repliesRenderState === "pending",
      messages: threadMessages,
      onTargetReached: onScrollTargetResolved,
      scrollContainerRef: threadBodyRef,
      targetMessageId: scrollTargetId,
    });

  if (!threadHead) {
    return null;
  }

  const threadHeadAgentConversationMarker =
    agentConversationMarkerByMessageId.get(threadHead.id) ?? null;
  const threadActivityRows =
    threadActivityAgents.length > 0
      ? threadActivityAgents.map((agent) => (
          <ThreadAgentActivityRow
            agent={agent}
            channelId={channelId}
            key={agent.pubkey}
            profiles={profiles}
          />
        ))
      : null;

  const threadScrollRegion = (
    <div
      className={cn(
        "min-h-0 flex-1 overflow-y-auto overflow-x-hidden overscroll-contain pb-24 [overflow-anchor:none]",
        isSplitLayout && auxiliaryPanelContentPaddingClass,
        !isSplitLayout && !isFloatingOverlay && "pt-[3.25rem]",
      )}
      data-testid="message-thread-body"
      onScroll={onScroll}
      ref={threadBodyRef}
    >
      <div ref={threadContentRef}>
        <div
          className={cn(THREAD_PANEL_MESSAGE_GUTTER_CLASS, "pb-1 pt-0")}
          data-testid="message-thread-head"
        >
          <div className="rounded-2xl">
            <MessageRow
              actionBarPlacement="inside"
              agentPubkeys={agentPubkeys}
              channelId={channelId}
              isFollowingThread={isFollowingThread}
              isUnread={isMessageUnreadById?.(threadHead.id)}
              message={threadHead}
              onDelete={
                onDelete && canManageMessage(threadHead, currentPubkey)
                  ? onDelete
                  : undefined
              }
              onEdit={
                onEdit && canManageMessage(threadHead, currentPubkey)
                  ? onEdit
                  : undefined
              }
              onFollowThread={
                onFollowThread ? (_msg) => onFollowThread() : undefined
              }
              onMarkUnread={onMarkUnread}
              onMarkRead={onMarkRead}
              onOpenAgentConversation={onOpenAgentConversation}
              onToggleReaction={onToggleReaction}
              onUnfollowThread={
                onUnfollowThread ? (_msg) => onUnfollowThread() : undefined
              }
              profiles={profiles}
              showDepthGuides={false}
              videoReviewContext={threadHeadVideoReviewContext}
            />
            {threadHeadAgentConversationMarker ? (
              <AgentConversationMarkerRow
                currentPubkey={currentPubkey}
                marker={threadHeadAgentConversationMarker}
                message={threadHead}
                onOpenAgentConversation={onOpenAgentConversation}
                profiles={profiles}
              />
            ) : null}
          </div>
        </div>

        <div
          className={cn(THREAD_PANEL_MESSAGE_GUTTER_CLASS, "pb-3 pt-0")}
          data-testid="message-thread-replies"
        >
          {repliesRenderState === "list" ? (
            <div
              className="space-y-0"
              data-render-pending={isRepliesPending ? "true" : undefined}
            >
              {flatThreadReplyEntries.map((entry, index) => {
                const showUnreadDivider =
                  index > 0 && entry.message.id === firstUnreadReplyId;
                const agentConversationMarker =
                  agentConversationMarkerByMessageId.get(entry.message.id) ??
                  null;

                return (
                  <div
                    className="flex flex-col gap-0"
                    key={entry.message.renderKey ?? entry.message.id}
                  >
                    {showUnreadDivider ? <UnreadDivider /> : null}
                    <MessageRow
                      agentPubkeys={agentPubkeys}
                      channelId={channelId}
                      isUnread={isMessageUnreadById?.(entry.message.id)}
                      message={entry.message}
                      onDelete={
                        onDelete &&
                        canManageMessage(entry.message, currentPubkey)
                          ? onDelete
                          : undefined
                      }
                      onEdit={
                        onEdit && canManageMessage(entry.message, currentPubkey)
                          ? onEdit
                          : undefined
                      }
                      onMarkUnread={onMarkUnread}
                      onMarkRead={onMarkRead}
                      onOpenAgentConversation={onOpenAgentConversation}
                      onReply={
                        onSelectReplyTarget
                          ? () => onSelectReplyTarget(entry.message)
                          : undefined
                      }
                      onToggleReaction={onToggleReaction}
                      profiles={profiles}
                      showDepthGuides={false}
                    />
                    {agentConversationMarker ? (
                      <AgentConversationMarkerRow
                        currentPubkey={currentPubkey}
                        marker={agentConversationMarker}
                        message={entry.message}
                        onOpenAgentConversation={onOpenAgentConversation}
                        profiles={profiles}
                      />
                    ) : null}
                  </div>
                );
              })}
              {threadActivityRows}
            </div>
          ) : repliesRenderState === "empty" && !threadActivityRows ? (
            // Only show the empty state when the thread is GENUINELY empty.
            // Keying off `deferredThreadReplies` would flash "No replies" for a
            // frame while a non-empty list streams in on the deferred commit.
            <div className="rounded-2xl border border-dashed border-border/70 bg-card/40 px-4 py-6 text-center">
              <p className="text-sm font-medium text-foreground/80">
                No replies in this branch yet
              </p>
              <p className="mt-1 text-xs text-muted-foreground">
                Reply in the thread to continue this branch.
              </p>
            </div>
          ) : repliesRenderState === "empty" ? (
            <div className="space-y-0">{threadActivityRows}</div>
          ) : // "pending": deferred list is empty but the live list has content —
          // rows are streaming in on the deferred commit. Paint nothing rather
          // than flashing the empty state.
          null}
        </div>
      </div>
    </div>
  );

  const threadFooter = (
    <>
      {!isAtBottom ? (
        <div className="pointer-events-none absolute inset-x-0 bottom-36 z-20 flex justify-center px-4">
          <Button
            className="pointer-events-auto h-7 min-h-7 gap-1.5 rounded-full border-border/50 bg-background/85 px-2.5 text-2xs font-medium text-muted-foreground shadow-xs backdrop-blur-sm hover:bg-muted/70 hover:text-foreground [&_svg]:size-4"
            data-testid="thread-scroll-to-latest"
            onClick={() => scrollToBottom("smooth")}
            size="sm"
            type="button"
            variant="outline"
          >
            <ArrowDown aria-hidden />
            {newMessageCount > 0
              ? `${newMessageCount} new message${newMessageCount === 1 ? "" : "s"}`
              : "Jump to latest"}
          </Button>
        </div>
      ) : null}

      <div
        className="pointer-events-none absolute inset-x-0 bottom-0 z-10"
        ref={threadComposerWrapperRef}
      >
        <div className="pointer-events-auto">
          <MessageComposer
            channelId={channelId}
            channelName={channelName}
            channelType={channel?.channelType ?? null}
            containerClassName={THREAD_PANEL_COMPOSER_GUTTER_CLASS}
            disabled={disabled || isSending || !channelId}
            draftKey={`thread:${threadHead.id}`}
            editTarget={editTarget}
            isSending={isSending}
            onCancelEdit={onCancelEdit}
            onCancelReply={composerReplyTarget ? onCancelReply : undefined}
            onEditLastOwnMessage={onEditLastOwnMessage}
            onEditSave={onEditSave}
            onSend={onSend}
            placeholder={`Reply in thread to ${threadHead.author}`}
            profiles={profiles}
            replyTarget={composerReplyTarget}
            typingParentEventId={threadHead.id}
            typingRootEventId={threadHead.rootId}
          />
          <div
            className={cn(
              "h-7 bg-background pb-1 pt-0",
              THREAD_PANEL_COMPOSER_GUTTER_CLASS,
            )}
          >
            <div className="mx-auto flex h-full w-full max-w-4xl items-center gap-2">
              {threadTypingPubkeys.length > 0 ? (
                <TypingIndicatorRow
                  channel={channel}
                  className="min-w-0 flex-1 py-0 pl-[calc(0.75rem+1px)] pr-0 sm:pl-[calc(1rem+1px)]"
                  currentPubkey={currentPubkey}
                  profiles={profiles}
                  typingPubkeys={threadTypingPubkeys}
                  variant="activity"
                />
              ) : null}
            </div>
          </div>
        </div>
      </div>
    </>
  );

  const threadHeaderContent = (
    <>
      <AuxiliaryPanelHeaderGroup>
        {isSinglePanelView ? (
          <Button
            aria-label="Back to conversation"
            className="shrink-0"
            data-testid="message-thread-back"
            onClick={onClose}
            size="icon"
            type="button"
            variant="outline"
          >
            <ArrowLeft />
          </Button>
        ) : null}
        <AuxiliaryPanelTitle>Thread</AuxiliaryPanelTitle>
      </AuxiliaryPanelHeaderGroup>
      <Button
        aria-label="Close thread"
        className="ml-auto"
        data-testid="message-thread-close"
        onClick={onClose}
        size="icon"
        type="button"
        variant="ghost"
      >
        <X />
      </Button>
    </>
  );

  if (isSplitLayout) {
    return (
      <div className="relative flex min-h-0 flex-1 flex-col">
        <AuxiliaryPanelHeader>{threadHeaderContent}</AuxiliaryPanelHeader>
        {threadScrollRegion}
        {threadFooter}
      </div>
    );
  }

  return (
    <>
      {isFloatingOverlay && <OverlayPanelBackdrop onClose={onClose} />}
      <aside
        className={cn(
          PANEL_ENTER_BASE_CLASS,
          isSinglePanelView && "border-l-0",
          isFloatingOverlay && PANEL_OVERLAY_CLASS,
        )}
        data-testid="message-thread-panel"
        style={{
          width: isSinglePanelView
            ? "100%"
            : `min(${widthPx}px, calc(100% - ${THREAD_PANEL_MIN_WIDTH_PX}px))`,
        }}
      >
        <div
          className={cn(
            "flex cursor-default select-none items-center",
            isSinglePanelView
              ? `relative ${PANEL_SINGLE_COLUMN_HEADER_LAYER_CLASS} -mb-[3.25rem] min-h-[3.25rem] shrink-0 gap-2.5 bg-background/80 px-4 py-2 backdrop-blur-md supports-[backdrop-filter]:bg-background/70 sm:pr-3 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55`
              : "relative z-50 min-h-[3.25rem] shrink-0 gap-3 bg-background/80 px-5 py-2 backdrop-blur-md supports-[backdrop-filter]:bg-background/70 dark:bg-background/70 dark:backdrop-blur-xl dark:supports-[backdrop-filter]:bg-background/55",
          )}
          data-tauri-drag-region
        >
          {threadHeaderContent}
        </div>

        {threadScrollRegion}
        {threadFooter}
      </aside>
    </>
  );
}
