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
import { getThreadReference } from "@/features/messages/lib/threading";
import { MessageComposer } from "@/features/messages/ui/MessageComposer";
import { UpdateIndicator } from "@/features/settings/UpdateIndicator";
import type { Channel } from "@/shared/api/types";
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
  onOpenChannel: (channelId: string) => void;
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
  replies = [],
  channel,
  contextChannelName = null,
  currentPubkey,
  selectedEventId,
  latchedDefaultParentId = null,
  onBack,
  onDelete,
  onOpenChannel,
  onSendReply,
  onToggleReaction,
}: InboxDetailPaneProps) {
  const detailPaneRef = React.useRef<HTMLElement | null>(null);
  // Refs for the scroll container and its inner content div — used by the
  // post-center anchor hold to compensate for late content growth above the
  // selected message (reactions, channel-window merge, image decode).
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
  // Scroll key: changes only when the user switches to a different conversation
  // or selects a different event anchor (which triggers centering once). Live
  // message arrivals in the same conversation do NOT change this key.
  const selectedMessageScrollKey = React.useMemo(() => {
    if (!conversationId || !selectedEventId) {
      return null;
    }
    return `${conversationId}:${selectedEventId}`;
  }, [conversationId, selectedEventId]);

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

  // Deferred deliberate-selection centering.
  //
  // Bug fixed here: the old one-shot rAF fired on the click render, before
  // useInboxThreadContext had started its async fetch.  The fetch then prepended
  // older messages ABOVE the viewport, shifting it mid-thread.
  //
  // Fix: arm a pending-center ref on each new (conversationId, selectedEventId)
  // pair.  If isThreadContextLoading is already false when the rAF fires (no
  // fetch needed), execute immediately.  If loading starts before/as the rAF
  // fires, cancel the rAF and re-execute once loading transitions true → false.
  // User scroll before the center fires cancels it (never yank the reader).
  //
  // Effect-ordering note: on the click commit, InboxDetailPane effects run
  // before HomeView (child-first), so isThreadContextLoading is still false
  // at that instant — the "is not loading right now" guard alone is NOT
  // sufficient; we must observe the true → false transition instead.
  // The isLoading ref is kept up-to-date unconditionally so rAF callbacks
  // always read the current value (closures would capture stale renders).
  const pendingCenterKeyRef = React.useRef<string | null>(null);
  const userScrolledRef = React.useRef(false);
  const prevLoadingRef = React.useRef(isThreadContextLoading);
  const isLoadingRef = React.useRef(isThreadContextLoading);
  // Keep isLoadingRef current every render so rAF callbacks see the live value.
  isLoadingRef.current = isThreadContextLoading;

  // Post-center anchor hold.
  //
  // After the deliberate-selection center fires, reactions, channel-window
  // merges, and image decodes can add content ABOVE the selected message.
  // The browser's native scroll anchoring pins the *topmost visible row*, so
  // growth between that row and the selected message pushes the selected row
  // down without compensation — producing the "correct snap → brief pause →
  // jumps up 2-3 messages" symptom.
  //
  // Fix (mirrors useAnchoredScroll.ts:423-433): after the center fires,
  // hold the selected row's absolute position within the scroll container's
  // content (invariant under user scroll: scrollBy on same axis changes both
  // bcrect.top and scrollTop by the same amount, so the sum is stable).  On
  // every subsequent message-list commit (useLayoutEffect), re-measure and
  // compensate with scrollBy(0, drift) when content above the anchor has grown.
  // Release the hold on user interaction or selection-key change.
  //
  // Measurement: contentTop = bcrect.top + scrollTop - container.bcrect.top.
  // This is scroll-invariant: a user scroll ±D changes bcrect.top by ∓D and
  // scrollTop by ±D, leaving the sum unchanged.  Only content growth above the
  // anchor changes contentTop.
  const anchorHoldRef = React.useRef<{
    contentTop: number;
    key: string;
  } | null>(null);
  // The expected target of a programmatic write. The scroll listener consumes
  // only an event at this exact position; a no-op write leaves it unset, and a
  // later real scroll at a different position still releases the hold.
  const programmaticScrollTopRef = React.useRef<number | null>(null);
  const isWritingScrollRef = React.useRef(false);
  const selectedMessageScrollKeyRef = React.useRef(selectedMessageScrollKey);
  selectedMessageScrollKeyRef.current = selectedMessageScrollKey;

  // Captures the hold after the center fires.  Called from both center paths.
  const captureAnchorHold = React.useCallback((key: string) => {
    const container = scrollContainerRef.current;
    if (!container) return;
    const selectedRow = container.querySelector<HTMLElement>(
      '[data-testid="home-inbox-selected-message"]',
    );
    if (!selectedRow) return;
    const contentTop =
      selectedRow.getBoundingClientRect().top +
      container.scrollTop -
      container.getBoundingClientRect().top;
    anchorHoldRef.current = { contentTop, key };
  }, []);

  // Releases the hold — called on user interaction and selection-key change.
  const releaseAnchorHold = React.useCallback(() => {
    anchorHoldRef.current = null;
  }, []);

  const noteProgrammaticScroll = React.useCallback(
    (container: HTMLDivElement, scrollTopBefore: number) => {
      if (scrollTopBefore === container.scrollTop) return;

      programmaticScrollTopRef.current = container.scrollTop;
      // Scroll events run before the next animation frame. Expiring this guard
      // prevents a write whose event never arrives from swallowing a later user
      // scroll, while still ignoring the matching programmatic scroll event.
      window.requestAnimationFrame(() => {
        if (programmaticScrollTopRef.current === container.scrollTop) {
          programmaticScrollTopRef.current = null;
        }
      });
    },
    [],
  );

  // Arm the pending center whenever the selection key changes.
  React.useEffect(() => {
    if (!selectedMessageScrollKey) {
      pendingCenterKeyRef.current = null;
      releaseAnchorHold();
      return;
    }

    pendingCenterKeyRef.current = selectedMessageScrollKey;
    userScrolledRef.current = false;
    releaseAnchorHold();

    // Attempt the center after the current paint.  By the time this rAF fires,
    // any synchronous state updates (including isLoading → true) will have
    // been committed.  Read from the ref so we see the live value.
    const rafId = window.requestAnimationFrame(() => {
      if (isLoadingRef.current) {
        // Loading is in progress — cancel now; the transition effect will fire.
        return;
      }
      if (
        pendingCenterKeyRef.current === selectedMessageScrollKey &&
        !userScrolledRef.current
      ) {
        pendingCenterKeyRef.current = null;
        const container = scrollContainerRef.current;
        const scrollTopBefore = container?.scrollTop;
        isWritingScrollRef.current = true;
        detailPaneRef.current
          ?.querySelector<HTMLElement>(
            '[data-testid="home-inbox-selected-message"]',
          )
          ?.scrollIntoView({ block: "center" });
        isWritingScrollRef.current = false;
        if (container && scrollTopBefore !== undefined) {
          noteProgrammaticScroll(container, scrollTopBefore);
        }
        captureAnchorHold(selectedMessageScrollKey);
      }
    });

    return () => {
      window.cancelAnimationFrame(rafId);
    };
  }, [
    selectedMessageScrollKey,
    captureAnchorHold,
    releaseAnchorHold,
    noteProgrammaticScroll,
  ]);

  // Fire the deferred center when loading transitions true → false.
  React.useEffect(() => {
    const wasLoading = prevLoadingRef.current;
    prevLoadingRef.current = isThreadContextLoading;

    if (wasLoading && !isThreadContextLoading) {
      // Loading just settled.  If a center is still pending for the current
      // selection key and the user hasn't scrolled, execute it now.
      if (
        pendingCenterKeyRef.current === selectedMessageScrollKey &&
        selectedMessageScrollKey !== null &&
        !userScrolledRef.current
      ) {
        pendingCenterKeyRef.current = null;
        const container = scrollContainerRef.current;
        const scrollTopBefore = container?.scrollTop;
        isWritingScrollRef.current = true;
        detailPaneRef.current
          ?.querySelector<HTMLElement>(
            '[data-testid="home-inbox-selected-message"]',
          )
          ?.scrollIntoView({ block: "center" });
        isWritingScrollRef.current = false;
        if (container && scrollTopBefore !== undefined) {
          noteProgrammaticScroll(container, scrollTopBefore);
        }
        captureAnchorHold(selectedMessageScrollKey);
      }
    }
  }, [
    isThreadContextLoading,
    selectedMessageScrollKey,
    captureAnchorHold,
    noteProgrammaticScroll,
  ]);

  // Compensate for late content growth above the anchor (reactions, channel-
  // window merge, image decode).  Runs as a layout effect so drift is corrected
  // before paint, preventing visible flicker.
  // biome-ignore lint/correctness/useExhaustiveDependencies: messages and replies are the reactive triggers that drive displayMessages; we intentionally re-run on every message-list commit to catch reaction/merge re-renders; scrollContainerRef is a stable ref
  React.useLayoutEffect(() => {
    const hold = anchorHoldRef.current;
    const container = scrollContainerRef.current;
    if (
      !hold ||
      !container ||
      hold.key !== selectedMessageScrollKeyRef.current
    ) {
      return;
    }

    const selectedRow = container.querySelector<HTMLElement>(
      '[data-testid="home-inbox-selected-message"]',
    );
    if (!selectedRow) return;

    // Recompute contentTop: scroll-invariant absolute position within content.
    const currentContentTop =
      selectedRow.getBoundingClientRect().top +
      container.scrollTop -
      container.getBoundingClientRect().top;
    const drift = currentContentTop - hold.contentTop;
    if (Math.abs(drift) > 0.5) {
      hold.contentTop = currentContentTop;
      const scrollTopBefore = container.scrollTop;
      isWritingScrollRef.current = true;
      container.scrollBy(0, drift);
      isWritingScrollRef.current = false;
      noteProgrammaticScroll(container, scrollTopBefore);
    }
  }, [messages, replies, noteProgrammaticScroll]);

  // ResizeObserver: compensate for non-React resizes (image decode, embed
  // expand) that grow content above the anchor without triggering a React
  // re-render.  Keyed on conversationId so the observer is re-attached when
  // a new conversation opens and contentRef.current becomes a fresh node.
  // biome-ignore lint/correctness/useExhaustiveDependencies: conversationId is the re-attachment trigger; the effect body reads only stable refs
  React.useEffect(() => {
    const content = contentRef.current;
    if (!content || typeof ResizeObserver === "undefined") return;

    const observer = new ResizeObserver(() => {
      const hold = anchorHoldRef.current;
      const container = scrollContainerRef.current;
      if (
        !hold ||
        !container ||
        hold.key !== selectedMessageScrollKeyRef.current
      ) {
        return;
      }

      const selectedRow = container.querySelector<HTMLElement>(
        '[data-testid="home-inbox-selected-message"]',
      );
      if (!selectedRow) return;

      const currentContentTop =
        selectedRow.getBoundingClientRect().top +
        container.scrollTop -
        container.getBoundingClientRect().top;
      const drift = currentContentTop - hold.contentTop;
      if (Math.abs(drift) > 0.5) {
        hold.contentTop = currentContentTop;
        const scrollTopBefore = container.scrollTop;
        isWritingScrollRef.current = true;
        container.scrollBy(0, drift);
        isWritingScrollRef.current = false;
        noteProgrammaticScroll(container, scrollTopBefore);
      }
    });

    observer.observe(content);
    return () => {
      observer.disconnect();
    };
  }, [conversationId]);

  // Cancel the pending center if the user scrolls before it fires.
  // Keyed on conversationId so listeners are reinstalled when a conversation
  // opens — detailPaneRef.current is null before the item branch renders, so
  // a [] effect would attach to null and miss all subsequent selections.
  // biome-ignore lint/correctness/useExhaustiveDependencies: conversationId is not used inside the effect body; it is listed as a dep solely to trigger re-attachment when a new conversation opens and detailPaneRef.current becomes non-null
  React.useEffect(() => {
    const pane = detailPaneRef.current;
    const container = scrollContainerRef.current;
    if (!pane || !container) return;

    const handleUserInteraction = () => {
      userScrolledRef.current = true;
      releaseAnchorHold();
    };
    const handleContainerScroll = () => {
      if (isWritingScrollRef.current) return;

      if (programmaticScrollTopRef.current === container.scrollTop) {
        programmaticScrollTopRef.current = null;
        return;
      }

      programmaticScrollTopRef.current = null;
      handleUserInteraction();
    };

    pane.addEventListener("wheel", handleUserInteraction, { passive: true });
    pane.addEventListener("touchstart", handleUserInteraction, {
      passive: true,
    });
    pane.addEventListener("keydown", handleUserInteraction, { passive: true });
    container.addEventListener("scroll", handleContainerScroll, {
      passive: true,
    });

    return () => {
      pane.removeEventListener("wheel", handleUserInteraction);
      pane.removeEventListener("touchstart", handleUserInteraction);
      pane.removeEventListener("keydown", handleUserInteraction);
      container.removeEventListener("scroll", handleContainerScroll);
    };
  }, [conversationId, releaseAnchorHold]);

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

  const selectedMessage = messages.find((message) => message.isSelected);
  const pendingReplyMessages: InboxDisplayMessage[] = replies.map((reply) => ({
    ...reply,
    depth: reply.depth ?? (selectedMessage?.depth ?? 0) + 1,
    isSelected: false,
    mentionNames: [],
  }));
  const displayMessages: InboxDisplayMessage[] =
    messages.length > 0
      ? [...messages, ...pendingReplyMessages]
      : [
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
        ];
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
                      onClick={() => onOpenChannel(contextChannelId)}
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
                          onOpenChannel(contextChannelId);
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
          className="min-h-0 flex-1 overflow-y-auto overscroll-contain pb-32"
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
