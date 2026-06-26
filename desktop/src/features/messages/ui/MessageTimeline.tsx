import * as React from "react";
import { Hash } from "lucide-react";

import {
  isDeferredTimelineSnapshotStale,
  isRenderedTimelineBehindHistoryPrepend,
  selectTimelineBodySurface,
  selectTimelineIntroSurface,
} from "@/features/messages/lib/timelineSnapshot";
import { getDmParticipantPreview } from "@/features/channels/lib/dmParticipantDisplay";
import type { TimelineMessage } from "@/features/messages/types";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import { buildMainTimelineEntries } from "@/features/messages/lib/threadPanel";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelType } from "@/shared/api/types";
import { buildTimelineItems } from "@/features/messages/lib/timelineItems";
import { cn } from "@/shared/lib/cn";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { Spinner } from "@/shared/ui/spinner";
import { TooltipProvider } from "@/shared/ui/tooltip";
import { UnreadPill, unreadCountLabel } from "@/shared/ui/UnreadPill";
import { UserAvatar } from "@/shared/ui/UserAvatar";
import type { ListVirtualizer } from "@/shared/ui/VirtualizedList";
import { TimelineSkeleton, useTimelineSkeletonRows } from "./TimelineSkeleton";
import { TimelineMessageList } from "./TimelineMessageList";
import { useAnchoredScroll } from "./useAnchoredScroll";
import { useConvergentScrollToMessage } from "./useConvergentScrollToMessage";
import { useLoadOlderOnScroll } from "./useLoadOlderOnScroll";

export type MessageTimelineHandle = {
  scrollToBottomOnNextUpdate: () => void;
};

type MessageTimelineProps = {
  agentPubkeys?: ReadonlySet<string>;
  channelId?: string | null;
  channelIntro?: ChannelIntro | null;
  channelName?: string;
  channelType?: ChannelType | null;
  messages: TimelineMessage[];
  mainEntries?: MainTimelineEntry[];
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
  isMessageUnreadById?: (messageId: string) => boolean;
  onDelete?: (message: TimelineMessage) => void;
  onEdit?: (message: TimelineMessage) => void;
  onMarkUnread?: (message: TimelineMessage) => void;
  onMarkRead?: (message: TimelineMessage) => void;
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

type TimelineSnapshot = {
  channelId: string | null;
  messages: TimelineMessage[];
};

const EMPTY_TIMELINE_SNAPSHOT: TimelineSnapshot = {
  channelId: null,
  messages: EMPTY_MESSAGES,
};

const MessageTimelineBase = React.forwardRef<
  MessageTimelineHandle,
  MessageTimelineProps
>(function MessageTimeline(
  {
    agentPubkeys,
    channelId,
    channelIntro = null,
    directMessageIntro = null,
    messages,
    mainEntries,
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
    isMessageUnreadById,
    messageFooters,
    personaLookup,
    profiles,
    onDelete,
    onEdit,
    onMarkUnread,
    onMarkRead,
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
  }: MessageTimelineProps,
  ref,
) {
  const internalScrollRef = React.useRef<HTMLDivElement>(null);
  const scrollContainerRef = externalScrollRef ?? internalScrollRef;
  const contentRef = React.useRef<HTMLDivElement>(null);
  const topSentinelRef = React.useRef<HTMLDivElement>(null);
  // The floating active-day header portals here — a non-scrolling sibling of
  // the scroll container — so it pins to a fixed offset and never drifts.
  const activeDayHeaderRef = React.useRef<HTMLDivElement>(null);

  // The convergence fallback for a windowed-out deep-link target. It's defined
  // below (it depends on the anchored-scroll result), so `useAnchoredScroll`
  // reads it through a ref via a stable wrapper — letting the hook stay
  // virtualizer-agnostic while the consumer owns the convergence machinery.
  const convergeToTargetRef = React.useRef<(messageId: string) => boolean>(
    () => false,
  );
  const convergeToTarget = React.useCallback((messageId: string) => {
    return convergeToTargetRef.current(messageId);
  }, []);

  // The mount bottom-pin driven through the virtualizer. Defined here as a
  // stable wrapper over a ref (assigned below, once the virtualizer/item count
  // are known) so `useAnchoredScroll` stays virtualizer-agnostic — same
  // indirection as `convergeToTarget`. Returns `true` once it issued the
  // index scroll; `false` (thread panel, no virtualizer) → the hook falls back
  // to its raw bottom pin.
  const pinToBottomByIndexRef = React.useRef<() => boolean>(() => false);
  const pinToBottomByIndex = React.useCallback(() => {
    return pinToBottomByIndexRef.current();
  }, []);

  // The virtualizer instance is owned by the child TimelineMessageList (which
  // mounts the VirtualizedList) and reported up here via a ref so the scroll
  // manager can drive index-model scroll paths. The virtualizer arrives via
  // VirtualizedList's onVirtualizer layout effect (child layout effects fire
  // before parent layout effects), so getVirtualizer() is live by the time
  // useAnchoredScroll's mount pin runs.
  const virtualizerRef = React.useRef<ListVirtualizer | null>(null);
  const handleVirtualizer = React.useCallback((instance: ListVirtualizer) => {
    virtualizerRef.current = instance;
  }, []);
  const getVirtualizer = React.useCallback(() => virtualizerRef.current, []);

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
  // Channel id travels with the deferred message snapshot. Without that guard, a
  // route change can paint the previous channel's deferred rows for a frame even
  // though the sidebar/header already moved to the new channel.
  const liveSnapshot = React.useMemo<TimelineSnapshot>(
    () => ({ channelId: channelId ?? null, messages }),
    [channelId, messages],
  );
  const deferredSnapshot = React.useDeferredValue(
    liveSnapshot,
    EMPTY_TIMELINE_SNAPSHOT,
  );
  const deferredMessages = deferredSnapshot.messages;
  // The flattened item stream mirrors what TimelineMessageList renders: use the
  // hoisted mainEntries when the deferred snapshot is current (same identity as
  // the live messages), fall back to building entries from deferredMessages when
  // the deferred value is stale. This keeps the scroll manager's index map in
  // sync with what's actually painted without a state-update round-trip.
  const deferredEntries = React.useMemo(
    () =>
      (deferredMessages === messages ? mainEntries : undefined) ??
      buildMainTimelineEntries(deferredMessages),
    [mainEntries, deferredMessages, messages],
  );
  const timelineItems = React.useMemo(
    () => buildTimelineItems(deferredEntries, firstUnreadMessageId),
    [deferredEntries, firstUnreadMessageId],
  );
  const virtualizerOption = React.useMemo(
    () =>
      timelineItems.items.length > 0
        ? {
            getVirtualizer,
            indexByMessageId: timelineItems.indexByMessageId,
            itemCount: timelineItems.items.length,
            liveMessageCount: messages.length,
          }
        : null,
    [getVirtualizer, timelineItems, messages.length],
  );
  const isDeferredSnapshotStale = isDeferredTimelineSnapshotStale({
    deferredSnapshot,
    liveSnapshot,
  });
  const isRenderPending = deferredSnapshot !== liveSnapshot;
  const scrollRestorationId = targetMessageId
    ? `message-timeline:${channelId ?? "none"}:target:${targetMessageId}`
    : `message-timeline:${channelId ?? "none"}`;
  // Keep the scroll node's DOM lifetime scoped to a channel. TanStack Router's
  // scroll-restoration listener runs outside React and may write a saved
  // scrollTop into the current scroll element during navigation; reusing the
  // same node across channel routes can leave the newly-loaded message list
  // painted at a stale offset until the user's next scroll event forces layout.
  const scrollContainerDomKey = channelId ?? "none";

  const timelineBodySurface = selectTimelineBodySurface({
    deferredCount: deferredMessages.length,
    isLoading: isLoading || isDeferredSnapshotStale,
    liveCount: messages.length,
  });
  const showTimelineSkeleton = timelineBodySurface === "skeleton";

  const {
    highlightedMessageId,
    isAtBottom,
    newMessageCount,
    onScroll,
    restoreScrollPosition,
    scrollToBottom,
    scrollToBottomOnNextUpdate,
    scrollToMessage,
    setLoadOlderRestoreInFlight,
    getAnchorIsAtBottom,
  } = useAnchoredScroll({
    channelId,
    contentRef,
    convergeToTarget,
    isLoading: showTimelineSkeleton,
    messages: deferredMessages,
    onTargetReached,
    pinToBottomByIndex,
    scrollContainerRef,
    targetMessageId,
  });

  const timelineIntroSurface = selectTimelineIntroSurface({
    hasChannelIntro: channelIntro !== null && directMessageIntro === null,
    hasDirectMessageIntro: directMessageIntro !== null,
    hasReachedChannelStart:
      !isRenderedTimelineBehindHistoryPrepend(deferredMessages, messages) &&
      (messages.length === 0 || (!hasOlderMessages && !isFetchingOlder)),
    isSkeletonVisible: showTimelineSkeleton,
  });
  const showDirectMessageIntro =
    timelineIntroSurface === "direct-message-intro";
  const showChannelIntro = timelineIntroSurface === "channel-intro";
  const activeDirectMessageIntro = showDirectMessageIntro
    ? directMessageIntro
    : null;
  const activeChannelIntro = showChannelIntro ? channelIntro : null;
  const showIntro = showDirectMessageIntro || showChannelIntro;
  const showGenericEmpty = timelineBodySurface === "empty" && !showIntro;
  const showMessageList = timelineBodySurface === "list";

  React.useImperativeHandle(
    ref,
    () => ({
      scrollToBottomOnNextUpdate,
    }),
    [scrollToBottomOnNextUpdate],
  );

  // Role 3 — jump-to-message into windowed-out history. The DOM-based
  // `scrollToMessage` no-ops when the target row isn't mounted (virtualized
  // out), so when it fails and the timeline is virtualized we drive the
  // convergence adapter: it scrolls the virtualizer to the target index,
  // re-aiming each frame as rows mount and measure, then on settle the row is
  // in the DOM and `scrollToMessage` centers + highlights it. When there's no
  // virtualizer (e.g. the thread panel), there's nothing to converge — the DOM
  // path is the whole story and a missing row simply isn't reachable.
  const { scrollToMessage: convergeToMessage, cancel: cancelConvergence } =
    useConvergentScrollToMessage(getVirtualizer, {
      indexByMessageId: timelineItems.indexByMessageId,
      align: "center",
      onConverged: (messageId) => {
        scrollToMessage(messageId, { highlight: true });
        onTargetReached?.(messageId);
      },
      onAbandoned: (messageId) => onTargetReached?.(messageId),
    });
  const jumpToMessage = React.useCallback(
    (messageId: string, options?: { behavior?: ScrollBehavior }) => {
      if (scrollToMessage(messageId, { highlight: true, ...options })) {
        return;
      }
      if (virtualizerOption) {
        convergeToMessage(messageId);
      }
    },
    [convergeToMessage, scrollToMessage, virtualizerOption],
  );
  // Feed the windowed-out deep-link fallback back into `useAnchoredScroll`,
  // which calls it when a target row isn't in the DOM. Gated on the virtualizer
  // so the thread panel (no virtualizer) never converges. Assigned in an effect
  // because `useAnchoredScroll` reads it asynchronously from a post-mount effect.
  React.useEffect(() => {
    convergeToTargetRef.current = virtualizerOption
      ? convergeToMessage
      : () => false;
  }, [convergeToMessage, virtualizerOption]);
  // Drive the mount bottom-pin through the virtualizer when one is present.
  // Assigned during render (not an effect) because `useAnchoredScroll`'s
  // mount pin runs in a layout effect on the same commit — a passive effect
  // would assign too late. `virtualizerOption` is derived from `timelineItems`
  // via useMemo (no state update, no extra render), so it is non-null from the
  // first deferred commit that carries real messages. `getVirtualizer()` is
  // live by pin time because VirtualizedList publishes the instance in a
  // layout effect, and child layout effects fire before parent layout effects.
  pinToBottomByIndexRef.current = virtualizerOption
    ? () => {
        const virtualizer = getVirtualizer();
        const lastIndex = virtualizerOption.itemCount - 1;
        if (!virtualizer || lastIndex < 0) return false;
        virtualizer.scrollToIndex(lastIndex, { align: "end" });
        return true;
      }
    : () => false;
  // Abandon any in-flight convergence on channel switch so a stale loop can't
  // hijack the new channel's scroll position.
  // biome-ignore lint/correctness/useExhaustiveDependencies: cancel on channel switch only
  React.useEffect(() => cancelConvergence, [channelId, cancelConvergence]);

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
    !showTimelineSkeleton;
  if (showUnreadPill) hasShownPillRef.current = true;
  const handleJumpToOldestUnread = React.useCallback(() => {
    setIsUnreadPillDismissed(true);
    if (firstUnreadMessageId) {
      jumpToMessage(firstUnreadMessageId);
    }
  }, [firstUnreadMessageId, jumpToMessage]);

  // Scroll to the active search match when it changes. `jumpToMessage` updates
  // the scroll anchor (so the post-commit restore won't yank the view back off
  // the match) and, when virtualized, converges on the target through the index
  // model — the row may be windowed out of the DOM.
  const prevSearchActiveRef = React.useRef<string | null>(null);
  React.useEffect(() => {
    if (showTimelineSkeleton) return;
    if (
      !searchActiveMessageId ||
      searchActiveMessageId === prevSearchActiveRef.current
    ) {
      prevSearchActiveRef.current = searchActiveMessageId;
      return;
    }
    prevSearchActiveRef.current = searchActiveMessageId;
    jumpToMessage(searchActiveMessageId, { behavior: "smooth" });
  }, [jumpToMessage, searchActiveMessageId, showTimelineSkeleton]);

  useLoadOlderOnScroll({
    fetchOlder,
    hasOlderMessages,
    isLoading: showTimelineSkeleton,
    restoreScrollPosition,
    setLoadOlderRestoreInFlight,
    getAnchorIsAtBottom,
    scrollContainerRef,
    sentinelRef: topSentinelRef,
    virtualizer: virtualizerOption,
  });

  const timelineSkeletonRows = useTimelineSkeletonRows({
    channelId,
    isLoading: showTimelineSkeleton,
    messages: showTimelineSkeleton ? EMPTY_MESSAGES : deferredMessages,
  });

  return (
    <TooltipProvider delayDuration={200}>
      <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        {/* Non-scrolling overlay anchored to the outer (non-scrolling) box: the
            floating active-day header portals in here, so it pins to a fixed
            offset and cannot drift as older history prepends. Sits below the
            unread pill / fetch spinner (z-20) in the stack. */}
        <div
          aria-hidden
          className={cn(
            "pointer-events-none absolute inset-x-0 z-[6] flex translate-y-3 justify-center px-4",
            channelChrome.top,
          )}
          ref={activeDayHeaderRef}
        />
        {showUnreadPill ? (
          <div
            className={cn(
              "pointer-events-none absolute inset-x-0 z-20 flex translate-y-3 justify-center px-4",
              channelChrome.top,
            )}
          >
            <UnreadPill
              direction="up"
              label={unreadCountLabel(unreadCount)}
              onClick={handleJumpToOldestUnread}
              testId="message-unread-pill"
            />
          </div>
        ) : null}
        {/* `isFetchingOlder` clears on fetch resolve, but rows paint a frame
            later off the deferred snapshot — keep the spinner up until then. */}
        {isFetchingOlder ||
        isRenderedTimelineBehindHistoryPrepend(deferredMessages, messages) ? (
          <div
            className={cn(
              "pointer-events-none absolute inset-x-0 z-20 flex translate-y-3 justify-center px-4",
              channelChrome.top,
            )}
            data-testid="message-timeline-fetching-older"
          >
            <span className="flex items-center rounded-full bg-background/80 p-1.5 shadow-sm ring-1 ring-border/40 backdrop-blur-sm">
              <Spinner className="h-4 w-4 border-2 text-muted-foreground" />
            </span>
          </div>
        ) : null}
        <div
          className={cn(
            "absolute inset-0 overflow-y-auto overflow-x-hidden overscroll-none px-2 pt-1 [overflow-anchor:none]",
            hasComposerOverlay ? "pb-24" : "pb-4",
          )}
          data-scroll-restoration-id={scrollRestorationId}
          data-testid="message-timeline"
          key={scrollContainerDomKey}
          onScroll={onScroll}
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

            {/* Fixed-height slot: an always-mounted height keeps the virtual
                spacer's offset stable across the load-older fetch toggle, so
                `scrollMargin` doesn't shift mid-fetch and yank the restore. The
                visible fetch spinner lives in the absolute overlay above, which
                does not occupy inline flow. */}
            <div aria-hidden className="h-8" />

            <div
              className={cn(
                "flex min-h-[18rem] min-w-0 flex-col gap-2",
                (showIntro || showGenericEmpty) && "min-h-full",
                showMessageList && !showIntro && "mt-auto",
              )}
            >
              {showTimelineSkeleton ? (
                <TimelineSkeleton rows={timelineSkeletonRows} />
              ) : null}
              {activeDirectMessageIntro ? (
                <div
                  className="mt-auto flex w-full flex-col items-start px-3 py-2 text-left"
                  data-testid="message-dm-intro"
                >
                  <DirectMessageIntroAvatarStack
                    participants={activeDirectMessageIntro.participants}
                  />
                  <p className="mt-4 max-w-full truncate text-xl font-semibold leading-7 tracking-tight text-foreground">
                    {activeDirectMessageIntro.displayName}
                  </p>
                  <p className="mt-1 max-w-full truncate whitespace-nowrap text-sm leading-5 text-muted-foreground">
                    This is the beginning of your direct message with{" "}
                    <span className="font-medium text-foreground">
                      {activeDirectMessageIntro.displayName}
                    </span>
                    .
                  </p>
                </div>
              ) : null}

              {activeChannelIntro ? (
                <div
                  className="mt-auto flex w-full max-w-2xl flex-col items-start px-3 py-2 text-left"
                  data-testid="message-channel-intro"
                >
                  <div
                    className="flex h-[60px] w-[60px] items-center justify-center rounded-2xl border border-border/70 bg-muted/40 text-muted-foreground"
                    data-testid="message-channel-intro-icon"
                  >
                    {activeChannelIntro.icon ?? (
                      <Hash aria-hidden className="h-7 w-7" />
                    )}
                  </div>
                  <p className="mt-4 max-w-full truncate text-xl font-semibold leading-7 tracking-tight text-foreground">
                    #{activeChannelIntro.channelName}
                  </p>
                  <p className="mt-1 max-w-full text-sm leading-5 text-muted-foreground">
                    This is the beginning of the{" "}
                    <span className="font-medium text-foreground">
                      {activeChannelIntro.channelKindLabel}
                    </span>
                    .
                  </p>
                  {activeChannelIntro.description ? (
                    <p className="mt-2 max-w-xl text-sm leading-5 text-muted-foreground">
                      {activeChannelIntro.description}
                    </p>
                  ) : null}
                  {activeChannelIntro.actions?.length ? (
                    <div className="mt-4 flex max-w-full flex-nowrap gap-3 overflow-x-auto pb-1">
                      {activeChannelIntro.actions.map((action) => {
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
                                  : "h-10 w-10 [&_svg]:h-4 [&_svg]:w-4",
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
                  className={cn("flex flex-col gap-2", !showIntro && "mt-auto")}
                  data-render-pending={isRenderPending ? "true" : undefined}
                >
                  <TimelineMessageList
                    key={scrollContainerDomKey}
                    agentPubkeys={agentPubkeys}
                    channelId={channelId}
                    channelName={channelName}
                    channelType={channelType}
                    currentPubkey={currentPubkey}
                    firstUnreadMessageId={firstUnreadMessageId}
                    followThreadById={followThreadById}
                    highlightedMessageId={highlightedMessageId}
                    isFollowingThreadById={isFollowingThreadById}
                    isMessageUnreadById={isMessageUnreadById}
                    messageFooters={messageFooters}
                    mainEntries={
                      deferredMessages === messages ? mainEntries : undefined
                    }
                    messages={deferredMessages}
                    onDelete={onDelete}
                    onEdit={onEdit}
                    onMarkUnread={onMarkUnread}
                    onMarkRead={onMarkRead}
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
                    scrollContainerRef={scrollContainerRef}
                    headerOverlayRef={activeDayHeaderRef}
                    onVirtualizer={handleVirtualizer}
                  />
                </div>
              ) : null}
            </div>
          </div>
        </div>

        {!isAtBottom ? (
          <div
            className={cn(
              "pointer-events-none absolute inset-x-0 z-20 flex justify-center px-4",
              hasComposerOverlay ? "bottom-36" : "bottom-4",
            )}
          >
            <UnreadPill
              direction="down"
              label={
                newMessageCount > 0
                  ? unreadCountLabel(newMessageCount)
                  : "Jump to latest"
              }
              onClick={() => {
                scrollToBottom("smooth");
              }}
              testId="message-scroll-to-latest"
            />
          </div>
        ) : null}
      </div>
    </TooltipProvider>
  );
});

export const MessageTimeline = React.memo(MessageTimelineBase);

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
