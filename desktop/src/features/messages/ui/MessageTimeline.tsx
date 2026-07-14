import * as React from "react";
import { Hash } from "lucide-react";

import {
  isDeferredTimelineSnapshotStale,
  isRenderedTimelineBehindHistoryPrepend,
  selectTimelineBodySurface,
  selectTimelineIntroSurface,
} from "@/features/messages/lib/timelineSnapshot";
import { preloadTimelineImages } from "@/features/messages/lib/timelineImagePreload";
import type { TimelineMessage } from "@/features/messages/types";
import type { MainTimelineEntry } from "@/features/messages/lib/threadPanel";
import type { ChannelWindowThreadSummary } from "@/features/messages/lib/channelWindowStore";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import type { ChannelType } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { channelChrome } from "@/shared/layout/chromeLayout";
import { Spinner } from "@/shared/ui/spinner";
import { TooltipProvider } from "@/shared/ui/tooltip";
import { UnreadPill, unreadCountLabel } from "@/shared/ui/UnreadPill";
import { TimelineSkeleton, useTimelineSkeletonRows } from "./TimelineSkeleton";
import { TimelineMessageList } from "./TimelineMessageList";
import type { TimelineVirtualizerApi } from "./TimelineMessageList";
import { useAnchoredScroll } from "./useAnchoredScroll";
import { useLoadOlderOnScroll } from "./useLoadOlderOnScroll";
import { useBufferedTimelineMessages } from "./useBufferedTimelineMessages";
import {
  DirectMessageIntroAvatarStack,
  type DirectMessageIntroParticipant,
} from "./DirectMessageIntroAvatarStack";
import { useSettleGatedPrependMessages } from "./useSettleGatedPrependMessages";

export type MessageTimelineHandle = {
  scrollToBottomOnNextUpdate: () => void;
};

type MessageTimelineProps = {
  channelId?: string | null;
  channelIntro?: ChannelIntro | null;
  channelName?: string;
  channelType?: ChannelType | null;
  huddleMemberPubkeys?: readonly string[];
  huddleMemberPubkeysPending?: boolean;
  messages: TimelineMessage[];
  mainEntries?: MainTimelineEntry[];
  /** Relay thread summaries (root id → summary) for the deferred-pass entry
   *  fallback, so badge rows survive while a scrollback page commits. */
  threadSummaries?: ReadonlyMap<string, ChannelWindowThreadSummary>;
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
  /**
   * True when the loaded window provably starts at the channel's beginning
   * (a resolved tail page with `hasMore: false`) — NOT merely the absence of
   * a paging signal. Gates the oldest loaded day's divider.
   */
  historyExhausted?: boolean;
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
  splitThreadPanelOpen?: boolean;
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

type TimelineSnapshot = {
  channelId: string | null;
  messages: TimelineMessage[];
  /**
   * History-exhaustion proof captured with the SAME rows it was derived from.
   * The oldest-day divider may only exist when this is true, and rows and
   * proof must travel every transport stage (deferral, buffering, settle
   * gating) as one value: delivering a fresh proof on the urgent render path
   * while the rows ride the deferred path lets an intermediate commit mint a
   * divider against the previous, partially-loaded oldest day — which breaks
   * Virtua's exact-suffix shift admission when the withheld same-day rows
   * finally land (the pass-1 tear, ledgered 2026-07-11).
   */
  historyExhausted: boolean;
};

const EMPTY_TIMELINE_SNAPSHOT: TimelineSnapshot = {
  channelId: null,
  messages: EMPTY_MESSAGES,
  historyExhausted: false,
};

const MessageTimelineBase = React.forwardRef<
  MessageTimelineHandle,
  MessageTimelineProps
>(function MessageTimeline(
  {
    channelId,
    channelIntro = null,
    directMessageIntro = null,
    messages,
    mainEntries,
    threadSummaries,
    isLoading = false,
    emptyTitle = "No messages yet",
    emptyDescription = "Send the first message to start the thread.",
    currentPubkey,
    fetchOlder,
    hasComposerOverlay = true,
    hasOlderMessages = true,
    historyExhausted = false,
    isFetchingOlder = false,
    followThreadById,
    huddleMemberPubkeys,
    huddleMemberPubkeysPending = false,
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
    splitThreadPanelOpen = false,
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
  const [virtualizerScrollParent, setVirtualizerScrollParent] =
    React.useState<HTMLDivElement | null>(null);
  const [virtualizerRenderVersion, bumpVirtualizerRenderVersion] =
    React.useReducer((version: number) => version + 1, 0);
  const [timelineVirtualizerApi, setTimelineVirtualizerApi] =
    React.useState<TimelineVirtualizerApi | null>(null);
  const useTimelineVirtualizer = true;
  const activeScrollContainerRef = React.useMemo(
    () => ({
      get current() {
        return virtualizerScrollParent ?? scrollContainerRef.current;
      },
    }),
    [scrollContainerRef, virtualizerScrollParent],
  );

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
    () => ({ channelId: channelId ?? null, messages, historyExhausted }),
    [channelId, historyExhausted, messages],
  );
  const deferredSnapshot = React.useDeferredValue(
    liveSnapshot,
    EMPTY_TIMELINE_SNAPSHOT,
  );
  const deferredMessages = deferredSnapshot.messages;
  const imagePreloadStateRef = React.useRef({
    activeImages: new Set<HTMLImageElement>(),
    requestedUrls: new Set<string>(),
  });
  React.useEffect(() => {
    preloadTimelineImages(messages, imagePreloadStateRef.current);
  }, [messages]);
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

  React.useLayoutEffect(() => {
    // Re-read after `scrollContainerDomKey` swaps the keyed scroll DOM node.
    void scrollContainerDomKey;
    if (!useTimelineVirtualizer) {
      setVirtualizerScrollParent(scrollContainerRef.current);
    }
    setTimelineVirtualizerApi(null);
  }, [scrollContainerRef, scrollContainerDomKey]);

  const timelineBodySurface = selectTimelineBodySurface({
    deferredCount: deferredMessages.length,
    isLoading: isLoading || isDeferredSnapshotStale,
    liveCount: messages.length,
  });
  const showTimelineSkeleton = timelineBodySurface === "skeleton";
  const [isSemanticallyAtBottom, setIsSemanticallyAtBottom] =
    React.useState(true);
  // biome-ignore lint/correctness/useExhaustiveDependencies: reset semantic tail state when the active channel changes
  React.useEffect(() => {
    setIsSemanticallyAtBottom(true);
  }, [channelId]);
  // Zulip-style data semantics: once the reader leaves the bottom, keep the
  // virtualizer's logical tail frozen. Live arrivals accumulate behind the
  // "new messages" affordance instead of changing Virtua's item model under
  // the reading position. Prepends still flow through immediately and Virtua's
  // `shift` transaction preserves the stable keyed row.
  const bufferedTimeline = useBufferedTimelineMessages({
    channelId,
    isAtBottom:
      isSemanticallyAtBottom ||
      targetMessageId !== null ||
      searchActiveMessageId !== null,
    messages: deferredMessages,
  });
  // Hold older-page render commits until the scroller is at rest: WKWebView
  // can drop scrollTop compensation writes during live trackpad momentum.
  // Full rationale in useSettleGatedPrependMessages.
  //
  // The history-exhaustion proof rides through this gate as snapshot metadata
  // (`meta`), so while a prepend is withheld the rendered rows keep the proof
  // they were projected with. The buffering stage above cannot split the pair:
  // it only freezes the TAIL (live arrivals) and passes history prepends
  // through unchanged, so the oldest rows the proof speaks about are exactly
  // the deferred snapshot's oldest rows.
  const {
    messages: renderedMessages,
    meta: renderedHistoryExhausted,
    isHoldingPrepend,
  } = useSettleGatedPrependMessages({
    channelId,
    messages: bufferedTimeline.messages,
    meta: deferredSnapshot.historyExhausted,
    scrollElementRef: activeScrollContainerRef,
  });

  const {
    highlightedMessageId,
    isAtBottom,
    newMessageCount,
    onScroll,
    scrollToBottom,
    scrollToBottomOnNextUpdate,
    scrollToMessage,
    onVirtualizerAtBottomStateChange,
  } = useAnchoredScroll({
    channelId,
    contentRef,
    isLoading: showTimelineSkeleton,
    messages: renderedMessages,
    onTargetReached,
    scrollContainerRef: activeScrollContainerRef,
    splitPanelOpen: splitThreadPanelOpen,
    targetMessageId,
    virtualScrollToMessage: timelineVirtualizerApi?.scrollToMessage,
    virtualScrollToBottom: timelineVirtualizerApi?.scrollToBottom,
    virtualSettleAtBottom: timelineVirtualizerApi?.settleAtBottom,
    virtualizerOwnsPrependAnchoring: useTimelineVirtualizer,
    virtualizerRenderVersion,
  });

  const hasConfirmedVirtualizerBottomRef = React.useRef(false);
  const bottomConfirmationChannelRef = React.useRef(channelId);
  if (bottomConfirmationChannelRef.current !== channelId) {
    bottomConfirmationChannelRef.current = channelId;
    hasConfirmedVirtualizerBottomRef.current = false;
  }
  const suppressNextSemanticBottomRef = React.useRef(false);
  const semanticAtBottomRef = React.useRef(isSemanticallyAtBottom);
  semanticAtBottomRef.current = isSemanticallyAtBottom;
  const semanticBottomRafRef = React.useRef<number | null>(null);
  const queueSemanticBottom = React.useCallback((atBottom: boolean) => {
    semanticAtBottomRef.current = atBottom;
    if (semanticBottomRafRef.current !== null) {
      window.cancelAnimationFrame(semanticBottomRafRef.current);
    }
    semanticBottomRafRef.current = window.requestAnimationFrame(() => {
      semanticBottomRafRef.current = null;
      setIsSemanticallyAtBottom(atBottom);
    });
  }, []);
  React.useEffect(
    () => () => {
      if (semanticBottomRafRef.current !== null) {
        window.cancelAnimationFrame(semanticBottomRafRef.current);
      }
    },
    [],
  );
  const handleVirtualizerAtBottomStateChange = React.useCallback(
    (atBottom: boolean) => {
      // Virtua can emit an intermediate non-bottom offset while its initial
      // scroll-to-end is still converging. Do not turn that mount transient
      // into a semantic dataset freeze: wait until this channel has reached a
      // confirmed bottom once, then track genuine bottom -> history movement.
      if (atBottom) {
        hasConfirmedVirtualizerBottomRef.current = true;
        onVirtualizerAtBottomStateChange(true);
        if (suppressNextSemanticBottomRef.current) {
          // Freezing the tail shortens Virtua's model and can itself make the
          // current offset report "at bottom". That synthetic transition must
          // not immediately release the snapshot and oscillate forever.
          suppressNextSemanticBottomRef.current = false;
        } else if (!semanticAtBottomRef.current) {
          queueSemanticBottom(true);
        }
      } else if (hasConfirmedVirtualizerBottomRef.current) {
        onVirtualizerAtBottomStateChange(false);
        if (semanticAtBottomRef.current) {
          suppressNextSemanticBottomRef.current = true;
          queueSemanticBottom(false);
        }
      }
    },
    [onVirtualizerAtBottomStateChange, queueSemanticBottom],
  );

  const timelineIntroSurface = selectTimelineIntroSurface({
    hasChannelIntro: channelIntro !== null && directMessageIntro === null,
    hasDirectMessageIntro: directMessageIntro !== null,
    hasReachedChannelStart:
      !isRenderedTimelineBehindHistoryPrepend(deferredMessages, messages) &&
      !isHoldingPrepend &&
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
  const showIntro =
    activeDirectMessageIntro !== null || activeChannelIntro !== null;
  const showGenericEmpty = timelineBodySurface === "empty" && !showIntro;
  const showMessageList = timelineBodySurface === "list";

  const prepareForOwnMessage = React.useCallback(() => {
    // The user's own send is the deliberate Zulip exception: release buffered
    // output before arming the next-append bottom pin so the sent row can enter
    // Virtua's model and become the new physical floor.
    setIsSemanticallyAtBottom(true);
    scrollToBottomOnNextUpdate();
  }, [scrollToBottomOnNextUpdate]);

  React.useImperativeHandle(
    ref,
    () => ({
      scrollToBottomOnNextUpdate: prepareForOwnMessage,
    }),
    [prepareForOwnMessage],
  );

  // Jump-to-message is purely DOM-based now: all loaded rows are mounted, so
  // `scrollToMessage` always finds the target row. No virtualizer convergence.
  const jumpToMessage = React.useCallback(
    (messageId: string, options?: { behavior?: ScrollBehavior }) => {
      return scrollToMessage(messageId, { highlight: true, ...options });
    },
    [scrollToMessage],
  );

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
  const pendingSearchTargetRef = React.useRef<string | null>(null);
  React.useEffect(() => {
    if (showTimelineSkeleton) return;
    if (!searchActiveMessageId) {
      pendingSearchTargetRef.current = null;
    }
    if (
      !searchActiveMessageId ||
      searchActiveMessageId === prevSearchActiveRef.current
    ) {
      prevSearchActiveRef.current = searchActiveMessageId;
      return;
    }
    pendingSearchTargetRef.current = null;
    prevSearchActiveRef.current = searchActiveMessageId;
    if (!jumpToMessage(searchActiveMessageId, { behavior: "smooth" })) {
      pendingSearchTargetRef.current = searchActiveMessageId;
    }
  }, [jumpToMessage, searchActiveMessageId, showTimelineSkeleton]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: deferredMessages and virtualizerRenderVersion are intentional retry triggers — a search hit may be spliced into messages asynchronously, and in virtualized mode a phase-1 index jump only realizes the row; retry when the rendered range changes so the DOM-visible path can center and highlight it.
  React.useEffect(() => {
    const target = pendingSearchTargetRef.current;
    if (!target || showTimelineSkeleton) return;
    if (
      useTimelineVirtualizer &&
      !activeScrollContainerRef.current?.querySelector(
        `[data-message-id="${CSS.escape(target)}"]`,
      )
    ) {
      // Phase 1: ask the virtualizer to realize the match's index. The retry effect
      // runs again on range change and the DOM-visible path does the actual
      // center + highlight once the row exists.
      void jumpToMessage(target, { behavior: "auto" });
      return;
    }
    if (jumpToMessage(target, { behavior: "auto" })) {
      pendingSearchTargetRef.current = null;
    }
  }, [
    deferredMessages,
    jumpToMessage,
    showTimelineSkeleton,
    virtualizerRenderVersion,
  ]);

  const loadOlderViaVirtualizer = React.useCallback((): boolean => {
    // Indexed find navigation can legitimately land near the current history
    // boundary. Do not mistake that programmatic jump for scrollback intent and
    // prepend underneath the active match.
    // A settle-gate hold means the reader is still parked at the OLD
    // boundary — don't stack more page fetches behind the held commit.
    if (
      searchActiveMessageId ||
      !fetchOlder ||
      isFetchingOlder ||
      isHoldingPrepend ||
      showTimelineSkeleton ||
      !hasOlderMessages
    ) {
      return false;
    }
    void fetchOlder();
    return true;
  }, [
    fetchOlder,
    hasOlderMessages,
    isFetchingOlder,
    isHoldingPrepend,
    searchActiveMessageId,
    showTimelineSkeleton,
  ]);

  useLoadOlderOnScroll({
    fetchOlder: useTimelineVirtualizer ? undefined : fetchOlder,
    hasOlderMessages,
    isLoading: showTimelineSkeleton,
    scrollContainerRef: activeScrollContainerRef,
    sentinelRef: topSentinelRef,
  });

  const timelineSkeletonRows = useTimelineSkeletonRows({
    channelId,
    isLoading: showTimelineSkeleton,
    messages: showTimelineSkeleton ? EMPTY_MESSAGES : deferredMessages,
  });

  const virtualizedLeadingContent = React.useMemo(
    () =>
      activeChannelIntro ? (
        <div
          className="flex w-full max-w-2xl flex-col items-start px-3 pb-4 pt-2 text-left"
          data-testid="message-channel-intro"
        >
          <div className="flex h-[60px] w-[60px] items-center justify-center rounded-2xl border border-border/70 bg-muted/40 text-muted-foreground">
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
              {activeChannelIntro.actions.map((action) => (
                <button
                  className={cn(
                    "flex shrink-0 border border-border/70 bg-background/70 text-left transition-colors hover:bg-muted/60 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring",
                    action.description
                      ? "h-56 w-[13.75rem] flex-col rounded-2xl p-4"
                      : "h-28 w-64 flex-col rounded-2xl p-4",
                  )}
                  data-testid={action.testId}
                  key={action.label}
                  onClick={action.onClick}
                  type="button"
                >
                  <span
                    className={cn(
                      "flex shrink-0 items-center justify-center rounded-full bg-muted/70 text-muted-foreground",
                      action.description
                        ? "h-12 w-12 [&_svg]:h-6 [&_svg]:w-6"
                        : "h-10 w-10 [&_svg]:h-4 [&_svg]:w-4",
                    )}
                  >
                    {action.icon}
                  </span>
                  <span className="mt-auto min-w-0">
                    <span className="block whitespace-normal break-words text-base font-medium leading-6 text-foreground">
                      {action.label}
                    </span>
                    {action.description ? (
                      <span className="mt-1 block whitespace-normal break-words text-sm leading-5 text-muted-foreground">
                        {action.description}
                      </span>
                    ) : null}
                  </span>
                </button>
              ))}
            </div>
          ) : null}
        </div>
      ) : activeDirectMessageIntro ? (
        <div
          className="mb-2 flex w-full flex-col items-start px-3 pb-2 pt-2 text-left"
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
      ) : null,
    [activeChannelIntro, activeDirectMessageIntro],
  );

  const handleVirtualizerRangeChanged = React.useCallback(() => {
    bumpVirtualizerRenderVersion();
  }, []);

  const timelineList = showMessageList ? (
    <TimelineMessageList
      key={scrollContainerDomKey}
      channelId={channelId}
      channelName={channelName}
      channelType={channelType}
      currentPubkey={currentPubkey}
      firstUnreadMessageId={firstUnreadMessageId}
      followThreadById={followThreadById}
      highlightedMessageId={highlightedMessageId}
      huddleMemberPubkeys={huddleMemberPubkeys}
      huddleMemberPubkeysPending={huddleMemberPubkeysPending}
      isFollowingThreadById={isFollowingThreadById}
      isMessageUnreadById={isMessageUnreadById}
      messageFooters={messageFooters}
      mainEntries={renderedMessages === messages ? mainEntries : undefined}
      leadingContent={virtualizedLeadingContent}
      historyExhausted={renderedHistoryExhausted}
      threadSummaries={threadSummaries}
      messages={renderedMessages}
      onDelete={onDelete}
      onEdit={onEdit}
      onMarkUnread={onMarkUnread}
      onMarkRead={onMarkRead}
      onReply={onReply}
      isSendingVideoReviewComment={isSendingVideoReviewComment}
      onSendVideoReviewComment={onSendVideoReviewComment}
      onStartReached={loadOlderViaVirtualizer}
      onToggleReaction={onToggleReaction}
      onVirtualizerApiChange={setTimelineVirtualizerApi}
      onVirtualizerRangeChanged={handleVirtualizerRangeChanged}
      onVirtualizerScrollerChange={setVirtualizerScrollParent}
      onAtBottomStateChange={handleVirtualizerAtBottomStateChange}
      personaLookup={personaLookup}
      profiles={profiles}
      searchActiveMessageId={searchActiveMessageId}
      searchMatchingMessageIds={searchMatchingMessageIds}
      searchQuery={searchQuery}
      useVirtualizer={useTimelineVirtualizer}
      threadUnreadCounts={threadUnreadCounts}
      unfollowThreadById={unfollowThreadById}
    />
  ) : null;

  return (
    <TooltipProvider delayDuration={200}>
      <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        {showUnreadPill ? (
          <div
            className={cn(
              "pointer-events-none absolute inset-x-0 z-30 flex translate-y-3 justify-center px-4",
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
            later (deferred snapshot / settle-gate hold) — keep the spinner up
            until the page actually renders. */}
        {isFetchingOlder ||
        isHoldingPrepend ||
        isRenderedTimelineBehindHistoryPrepend(deferredMessages, messages) ? (
          <div
            className={cn(
              "pointer-events-none absolute inset-x-0 z-30 flex translate-y-3 justify-center px-4",
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
            "absolute inset-0 overflow-hidden",
            (!useTimelineVirtualizer || !showMessageList) &&
              cn(
                "overflow-y-auto overflow-x-hidden overscroll-contain px-2 pt-1",
                hasComposerOverlay
                  ? "pb-[var(--composer-overlay-height,6rem)]"
                  : "pb-4",
              ),
          )}
          data-buzz-conversation-scroll={
            useTimelineVirtualizer && showMessageList ? undefined : "true"
          }
          data-scroll-restoration-id={scrollRestorationId}
          data-testid={
            useTimelineVirtualizer && showMessageList
              ? undefined
              : "message-timeline"
          }
          key={scrollContainerDomKey}
          onScroll={useTimelineVirtualizer ? undefined : onScroll}
          ref={scrollContainerRef}
        >
          {useTimelineVirtualizer && timelineList ? (
            <div
              className="h-full min-h-0 w-full"
              data-render-pending={isRenderPending ? "true" : undefined}
            >
              {timelineList}
            </div>
          ) : (
            <div
              className={cn(
                "flex w-full flex-col gap-2",
                channelChrome.contentPadding,
                (showIntro || showGenericEmpty || showMessageList) &&
                  "min-h-full",
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
                  useTimelineVirtualizer && "min-h-0 flex-1",
                  (showIntro || showGenericEmpty) && "min-h-full",
                  showMessageList &&
                    !showIntro &&
                    !useTimelineVirtualizer &&
                    "mt-auto",
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
                                  : "h-28 w-64 flex-col rounded-2xl p-4",
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
                    className="mt-auto rounded-2xl border border-dashed border-border/80 bg-card/70 px-6 py-10 text-center shadow-xs"
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
                      !showIntro && !useTimelineVirtualizer && "mt-auto",
                      useTimelineVirtualizer && "min-h-0 flex-1",
                    )}
                    data-render-pending={isRenderPending ? "true" : undefined}
                  >
                    {timelineList}
                  </div>
                ) : null}
              </div>
            </div>
          )}
        </div>

        {!isAtBottom ? (
          <div
            className={cn(
              "pointer-events-none absolute inset-x-0 z-50 flex justify-center px-4",
              hasComposerOverlay ? "bottom-36" : "bottom-4",
            )}
          >
            <UnreadPill
              direction="down"
              label={
                bufferedTimeline.pendingCount > 0
                  ? unreadCountLabel(bufferedTimeline.pendingCount)
                  : newMessageCount > 0
                    ? unreadCountLabel(newMessageCount)
                    : "Jump to latest"
              }
              onClick={() => {
                setIsSemanticallyAtBottom(true);
                window.requestAnimationFrame(() => scrollToBottom("auto"));
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
