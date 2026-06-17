import * as React from "react";
import { ArrowDown, Hash } from "lucide-react";
import { useVirtualizer } from "@tanstack/react-virtual";

import { getDmParticipantPreview } from "@/features/channels/lib/dmParticipantDisplay";
import { buildVirtualTimelineRows } from "@/features/messages/lib/buildVirtualTimelineRows";
import { buildMainTimelineEntries } from "@/features/messages/lib/threadPanel";
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
import { TimelineDebugOverlay } from "./TimelineDebugOverlay";
import {
  renderTimelineEntry,
  type TimelineEntryRenderContext,
} from "./timelineEntryRender";
import { useLoadOlderOnScroll } from "./useLoadOlderOnScroll";
import { useVideoReviewContextById } from "./useVideoReviewContextById";
import { useVirtualScrollMargin } from "./useVirtualScrollMargin";
import { useVirtualTimelineScroll } from "./useVirtualTimelineScroll";
import { VirtualizedTimelineList } from "./VirtualizedTimelineList";

// Initial row-size guesses only — `measureElement` corrects each row to its
// real height after first paint, so variable-height messages and dividers need
// no fixed-height assumption.
const ESTIMATED_MESSAGE_HEIGHT = 64;
const ESTIMATED_DIVIDER_HEIGHT = 32;
const ESTIMATED_INTRO_HEIGHT = 320;
const VIRTUAL_OVERSCAN = 8;

// Fallback escape hatch for find-in-page: when find is open, optionally bypass
// virtualization and render every row so native browser cmd+F can see all
// matches. OFF by default — the in-app find path (scroll-to-row via
// `findVirtualRowIndexForMessage`) is the default and keeps the perf win.
// Flip via the `renderAllWhileSearching` prop only if QA finds a gap.
const RENDER_ALL_WHILE_SEARCHING_DEFAULT = false;

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
  /** Escape hatch: render all rows while find-in-page is open so native
   *  browser cmd+F can see every match. Defaults off (in-app find is default). */
  renderAllWhileSearching?: boolean;
  targetMessageId?: string | null;
  onTargetReached?: (messageId: string) => void;
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
  renderAllWhileSearching = RENDER_ALL_WHILE_SEARCHING_DEFAULT,
  targetMessageId = null,
  onTargetReached,
}: MessageTimelineProps) {
  const internalScrollRef = React.useRef<HTMLDivElement>(null);
  const scrollContainerRef = externalScrollRef ?? internalScrollRef;
  const topSentinelRef = React.useRef<HTMLDivElement>(null);
  // Wraps the virtualized list; its offset within the scroll container is the
  // virtualizer's `scrollMargin` (content above it: sentinel, spinner).
  const listOuterRef = React.useRef<HTMLDivElement>(null);

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

  // Filtered main-timeline entries + flat virtual rows, both off the SAME
  // deferred snapshot the rows render from (the no-tearing property from
  // Phase 1). `entryMessages` is what the virtual rows index into, so dropped
  // thread replies never desync the row→entry mapping.
  const entries = React.useMemo(
    () => buildMainTimelineEntries(deferredMessages),
    [deferredMessages],
  );
  const entryMessages = React.useMemo(
    () => entries.map((entry) => entry.message),
    [entries],
  );
  // The channel intro is the terminal header at the true top of history. Include
  // it as virtual row 0 only once the deferred list has settled AND we've reached
  // the genuine history boundary (no older pages remain).
  const introReady =
    !isLoading &&
    channelIntro !== null &&
    directMessageIntro === null &&
    !isRenderPending &&
    (deferredMessages.length === 0 || !hasOlderMessages);
  const rows = React.useMemo(
    () => buildVirtualTimelineRows(entryMessages, { includeIntro: introReady }),
    [entryMessages, introReady],
  );

  // When the render-all escape hatch is enabled AND find is open, expand the
  // overscan to the whole list so native browser cmd+F can see every match.
  // Default path keeps the lean fixed overscan.
  const isSearchOpen = Boolean(searchQuery || searchActiveMessageId);
  const overscan =
    renderAllWhileSearching && isSearchOpen && rows.length > 0
      ? rows.length
      : VIRTUAL_OVERSCAN;

  // Offset of the virtualized list within the scroll container — content above
  // it (sentinel, "load older" spinner) lives in the SAME scrollable element,
  // so the virtualizer must know that offset or rows paint at the wrong
  // scrollTop (header/list sandwich + anchor drift on fill).
  const scrollMargin = useVirtualScrollMargin(
    scrollContainerRef,
    listOuterRef,
    [
      isLoading,
      isFetchingOlder,
      deferredMessages.length,
      directMessageIntro,
      rows.length,
    ],
  );

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollContainerRef.current,
    estimateSize: (index) => {
      const row = rows[index];
      if (!row) {
        return ESTIMATED_MESSAGE_HEIGHT;
      }
      if (row.kind === "day-divider") {
        return ESTIMATED_DIVIDER_HEIGHT;
      }
      if (row.kind === "intro") {
        return ESTIMATED_INTRO_HEIGHT;
      }
      return ESTIMATED_MESSAGE_HEIGHT;
    },
    // Stable per-row identity. THIS is what lets a top-prepend (older page)
    // retain scroll position natively — surviving rows keep their key, so the
    // measurement cache survives and the virtualizer re-anchors itself. No
    // before/after scrollHeight delta math, no double-rAF correction.
    getItemKey: (index) => rows[index]?.key ?? index,
    overscan,
    // Account for the sentinel/spinner above the list inside the same scroll
    // container, so item offsets line up with where they actually paint.
    scrollMargin: scrollMargin.value,
  });

  const {
    highlightedMessageId,
    isAtBottom,
    newMessageCount,
    scrollToBottom,
    syncScrollState,
  } = useVirtualTimelineScroll({
    channelId,
    isLoading,
    messages: entryMessages,
    rows,
    scrollContainerRef,
    virtualizer,
    // The init bottom pin must wait until the list's scroll margin is measured;
    // pinning against the pre-mount stale `0` lands `scrollMargin` px short of
    // true bottom and paints the rows out of place for a beat (the first-load
    // flash) before re-anchoring. `measured` gates that first pin.
    scrollMarginReady: scrollMargin.measured,
    targetMessageId,
    onTargetReached,
    searchActiveMessageId,
  });

  const videoReviewContextById = useVideoReviewContextById({
    channelId,
    channelName,
    channelType,
    isSendingVideoReviewComment,
    messages: deferredMessages,
    onSendVideoReviewComment,
    onToggleReaction,
    profiles,
  });

  const renderContext: TimelineEntryRenderContext = {
    agentPubkeys,
    channelId,
    channelType,
    currentPubkey,
    followThreadById,
    highlightedMessageId,
    isFollowingThreadById,
    messageFooters,
    onDelete,
    onEdit,
    onMarkUnread,
    onReply,
    personaLookup,
    onToggleReaction,
    profiles,
    searchActiveMessageId,
    searchMatchingMessageIds,
    searchQuery,
    unfollowThreadById,
    videoReviewContextById,
  };

  const renderEntry = React.useCallback(
    (entry: Parameters<typeof renderTimelineEntry>[0]) =>
      renderTimelineEntry(entry, renderContext),
    // renderContext is rebuilt every render from the same inputs; the entry
    // render reads its current values, so depending on the bundle directly
    // keeps the callback in sync without a stale closure.
    // biome-ignore lint/correctness/useExhaustiveDependencies: renderContext fields are the real deps
    [renderContext],
  );

  const renderChannelIntro = React.useCallback(() => {
    if (!channelIntro) {
      return null;
    }
    return <ChannelIntroHeader intro={channelIntro} />;
  }, [channelIntro]);

  // Pagination trigger only — the virtualizer holds scroll position on prepend
  // natively (stable keys), so there is no position-restore plumbing to pass.
  useLoadOlderOnScroll({
    fetchOlder,
    hasOlderMessages,
    isLoading,
    scrollContainerRef,
    sentinelRef: topSentinelRef,
  });

  const showDirectMessageIntro = !isLoading && directMessageIntro !== null;
  const showGenericEmpty =
    !isLoading &&
    deferredMessages.length === 0 &&
    directMessageIntro === null &&
    channelIntro === null;
  const showVirtualList =
    !isLoading && (deferredMessages.length > 0 || introReady);
  const useMinHeightFill =
    showDirectMessageIntro ||
    showGenericEmpty ||
    (introReady && deferredMessages.length === 0);
  const timelineSkeletonRows = useTimelineSkeletonRows({
    channelId,
    isLoading,
    messages,
  });

  return (
    <TooltipProvider delayDuration={200}>
      <div className="relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
        <div
          className={cn(
            "absolute inset-0 overflow-y-auto overflow-x-hidden overscroll-contain px-4 pt-1 [overflow-anchor:none] sm:px-6",
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
              useMinHeightFill && "min-h-full",
            )}
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
                useMinHeightFill && "min-h-full",
                showVirtualList && !showDirectMessageIntro && "mt-auto",
              )}
              contentClassName={cn(
                "flex flex-col gap-2",
                useMinHeightFill && "min-h-full",
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

              {showVirtualList ? (
                <div
                  className={cn(
                    "flex flex-col",
                    // While a deferred render is in flight the painted
                    // list lags the latest `messages`. Dim it slightly so the
                    // streaming-in feels intentional instead of frozen.
                    isRenderPending && "opacity-60 transition-opacity",
                  )}
                  data-render-pending={isRenderPending ? "true" : undefined}
                  ref={listOuterRef}
                >
                  <VirtualizedTimelineList
                    entries={entries}
                    renderEntry={renderEntry}
                    renderIntro={introReady ? renderChannelIntro : undefined}
                    rows={rows}
                    scrollMargin={scrollMargin.value}
                    virtualizer={virtualizer}
                  />
                </div>
              ) : null}
            </SkeletonReveal>
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
              className="pointer-events-auto h-7 min-h-7 gap-1.5 rounded-full border-border/50 bg-background/85 px-2.5 text-[11px] font-medium text-muted-foreground shadow-xs backdrop-blur-sm hover:bg-muted/70 hover:text-foreground [&_svg]:size-4"
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

        {showVirtualList ? (
          <TimelineDebugOverlay
            highlightedMessageId={highlightedMessageId}
            isAtBottom={isAtBottom}
            newMessageCount={newMessageCount}
            overscan={overscan}
            rows={rows}
            scrollContainerRef={scrollContainerRef}
            searchActiveMessageId={searchActiveMessageId}
            targetMessageId={targetMessageId}
            virtualizer={virtualizer}
          />
        ) : null}
      </div>
    </TooltipProvider>
  );
});

function ChannelIntroHeader({ intro }: { intro: ChannelIntro }) {
  return (
    <div
      className="mb-0.5 flex w-full max-w-2xl flex-col items-start px-3 py-2 text-left"
      data-testid="message-channel-intro"
    >
      <div
        className="flex h-[60px] w-[60px] items-center justify-center rounded-2xl border border-border/70 bg-muted/40 text-muted-foreground"
        data-testid="message-channel-intro-icon"
      >
        {intro.icon ?? <Hash aria-hidden className="h-7 w-7" />}
      </div>
      <p className="mt-4 max-w-full truncate text-xl font-semibold leading-7 tracking-tight text-foreground">
        #{intro.channelName}
      </p>
      <p className="mt-1 max-w-full text-sm leading-5 text-muted-foreground">
        This is the beginning of the{" "}
        <span className="font-medium text-foreground">
          {intro.channelKindLabel}
        </span>
        .
      </p>
      {intro.description ? (
        <p className="mt-2 max-w-xl text-sm leading-5 text-muted-foreground">
          {intro.description}
        </p>
      ) : null}
      {intro.actions?.length ? (
        <div className="mt-4 flex max-w-full flex-nowrap gap-3 overflow-x-auto pb-1">
          {intro.actions.map((action) => {
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
                    action.testId ? `${action.testId}-icon` : undefined
                  }
                >
                  {action.icon}
                </span>
                <span className="mt-auto min-w-0">
                  <span
                    className="block whitespace-normal break-words text-base font-medium leading-6 text-foreground"
                    data-testid={
                      action.testId ? `${action.testId}-title` : undefined
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
  );
}

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
