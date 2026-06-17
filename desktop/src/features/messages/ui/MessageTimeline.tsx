import * as React from "react";
import { ArrowDown, ArrowUp, Hash } from "lucide-react";

import { getDmParticipantPreview } from "@/features/channels/lib/dmParticipantDisplay";
import type { TimelineMessage } from "@/features/messages/types";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelType } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { Button } from "@/shared/ui/button";
import { Spinner } from "@/shared/ui/spinner";
import { SkeletonReveal } from "@/shared/ui/skeleton";
import { TooltipProvider } from "@/shared/ui/tooltip";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import { TimelineSkeleton, useTimelineSkeletonRows } from "./TimelineSkeleton";
import { TimelineMessageList } from "./TimelineMessageList";
import { useLoadOlderOnScroll } from "./useLoadOlderOnScroll";
import { useTimelineScrollManager } from "./useTimelineScrollManager";

type MessageTimelineProps = {
  agentPubkeys?: ReadonlySet<string>;
  channelId?: string | null;
  channelIntro?: ChannelIntro | null;
  channelName?: string;
  channelType?: ChannelType | null;
  messages: TimelineMessage[];
  directMessageIntro?: {
    displayName: string;
    participants: DirectMessageIntroParticipant[];
  } | null;
  isLoading?: boolean;
  emptyTitle?: string;
  emptyDescription?: string;
  currentPubkey?: string;
  fetchOlder?: () => Promise<void>;
  hasOlderMessages?: boolean;
  /** Optional external ref to the scroll container — used by the parent to
   *  observe scroll position or adjust padding dynamically. */
  scrollContainerRef?: React.RefObject<HTMLDivElement | null>;
  /** True when the timeline has the composer overlay below it. */
  hasComposerOverlay?: boolean;
  isFetchingOlder?: boolean;
  messageFooters?: Record<string, React.ReactNode>;
  /** Map from lowercase pubkey → persona display name for bot members. */
  personaLookup?: Map<string, string>;
  profiles?: UserProfileLookup;
  followThreadById?: (rootId: string) => void;
  isFollowingThreadById?: (rootId: string) => boolean;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onReply?: (message: TimelineMessage) => void;
  isSendingVideoReviewComment?: boolean;
  onSendVideoReviewComment?: (
    message: TimelineMessage,
    content: string,
    mentionPubkeys: string[],
    mediaTags?: string[][],
    parentEventId?: string,
  ) => Promise<void>;
  unfollowThreadById?: (rootId: string) => void;
  onToggleReaction?: (
    message: TimelineMessage,
    emoji: string,
    remove: boolean,
  ) => Promise<void>;
  /** The message ID of the currently active find-in-channel match. */
  searchActiveMessageId?: string | null;
  /** Set of message IDs that match the current find-in-channel query. */
  searchMatchingMessageIds?: Set<string>;
  /** The current find-in-channel query string. */
  searchQuery?: string;
  targetMessageId?: string | null;
  onTargetReached?: (messageId: string) => void;
  /** Event id of the oldest unread top-level message at channel open, or null. */
  firstUnreadMessageId?: string | null;
  /** Count of unread top-level messages at channel open. */
  unreadCount?: number;
  /** Per-thread unread counts keyed by thread root id. */
  threadUnreadCounts?: ReadonlyMap<string, number>;
};

type ChannelIntroAction = {
  description?: string;
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
  testId?: string;
};

type ChannelIntro = {
  actions?: ChannelIntroAction[];
  channelKindLabel: string;
  channelName: string;
  description?: string | null;
  icon?: React.ReactNode;
};

/** Stable empty reference used as the `useDeferredValue` initial value so the
 *  first render on channel entry stays light instead of blocking on the full
 *  message list. Must be module-level so its identity never changes. */
const EMPTY_MESSAGES: TimelineMessage[] = [];

type DirectMessageIntroParticipant = {
  avatarUrl: string | null;
  displayName: string;
  pubkey: string;
};

export const MessageTimeline = React.memo(function MessageTimeline({
  agentPubkeys,
  channelId,
  channelIntro = null,
  directMessageIntro = null,
  messages,
  isLoading = false,
  emptyTitle = "No messages yet",
  emptyDescription = "Send the first message to start the thread.",
  currentPubkey,
  fetchOlder,
  hasComposerOverlay = true,
  hasOlderMessages = true,
  isFetchingOlder = false,
  followThreadById,
  isFollowingThreadById,
  messageFooters,
  personaLookup,
  profiles,
  onDelete,
  onEdit,
  onMarkUnread,
  onReply,
  channelName,
  channelType,
  isSendingVideoReviewComment = false,
  onSendVideoReviewComment,
  onToggleReaction,
  unfollowThreadById,
  scrollContainerRef: externalScrollRef,
  searchActiveMessageId = null,
  searchMatchingMessageIds,
  searchQuery,
  targetMessageId = null,
  onTargetReached,
  firstUnreadMessageId = null,
  unreadCount = 0,
  threadUnreadCounts,
}: MessageTimelineProps) {
  const internalScrollRef = React.useRef<HTMLDivElement>(null);
  const scrollContainerRef = externalScrollRef ?? internalScrollRef;
  const topSentinelRef = React.useRef<HTMLDivElement>(null);

  // Gate the heavy timeline render (each row runs a synchronous
  // react-markdown parse) behind React concurrency. `useDeferredValue` lets the
  // commit that rebuilds the message list yield to higher-priority work, so the
  // main thread stops freezing and the OS no longer shows the busy cursor when
  // entering a channel. We pass `initialValue: []` so even the FIRST render on
  // channel entry stays light — the heavy list streams in on a deferred commit
  // rather than blocking the initial paint. We deliberately drive BOTH the
  // scroll manager and the rendered list off the same deferred value —
  // scroll/autoscroll/deep-link logic reads the DOM (`scrollIntoView`,
  // ResizeObserver on the content), so it must stay consistent with what's
  // actually painted. You can't scroll to a row that hasn't committed yet.
  const deferredMessages = React.useDeferredValue(messages, EMPTY_MESSAGES);
  const isRenderPending = deferredMessages !== messages;
  const scrollRestorationId = targetMessageId
    ? `message-timeline:${channelId ?? "none"}:target:${targetMessageId}`
    : `message-timeline:${channelId ?? "none"}`;

  const {
    bottomAnchorRef,
    contentRef,
    highlightedMessageId,
    isAtBottom,
    newMessageCount,
    restoreScrollPosition,
    scrollToBottom,
    scrollToMessage,
    syncScrollState,
  } = useTimelineScrollManager({
    channelId,
    isLoading,
    messages: deferredMessages,
    onTargetReached,
    scrollContainerRef,
    targetMessageId,
  });

  // The unread pill is a transient, per-open affordance: dismiss it once the
  // user acts on it (jumps to the oldest unread) or catches up by reaching the
  // bottom of the timeline. Reset when the channel changes so a freshly opened
  // channel shows its own pill.
  const [isUnreadPillDismissed, setIsUnreadPillDismissed] =
    React.useState(false);
  // Track whether the pill has been shown at least once this channel visit.
  // This prevents the dismiss effect from firing on mount (when isAtBottom
  // initializes as true) before the pill ever renders.
  const hasShownPillRef = React.useRef(false);
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset on channel switch only
  React.useEffect(() => {
    setIsUnreadPillDismissed(false);
    hasShownPillRef.current = false;
  }, [channelId]);
  React.useEffect(() => {
    if (isAtBottom && hasShownPillRef.current) {
      setIsUnreadPillDismissed(true);
    }
  }, [isAtBottom]);
  const showUnreadPill =
    !isUnreadPillDismissed &&
    unreadCount > 0 &&
    firstUnreadMessageId !== null &&
    !isLoading;
  if (showUnreadPill) hasShownPillRef.current = true;
  const handleJumpToOldestUnread = React.useCallback(() => {
    setIsUnreadPillDismissed(true);
    if (firstUnreadMessageId) {
      scrollToMessage(firstUnreadMessageId);
    }
  }, [firstUnreadMessageId, scrollToMessage]);

  // Scroll to the active search match when it changes.
  const prevSearchActiveRef = React.useRef<string | null>(null);
  // biome-ignore lint/correctness/useExhaustiveDependencies: scrollContainerRef is a stable React ref
  React.useEffect(() => {
    if (
      !searchActiveMessageId ||
      searchActiveMessageId === prevSearchActiveRef.current
    ) {
      prevSearchActiveRef.current = searchActiveMessageId;
      return;
    }
    prevSearchActiveRef.current = searchActiveMessageId;

    const container = scrollContainerRef.current;
    if (!container) return;

    const el = container.querySelector<HTMLElement>(
      `[data-message-id="${searchActiveMessageId}"]`,
    );
    if (el) {
      el.scrollIntoView({ block: "center", behavior: "smooth" });
    }
  }, [searchActiveMessageId]);

  useLoadOlderOnScroll({
    fetchOlder,
    hasOlderMessages,
    isLoading,
    restoreScrollPosition,
    scrollContainerRef,
    sentinelRef: topSentinelRef,
  });

  const showDirectMessageIntro = !isLoading && directMessageIntro !== null;
  const showChannelIntro =
    !isLoading && channelIntro !== null && directMessageIntro === null;
  const showIntro = showDirectMessageIntro || showChannelIntro;
  const showGenericEmpty =
    !isLoading &&
    deferredMessages.length === 0 &&
    directMessageIntro === null &&
    channelIntro === null;
  const showMessageList = !isLoading && deferredMessages.length > 0;
  const timelineSkeletonRows = useTimelineSkeletonRows({
    channelId,
    isLoading,
    messages,
  });

  return (
    <TooltipProvider delayDuration={200}>
      <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        {showUnreadPill ? (
          <div
            className={cn(
              "pointer-events-none absolute inset-x-0 z-20 flex justify-center px-4",
              channelChrome.top,
            )}
          >
            <Button
              className="pointer-events-auto h-7 min-h-7 gap-1.5 rounded-full border-primary/40 bg-primary/10 px-2.5 text-2xs font-medium text-primary shadow-xs backdrop-blur-sm hover:bg-primary/20 [&_svg]:size-3.5"
              data-testid="message-unread-pill"
              onClick={handleJumpToOldestUnread}
              size="sm"
              type="button"
              variant="outline"
            >
              <ArrowUp aria-hidden />
              {`${unreadCount} new message${unreadCount === 1 ? "" : "s"}`}
            </Button>
          </div>
        ) : null}
        <div
          className={cn(
            "absolute inset-0 overflow-y-auto overflow-x-hidden overscroll-contain px-2 pt-1 [overflow-anchor:none]",
            hasComposerOverlay ? "pb-24" : "pb-4",
          )}
          data-scroll-restoration-id={scrollRestorationId}
          data-testid="message-timeline"
          onScroll={syncScrollState}
          ref={scrollContainerRef}
        >
          <div
            className={cn(
              "flex w-full flex-col gap-2",
              channelChrome.contentPadding,
              (showIntro || showGenericEmpty) && "min-h-full",
            )}
            ref={contentRef}
          >
            <div ref={topSentinelRef} aria-hidden className="h-px" />

            {isFetchingOlder ? (
              <div className="flex justify-center py-2">
                <Spinner className="h-4 w-4 border-2 text-muted-foreground" />
              </div>
            ) : null}

            <SkeletonReveal
              className={cn(
                "min-h-[18rem]",
                (showIntro || showGenericEmpty) && "min-h-full",
                showMessageList && !showIntro && "mt-auto",
              )}
              contentClassName={cn(
                "flex min-w-0 flex-col gap-2",
                (showIntro || showGenericEmpty) && "min-h-full",
              )}
              loading={isLoading}
              skeleton={<TimelineSkeleton rows={timelineSkeletonRows} />}
            >
              {showDirectMessageIntro ? (
                <div
                  className="mb-0.5 mt-auto flex w-full flex-col items-start px-3 py-2 text-left"
                  data-testid="message-dm-intro"
                >
                  <DirectMessageIntroAvatarStack
                    participants={directMessageIntro.participants}
                  />
                  <p className="mt-4 max-w-full truncate text-xl font-semibold leading-7 tracking-tight text-foreground">
                    {directMessageIntro.displayName}
                  </p>
                  <p className="mt-1 max-w-full truncate whitespace-nowrap text-sm leading-5 text-muted-foreground">
                    This is the beginning of your direct message with{" "}
                    <span className="font-medium text-foreground">
                      {directMessageIntro.displayName}
                    </span>
                    .
                  </p>
                </div>
              ) : null}

              {showChannelIntro ? (
                <div
                  className="mb-0.5 mt-auto flex w-full max-w-2xl flex-col items-start px-3 py-2 text-left"
                  data-testid="message-channel-intro"
                >
                  <div
                    className="flex h-[60px] w-[60px] items-center justify-center rounded-2xl border border-border/70 bg-muted/40 text-muted-foreground"
                    data-testid="message-channel-intro-icon"
                  >
                    {channelIntro.icon ?? (
                      <Hash aria-hidden className="h-7 w-7" />
                    )}
                  </div>
                  <p className="mt-4 max-w-full truncate text-xl font-semibold leading-7 tracking-tight text-foreground">
                    #{channelIntro.channelName}
                  </p>
                  <p className="mt-1 max-w-full text-sm leading-5 text-muted-foreground">
                    This is the beginning of the{" "}
                    <span className="font-medium text-foreground">
                      {channelIntro.channelKindLabel}
                    </span>
                    .
                  </p>
                  {channelIntro.description ? (
                    <p className="mt-2 max-w-xl text-sm leading-5 text-muted-foreground">
                      {channelIntro.description}
                    </p>
                  ) : null}
                  {channelIntro.actions?.length ? (
                    <div className="mt-4 flex max-w-full flex-nowrap gap-3 overflow-x-auto pb-1">
                      {channelIntro.actions.map((action) => {
                        const hasDescription = Boolean(action.description);

                        return (
                          <button
                            className={cn(
                              "flex shrink-0 border border-border/70 bg-background/70 text-left transition-colors hover:bg-muted/60 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
                              hasDescription
                                ? "h-56 w-[13.75rem] flex-col rounded-2xl p-4"
                                : "h-28 w-64 flex-col rounded-xl p-4",
                            )}
                            data-testid={action.testId}
                            key={action.label}
                            onClick={action.onClick}
                            type="button"
                          >
                            <span
                              className={cn(
                                "flex shrink-0 items-center justify-center rounded-full bg-muted/70 text-muted-foreground",
                                hasDescription
                                  ? "h-12 w-12 [&_svg]:h-6 [&_svg]:w-6"
                                  : "h-10 w-10 [&_svg]:h-5 [&_svg]:w-5",
                              )}
                              data-testid={
                                action.testId
                                  ? `${action.testId}-icon`
                                  : undefined
                              }
                            >
                              {action.icon}
                            </span>
                            <span className="mt-auto min-w-0">
                              <span
                                className="block whitespace-normal break-words text-base font-medium leading-6 text-foreground"
                                data-testid={
                                  action.testId
                                    ? `${action.testId}-title`
                                    : undefined
                                }
                              >
                                {action.label}
                              </span>
                              {action.description ? (
                                <span
                                  className="mt-1 block whitespace-normal break-words text-sm leading-5 text-muted-foreground"
                                  data-testid={
                                    action.testId
                                      ? `${action.testId}-description`
                                      : undefined
                                  }
                                >
                                  {action.description}
                                </span>
                              ) : null}
                            </span>
                          </button>
                        );
                      })}
                    </div>
                  ) : null}
                </div>
              ) : null}

              {showGenericEmpty ? (
                <div
                  className="mt-auto rounded-3xl border border-dashed border-border/80 bg-card/70 px-6 py-10 text-center shadow-xs"
                  data-testid="message-empty"
                >
                  <p className="text-base font-semibold tracking-tight">
                    {emptyTitle}
                  </p>
                  <p className="mt-2 text-sm text-muted-foreground">
                    {emptyDescription}
                  </p>
                </div>
              ) : null}

              {showMessageList ? (
                <div
                  className={cn(
                    "flex flex-col gap-2",
                    !showIntro && "mt-auto",
                    // While a deferred render is in flight the painted
                    // list lags the latest `messages`. Dim it slightly so the
                    // streaming-in feels intentional instead of frozen.
                    isRenderPending && "opacity-60 transition-opacity",
                  )}
                  data-render-pending={isRenderPending ? "true" : undefined}
                >
                  <TimelineMessageList
                    agentPubkeys={agentPubkeys}
                    channelId={channelId}
                    channelName={channelName}
                    channelType={channelType}
                    currentPubkey={currentPubkey}
                    firstUnreadMessageId={firstUnreadMessageId}
                    followThreadById={followThreadById}
                    highlightedMessageId={highlightedMessageId}
                    isFollowingThreadById={isFollowingThreadById}
                    messageFooters={messageFooters}
                    messages={deferredMessages}
                    onDelete={onDelete}
                    onEdit={onEdit}
                    onMarkUnread={onMarkUnread}
                    onReply={onReply}
                    isSendingVideoReviewComment={isSendingVideoReviewComment}
                    onSendVideoReviewComment={onSendVideoReviewComment}
                    onToggleReaction={onToggleReaction}
                    personaLookup={personaLookup}
                    profiles={profiles}
                    searchActiveMessageId={searchActiveMessageId}
                    searchMatchingMessageIds={searchMatchingMessageIds}
                    searchQuery={searchQuery}
                    threadUnreadCounts={threadUnreadCounts}
                    unfollowThreadById={unfollowThreadById}
                  />
                </div>
              ) : null}
            </SkeletonReveal>

            <div aria-hidden className="h-px" ref={bottomAnchorRef} />
          </div>
        </div>

        {!isAtBottom ? (
          <div
            className={cn(
              "pointer-events-none absolute inset-x-0 z-20 flex justify-center px-4",
              hasComposerOverlay ? "bottom-36" : "bottom-4",
            )}
          >
            <Button
              className="pointer-events-auto h-7 min-h-7 gap-1.5 rounded-full border-border/50 bg-background/85 px-2.5 text-2xs font-medium text-muted-foreground shadow-xs backdrop-blur-sm hover:bg-muted/70 hover:text-foreground [&_svg]:size-4"
              data-testid="message-scroll-to-latest"
              onClick={() => {
                scrollToBottom("smooth");
              }}
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
      </div>
    </TooltipProvider>
  );
});

function DirectMessageIntroAvatarStack({
  participants,
}: {
  participants: DirectMessageIntroParticipant[];
}) {
  const { hiddenCount, visibleParticipants } =
    getDmParticipantPreview(participants);
  const stackItemCount = visibleParticipants.length + (hiddenCount > 0 ? 1 : 0);

  return (
    <div
      aria-hidden="true"
      className="flex shrink-0 items-center"
      data-testid="message-dm-intro-avatar-stack"
    >
      {visibleParticipants.map((participant, index) => (
        <div
          className={index > 0 ? "-ml-5" : ""}
          data-testid="message-dm-intro-avatar-stack-participant"
          key={participant.pubkey}
          style={{
            zIndex: index + 1,
            ...(index < stackItemCount - 1 && {
              mask: "radial-gradient(circle 34px at calc(100% + 10px) 50%, transparent 99%, #fff 100%)",
              WebkitMask:
                "radial-gradient(circle 34px at calc(100% + 10px) 50%, transparent 99%, #fff 100%)",
            }),
          }}
        >
          <UserAvatar
            avatarUrl={participant.avatarUrl}
            className="h-[60px] w-[60px] text-base"
            displayName={participant.displayName}
            size="md"
          />
        </div>
      ))}
      {hiddenCount > 0 ? (
        <div
          className={visibleParticipants.length > 0 ? "-ml-5" : ""}
          data-testid="message-dm-intro-avatar-stack-more"
          style={{ zIndex: stackItemCount }}
        >
          <span className="flex h-[60px] w-[60px] items-center justify-center rounded-full bg-secondary font-semibold text-secondary-foreground shadow-xs">
            <span className="text-lg leading-none">+{hiddenCount}</span>
          </span>
        </div>
      ) : null}
    </div>
  );
}
