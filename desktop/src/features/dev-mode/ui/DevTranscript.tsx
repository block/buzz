import * as React from "react";

import { useAppShell } from "@/app/AppShellContext";
import {
  useAuthorColorResolver,
  type AuthorColorResolver,
} from "@/features/dev-mode/lib/authorColors";
import {
  applyMessageEdits,
  collectMessageEdits,
} from "@/features/dev-mode/lib/messageEdits";
import {
  selectMembershipEvents,
  type MembershipChange,
} from "@/features/dev-mode/lib/membershipEvents";
import {
  collectReactions,
  type MessageReaction,
} from "@/features/dev-mode/lib/messageReactions";
import {
  byCreatedAscending,
  DEV_MESSAGE_KINDS,
  selectInlineVisibleCount,
  selectRootEvents,
} from "@/features/dev-mode/lib/transcriptRoots";
import type {
  AgentResolver,
  NameResolver,
} from "@/features/dev-mode/lib/useMemberNameResolver";
import {
  useMemberAgentResolver,
  useMemberNameResolver,
} from "@/features/dev-mode/lib/useMemberNameResolver";
import { selectUnreadThreadRoots } from "@/features/dev-mode/lib/unreadThreads";
import { usePinnedScroll } from "@/features/dev-mode/lib/usePinnedScroll";
import { DevMessageRow } from "@/features/dev-mode/ui/DevMessageRow";
import {
  useChannelMessagesQuery,
  useChannelSubscription,
  useChannelWindowQuery,
} from "@/features/messages/hooks";
import {
  channelWindowThreadSummaries,
  type ChannelWindowThreadSummary,
} from "@/features/messages/lib/channelWindowStore";
import { useLoadOlderOnScroll } from "@/features/messages/ui/useLoadOlderOnScroll";
import { useFetchOlderMessages } from "@/features/messages/useFetchOlderMessages";
import { useThreadReplies } from "@/features/messages/useThreadReplies";
import type { Channel, RelayEvent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

/**
 * The channel view shows the leading run of agent replies — everything
 * the agent said before a human responded — inline; once a human replies,
 * that message and everything after it lives in the side chat, collapsed
 * here into a "… N more replies" affordance. A thread whose first reply
 * is human still shows that one reply inline so prompts never render
 * without their first response.
 */
function ThreadInlineReplies({
  channel,
  rootId,
  replyCount,
  unread,
  markRead,
  currentPubkey,
  resolveName,
  resolveColor,
  resolveIsAgent,
  onOpenThread,
}: {
  channel: Channel;
  rootId: string;
  replyCount: number;
  unread: boolean;
  /** Whether rendering the inline reply advances the thread read frontier. */
  markRead: boolean;
  currentPubkey: string | null;
  resolveName: NameResolver;
  resolveColor: AuthorColorResolver;
  resolveIsAgent: AgentResolver;
  onOpenThread: () => void;
}) {
  const repliesQuery = useThreadReplies(channel, rootId);
  // Thread fetches include kind:40003 edit aux events — resolve them here
  // so edited replies render their current text.
  const replyEdits = React.useMemo(
    () => collectMessageEdits(repliesQuery.data),
    [repliesQuery.data],
  );
  const replies = React.useMemo(
    () =>
      applyMessageEdits(repliesQuery.data)
        .filter((event) => DEV_MESSAGE_KINDS.has(event.kind))
        .sort(byCreatedAscending),
    [repliesQuery.data],
  );
  // Thread fetches include their reaction aux events (see useThreadReplies).
  const reactions = React.useMemo(
    () => collectReactions(repliesQuery.data),
    [repliesQuery.data],
  );

  const visible = React.useMemo(
    () => replies.slice(0, selectInlineVisibleCount(replies, resolveIsAgent)),
    [replies, resolveIsAgent],
  );
  // The summary count can outrun the fetched subtree (live recounts) —
  // trust whichever knows about more replies.
  const moreCount = Math.max(replyCount, replies.length) - visible.length;

  // The inline replies are on screen, so seeing the channel counts as
  // reading them: advance the thread frontier through the last visible
  // reply. Later (collapsed) replies stay unread until the side chat is
  // opened.
  const { getThreadReadAt, markThreadRead } = useAppShell();
  const lastVisibleAt = visible.at(-1)?.created_at ?? null;
  const channelId = channel.id;
  React.useEffect(() => {
    if (!markRead || lastVisibleAt === null) return;
    const readAt = getThreadReadAt(rootId, channelId);
    if (readAt === null || readAt < lastVisibleAt) {
      markThreadRead(rootId, lastVisibleAt);
    }
  }, [
    channelId,
    lastVisibleAt,
    getThreadReadAt,
    markRead,
    markThreadRead,
    rootId,
  ]);

  // The replies sit on the same indent as the prompt that produced them.
  return (
    <div className="mt-1">
      {visible.length > 0 ? (
        visible.map((reply) => (
          <DevMessageRow
            key={reply.localKey ?? reply.id}
            event={reply}
            currentPubkey={currentPubkey}
            edited={replyEdits.has(reply.id)}
            reactions={reactions.get(reply.id)}
            resolveColor={resolveColor}
            resolveIsAgent={resolveIsAgent}
            resolveName={resolveName}
          />
        ))
      ) : repliesQuery.isLoading ? (
        <div className="py-0.5 text-sm text-muted-foreground/60">
          loading reply…
        </div>
      ) : null}
      {repliesQuery.isError ? (
        <button
          className="cursor-pointer py-0.5 text-sm text-destructive hover:underline"
          onClick={() => void repliesQuery.refetch()}
          type="button"
        >
          failed to load replies — retry
        </button>
      ) : moreCount > 0 ? (
        <button
          className={cn(
            "mt-1 cursor-pointer py-0.5 text-sm",
            unread
              ? "text-primary hover:text-primary/80"
              : "text-muted-foreground hover:text-foreground",
          )}
          data-testid="dev-mode-more-replies"
          data-unread={unread || undefined}
          onClick={(event) => {
            event.stopPropagation();
            onOpenThread();
          }}
          type="button"
        >
          … {moreCount} more {moreCount === 1 ? "reply" : "replies"}
          {unread ? " ●" : ""}
        </button>
      ) : null}
    </div>
  );
}

function MembershipRow({
  change,
  resolveName,
  resolveColor,
}: {
  change: MembershipChange;
  resolveName: NameResolver;
  resolveColor: AuthorColorResolver;
}) {
  const name = (pubkey: string) => (
    <span style={{ color: resolveColor(pubkey) }}>{resolveName(pubkey)}</span>
  );
  return (
    <div
      className="mb-2 select-none px-3 text-xs text-muted-foreground/70"
      data-testid="dev-mode-membership-row"
    >
      {change.change === "left" || change.change === "removed" ? "← " : "→ "}
      {name(change.member)}{" "}
      {change.change === "joined" ? (
        "joined"
      ) : change.change === "added" && change.actor ? (
        <>added by {name(change.actor)}</>
      ) : change.change === "added" ? (
        "joined"
      ) : change.change === "left" ? (
        "left"
      ) : change.actor ? (
        <>removed by {name(change.actor)}</>
      ) : (
        "removed"
      )}
    </div>
  );
}

function PromptCard({
  channel,
  root,
  rootReactions,
  replyCount,
  unread,
  markRead,
  selected,
  edited,
  currentPubkey,
  resolveName,
  resolveColor,
  resolveIsAgent,
  onOpenThread,
}: {
  channel: Channel;
  root: RelayEvent;
  rootReactions: MessageReaction[] | undefined;
  replyCount: number;
  unread: boolean;
  markRead: boolean;
  selected: boolean;
  edited: boolean;
  currentPubkey: string | null;
  resolveName: NameResolver;
  resolveColor: AuthorColorResolver;
  resolveIsAgent: AgentResolver;
  onOpenThread: () => void;
}) {
  // Callback ref mounts only on the selected card, so keyboard navigation
  // scrolls the newly selected card into view without an effect.
  const scrollSelectedIntoView = React.useCallback(
    (node: HTMLDivElement | null) => {
      node?.scrollIntoView({ block: "nearest" });
    },
    [],
  );

  return (
    // Selection is keyboard-only (↑/↓ + Enter on the composer); clicks land
    // here only for text selection and never move focus or selection.
    <div
      ref={selected ? scrollSelectedIntoView : undefined}
      className={cn(
        // The border is transparent until keyboard focus so nothing shifts
        // when ↑/↓ start walking the prompts.
        "relative mb-2 rounded-none border px-3 py-2",
        selected ? "border-primary/60 bg-primary/5" : "border-transparent",
      )}
      data-testid="dev-mode-prompt-card"
    >
      {/* Absolute so selecting a card never changes its height (no layout
          shift while ↑/↓ walk the prompts). */}
      {selected || unread ? (
        <div className="pointer-events-none absolute right-1 top-1 flex select-none items-center gap-1.5 bg-background/90 px-1 text-xs text-primary/80">
          {selected ? <span>⏎ side chat</span> : null}
          {unread ? (
            <span
              aria-label="unread thread"
              className="text-3xs leading-none text-primary"
              data-testid="dev-mode-card-unread-dot"
              role="img"
            >
              ●
            </span>
          ) : null}
        </div>
      ) : null}
      <DevMessageRow
        event={root}
        currentPubkey={currentPubkey}
        edited={edited}
        reactions={rootReactions}
        resolveColor={resolveColor}
        resolveIsAgent={resolveIsAgent}
        resolveName={resolveName}
      />
      {replyCount > 0 ? (
        <ThreadInlineReplies
          channel={channel}
          currentPubkey={currentPubkey}
          markRead={markRead}
          onOpenThread={onOpenThread}
          replyCount={replyCount}
          resolveColor={resolveColor}
          resolveIsAgent={resolveIsAgent}
          resolveName={resolveName}
          rootId={root.id}
          unread={unread}
        />
      ) : null}
    </div>
  );
}

export function DevTranscript({
  channel,
  currentPubkey,
  selectedRootId,
  markRead,
  onOpenThread,
}: {
  channel: Channel;
  currentPubkey: string | null;
  selectedRootId: string | null;
  /** False for previews — looking at a preview must not advance read state. */
  markRead: boolean;
  onOpenThread: (rootId: string) => void;
}) {
  const messagesQuery = useChannelMessagesQuery(channel);
  const windowQuery = useChannelWindowQuery(channel);
  useChannelSubscription(channel);
  const { getThreadReadAt, readStateVersion } = useAppShell();

  const { scrollRef, contentRef, handleScroll } = usePinnedScroll(channel.id);

  const { fetchOlder, hasOlderMessages, isFetchingOlder } =
    useFetchOlderMessages(channel);

  const roots = React.useMemo(
    () => selectRootEvents(messagesQuery.data),
    [messagesQuery.data],
  );
  // Which roots carry an edit — drives the "(edited)" marker (the edited
  // content itself is already applied inside selectRootEvents).
  const rootEdits = React.useMemo(
    () => collectMessageEdits(messagesQuery.data),
    [messagesQuery.data],
  );
  const memberships = React.useMemo(
    () => selectMembershipEvents(messagesQuery.data),
    [messagesQuery.data],
  );

  // Membership rows can name people who already left — resolve those via
  // the profile fallback rather than the (current-only) member list.
  const membershipPubkeys = React.useMemo(
    () => [
      ...new Set(
        memberships.flatMap((change) =>
          change.actor ? [change.member, change.actor] : [change.member],
        ),
      ),
    ],
    [memberships],
  );
  const resolveName = useMemberNameResolver(channel.id, membershipPubkeys);
  const resolveColor = useAuthorColorResolver();
  const resolveIsAgent = useMemberAgentResolver(channel.id);

  // Prompt cards and member join/leave rows share one chronological flow;
  // membership rows are narration only — ↑/↓ card navigation skips them.
  const items = React.useMemo(() => {
    const merged: Array<
      | { type: "prompt"; root: RelayEvent }
      | { type: "membership"; change: MembershipChange }
    > = [
      ...roots.map((root) => ({
        type: "prompt" as const,
        root,
      })),
      ...memberships.map((change) => ({
        type: "membership" as const,
        change,
      })),
    ];
    return merged.sort((left, right) =>
      byCreatedAscending(
        left.type === "prompt" ? left.root : left.change.event,
        right.type === "prompt" ? right.root : right.change.event,
      ),
    );
  }, [memberships, roots]);

  // Scroll-up pagination. Chromium's native scroll anchoring keeps the view
  // stable when older pages prepend — except at scrollTop 0, where the spec
  // suppresses anchoring. Capture the pre-fetch metrics and compensate
  // manually in that one case once the prepended rows commit.
  const topSentinelRef = React.useRef<HTMLDivElement>(null);
  const pendingPrependRef = React.useRef<{ height: number } | null>(null);
  const fetchOlderCompensated = React.useCallback(async () => {
    const node = scrollRef.current;
    if (node) pendingPrependRef.current = { height: node.scrollHeight };
    await fetchOlder();
  }, [fetchOlder, scrollRef]);

  const oldestItemKey =
    items[0] === undefined
      ? null
      : items[0].type === "prompt"
        ? items[0].root.id
        : items[0].change.event.id;
  // biome-ignore lint/correctness/useExhaustiveDependencies: oldestItemKey changing is the prepend-commit signal
  React.useLayoutEffect(() => {
    const pending = pendingPrependRef.current;
    if (!pending) return;
    pendingPrependRef.current = null;
    const node = scrollRef.current;
    // scrollTop > 0 means the browser's scroll anchoring already adjusted.
    if (!node || node.scrollTop > 0) return;
    node.scrollTop = node.scrollHeight - pending.height;
  }, [oldestItemKey, scrollRef]);

  useLoadOlderOnScroll({
    fetchOlder: fetchOlderCompensated,
    hasOlderMessages,
    isLoading: messagesQuery.isLoading,
    scrollContainerRef: scrollRef,
    sentinelRef: topSentinelRef,
  });

  const threadSummaries = React.useMemo(
    () =>
      windowQuery.data
        ? channelWindowThreadSummaries(windowQuery.data)
        : new Map<string, ChannelWindowThreadSummary>(),
    [windowQuery.data],
  );

  const replyCounts = React.useMemo(() => {
    const counts = new Map<string, number>();
    for (const [rootId, summary] of threadSummaries) {
      counts.set(rootId, summary.replyCount);
    }
    return counts;
  }, [threadSummaries]);

  // Threads with replies past the read frontier — carries the per-card
  // unread dot. readStateVersion invalidates when any read marker moves.
  // biome-ignore lint/correctness/useExhaustiveDependencies: readStateVersion is an intentional invalidation signal
  const unreadRootIds = React.useMemo(
    () =>
      selectUnreadThreadRoots(threadSummaries, (rootId) =>
        getThreadReadAt(rootId, channel.id),
      ),
    [channel.id, getThreadReadAt, threadSummaries, readStateVersion],
  );

  // Kind-7 reactions ride along as window aux events (pages + live); agents
  // react while working, so these double as a per-prompt activity signal.
  const rootReactions = React.useMemo(() => {
    const store = windowQuery.data;
    if (!store) return new Map<string, MessageReaction[]>();
    return collectReactions([
      ...store.pages.flatMap((page) => page.aux),
      ...store.liveAux,
    ]);
  }, [windowQuery.data]);

  return (
    <div
      ref={scrollRef}
      className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden px-4 py-3 font-mono"
      data-allow-text-selection
      data-testid="dev-mode-transcript"
      onScroll={handleScroll}
    >
      <div ref={contentRef}>
        <div aria-hidden ref={topSentinelRef} />
        {isFetchingOlder ? (
          <div
            className="pb-2 text-sm text-muted-foreground/60"
            data-testid="dev-mode-loading-older"
          >
            loading older messages…
          </div>
        ) : null}
        {items.map((item) =>
          item.type === "membership" ? (
            <MembershipRow
              key={item.change.event.id}
              change={item.change}
              resolveColor={resolveColor}
              resolveName={resolveName}
            />
          ) : (
            <PromptCard
              key={item.root.localKey ?? item.root.id}
              channel={channel}
              currentPubkey={currentPubkey}
              edited={rootEdits.has(item.root.id)}
              markRead={markRead}
              onOpenThread={() => onOpenThread(item.root.id)}
              replyCount={replyCounts.get(item.root.id) ?? 0}
              unread={unreadRootIds.has(item.root.id)}
              resolveColor={resolveColor}
              resolveIsAgent={resolveIsAgent}
              resolveName={resolveName}
              root={item.root}
              rootReactions={rootReactions.get(item.root.id)}
              selected={item.root.id === selectedRootId}
            />
          ),
        )}
        {messagesQuery.isLoading && roots.length === 0 ? (
          <div className="text-sm text-muted-foreground/60">loading…</div>
        ) : null}
        {messagesQuery.isError ? (
          <button
            className="cursor-pointer py-0.5 text-sm text-destructive hover:underline"
            onClick={() => void messagesQuery.refetch()}
            type="button"
          >
            failed to load messages — retry
          </button>
        ) : null}
      </div>
    </div>
  );
}
