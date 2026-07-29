import * as React from "react";

import {
  useAuthorColorResolver,
  type AuthorColorResolver,
} from "@/features/dev-mode/lib/authorColors";
import { collectReactions } from "@/features/dev-mode/lib/messageReactions";
import {
  byCreatedAscending,
  DEV_MESSAGE_KINDS,
  selectRootEvents,
} from "@/features/dev-mode/lib/transcriptRoots";
import type { NameResolver } from "@/features/dev-mode/lib/useMemberNameResolver";
import { useMemberNameResolver } from "@/features/dev-mode/lib/useMemberNameResolver";
import { usePinnedScroll } from "@/features/dev-mode/lib/usePinnedScroll";
import { DevMessageRow } from "@/features/dev-mode/ui/DevMessageRow";
import {
  useChannelMessagesQuery,
  useChannelSubscription,
  useChannelWindowQuery,
} from "@/features/messages/hooks";
import { useThreadReplies } from "@/features/messages/useThreadReplies";
import type { Channel, RelayEvent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";

/** Newest prompt cards whose thread replies render inline without a click. */
const AUTO_EXPAND_ROOT_COUNT = 3;

/** Mounted only while expanded, so collapsed cards carry no query observer. */
function ThreadReplies({
  channel,
  rootId,
  currentPubkey,
  resolveName,
  resolveColor,
}: {
  channel: Channel;
  rootId: string;
  currentPubkey: string | null;
  resolveName: NameResolver;
  resolveColor: AuthorColorResolver;
}) {
  const repliesQuery = useThreadReplies(channel, rootId);
  const replies = React.useMemo(
    () =>
      (repliesQuery.data ?? [])
        .filter((event) => DEV_MESSAGE_KINDS.has(event.kind))
        .sort(byCreatedAscending),
    [repliesQuery.data],
  );
  // Thread fetches include their reaction aux events (see useThreadReplies).
  const reactions = React.useMemo(
    () => collectReactions(repliesQuery.data),
    [repliesQuery.data],
  );

  // Replies sit on the same indent as the prompt that produced them.
  return (
    <div className="mt-1">
      {replies.map((reply) => (
        <DevMessageRow
          key={reply.localKey ?? reply.id}
          event={reply}
          isSelf={reply.pubkey === currentPubkey}
          reactions={reactions.get(reply.id)}
          resolveColor={resolveColor}
          resolveName={resolveName}
        />
      ))}
      {repliesQuery.isLoading ? (
        <div className="py-0.5 text-sm text-muted-foreground/60">
          loading replies…
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
      ) : null}
    </div>
  );
}

function PromptCard({
  channel,
  root,
  rootReactions,
  replyCount,
  autoExpand,
  selected,
  currentPubkey,
  resolveName,
  resolveColor,
  onOpenThread,
}: {
  channel: Channel;
  root: RelayEvent;
  rootReactions: string[] | undefined;
  replyCount: number;
  autoExpand: boolean;
  selected: boolean;
  currentPubkey: string | null;
  resolveName: NameResolver;
  resolveColor: AuthorColorResolver;
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
        "relative mb-2 rounded-none border px-3 py-2",
        selected ? "border-primary/60 bg-primary/5" : "border-border/40",
      )}
      data-testid="dev-mode-prompt-card"
    >
      {/* Absolute so selecting a card never changes its height (no layout
          shift while ↑/↓ walk the prompts). */}
      {selected ? (
        <div className="pointer-events-none absolute right-1 top-1 select-none bg-background/90 px-1 text-xs text-primary/80">
          ⏎ side chat
        </div>
      ) : null}
      <DevMessageRow
        event={root}
        isSelf={root.pubkey === currentPubkey}
        reactions={rootReactions}
        resolveColor={resolveColor}
        resolveName={resolveName}
      />
      {autoExpand && replyCount > 0 ? (
        <ThreadReplies
          channel={channel}
          currentPubkey={currentPubkey}
          resolveColor={resolveColor}
          resolveName={resolveName}
          rootId={root.id}
        />
      ) : replyCount > 0 ? (
        <button
          className="mt-1 cursor-pointer py-0.5 text-sm text-muted-foreground hover:text-foreground"
          onClick={(event) => {
            event.stopPropagation();
            onOpenThread();
          }}
          type="button"
        >
          … {replyCount} {replyCount === 1 ? "reply" : "replies"}
        </button>
      ) : null}
    </div>
  );
}

export function DevTranscript({
  channel,
  currentPubkey,
  selectedRootId,
  onOpenThread,
}: {
  channel: Channel;
  currentPubkey: string | null;
  selectedRootId: string | null;
  onOpenThread: (rootId: string) => void;
}) {
  const messagesQuery = useChannelMessagesQuery(channel);
  const windowQuery = useChannelWindowQuery(channel);
  useChannelSubscription(channel);

  const { scrollRef, contentRef, handleScroll } = usePinnedScroll(channel.id);
  const resolveName = useMemberNameResolver(channel.id);
  const resolveColor = useAuthorColorResolver();

  const roots = React.useMemo(
    () => selectRootEvents(messagesQuery.data),
    [messagesQuery.data],
  );

  const replyCounts = React.useMemo(() => {
    const counts = new Map<string, number>();
    const store = windowQuery.data;
    if (!store) return counts;
    for (const page of store.pages) {
      for (const row of page.rows) {
        if (row.thread) counts.set(row.event.id, row.thread.replyCount);
      }
    }
    for (const [rootId, live] of Object.entries(store.liveSummaries)) {
      counts.set(rootId, live.summary.replyCount);
    }
    return counts;
  }, [windowQuery.data]);

  // Kind-7 reactions ride along as window aux events (pages + live); agents
  // react while working, so these double as a per-prompt activity signal.
  const rootReactions = React.useMemo(() => {
    const store = windowQuery.data;
    if (!store) return new Map<string, string[]>();
    return collectReactions([
      ...store.pages.flatMap((page) => page.aux),
      ...store.liveAux,
    ]);
  }, [windowQuery.data]);

  return (
    <div
      ref={scrollRef}
      className="min-h-0 flex-1 overflow-y-auto px-4 py-3 font-mono"
      data-allow-text-selection
      data-testid="dev-mode-transcript"
      onScroll={handleScroll}
    >
      <div ref={contentRef}>
        <div className="pb-2 text-sm text-muted-foreground">
          # {channel.name}
          {channel.description ? (
            <span className="text-muted-foreground/60">
              {" "}
              — {channel.description}
            </span>
          ) : null}
        </div>
        {roots.map((root, index) => (
          <PromptCard
            key={root.localKey ?? root.id}
            autoExpand={index >= roots.length - AUTO_EXPAND_ROOT_COUNT}
            channel={channel}
            currentPubkey={currentPubkey}
            onOpenThread={() => onOpenThread(root.id)}
            replyCount={replyCounts.get(root.id) ?? 0}
            resolveColor={resolveColor}
            resolveName={resolveName}
            root={root}
            rootReactions={rootReactions.get(root.id)}
            selected={root.id === selectedRootId}
          />
        ))}
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
