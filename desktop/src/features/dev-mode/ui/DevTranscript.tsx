import * as React from "react";

import { useChannelMembersQuery } from "@/features/channels/hooks";
import {
  useChannelMessagesQuery,
  useChannelSubscription,
  useChannelWindowQuery,
} from "@/features/messages/hooks";
import { isThreadReply } from "@/features/messages/lib/threading";
import { useThreadReplies } from "@/features/messages/useThreadReplies";
import type { Channel, RelayEvent } from "@/shared/api/types";
import {
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
  KIND_SYSTEM_MESSAGE,
} from "@/shared/constants/kinds";
import { cn } from "@/shared/lib/cn";
import { truncatePubkey } from "@/shared/lib/pubkey";

const MESSAGE_KINDS = new Set([KIND_STREAM_MESSAGE, KIND_STREAM_MESSAGE_V2]);

/** Newest roots whose thread replies auto-expand without a click. */
const AUTO_EXPAND_ROOT_COUNT = 3;

/** Distance from the bottom (px) within which the view stays pinned. */
const PIN_THRESHOLD = 48;

function byCreatedAscending(left: RelayEvent, right: RelayEvent) {
  return left.created_at !== right.created_at
    ? left.created_at - right.created_at
    : left.id < right.id
      ? -1
      : 1;
}

function formatTime(createdAt: number) {
  return new Date(createdAt * 1_000).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
  });
}

type NameResolver = (pubkey: string) => string;

function TranscriptRow({
  event,
  isSelf,
  resolveName,
}: {
  event: RelayEvent;
  isSelf: boolean;
  resolveName: NameResolver;
}) {
  if (event.kind === KIND_SYSTEM_MESSAGE) {
    return null;
  }

  return (
    <div className="flex gap-2 py-0.5 text-sm leading-6">
      <span className="shrink-0 select-none text-muted-foreground/50">
        {formatTime(event.created_at)}
      </span>
      <span
        className={cn(
          "shrink-0 font-medium",
          isSelf ? "text-foreground" : "text-primary",
        )}
      >
        {resolveName(event.pubkey)}
      </span>
      <span
        className={cn(
          "min-w-0 whitespace-pre-wrap break-words",
          event.pending && "text-muted-foreground",
        )}
      >
        {event.content}
      </span>
    </div>
  );
}

/** Mounted only while expanded, so collapsed roots carry no query observer. */
function ThreadReplies({
  channel,
  rootId,
  currentPubkey,
  resolveName,
}: {
  channel: Channel;
  rootId: string;
  currentPubkey: string | null;
  resolveName: NameResolver;
}) {
  const repliesQuery = useThreadReplies(channel, rootId);
  const replies = React.useMemo(
    () =>
      (repliesQuery.data ?? [])
        .filter((event) => MESSAGE_KINDS.has(event.kind))
        .sort(byCreatedAscending),
    [repliesQuery.data],
  );

  return (
    <div className="ml-14 border-l border-border/60 pl-3">
      {replies.map((reply) => (
        <TranscriptRow
          key={reply.localKey ?? reply.id}
          event={reply}
          isSelf={reply.pubkey === currentPubkey}
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

function TranscriptThread({
  channel,
  root,
  replyCount,
  autoExpand,
  currentPubkey,
  resolveName,
}: {
  channel: Channel;
  root: RelayEvent;
  replyCount: number;
  autoExpand: boolean;
  currentPubkey: string | null;
  resolveName: NameResolver;
}) {
  const [manuallyExpanded, setManuallyExpanded] = React.useState(false);
  const expanded = (autoExpand || manuallyExpanded) && replyCount > 0;

  return (
    <div>
      <TranscriptRow
        event={root}
        isSelf={root.pubkey === currentPubkey}
        resolveName={resolveName}
      />
      {expanded ? (
        <ThreadReplies
          channel={channel}
          currentPubkey={currentPubkey}
          resolveName={resolveName}
          rootId={root.id}
        />
      ) : replyCount > 0 ? (
        <button
          className="ml-14 cursor-pointer border-l border-border/60 py-0.5 pl-3 text-sm text-muted-foreground hover:text-foreground"
          onClick={() => setManuallyExpanded(true)}
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
}: {
  channel: Channel;
  currentPubkey: string | null;
}) {
  const messagesQuery = useChannelMessagesQuery(channel);
  const windowQuery = useChannelWindowQuery(channel);
  const membersQuery = useChannelMembersQuery(channel.id);
  useChannelSubscription(channel);

  const scrollRef = React.useRef<HTMLDivElement>(null);
  const contentRef = React.useRef<HTMLDivElement>(null);
  const pinnedRef = React.useRef(true);

  const handleScroll = React.useCallback(() => {
    const node = scrollRef.current;
    if (!node) return;
    pinnedRef.current =
      node.scrollHeight - node.scrollTop - node.clientHeight < PIN_THRESHOLD;
  }, []);

  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional — a channel switch re-pins the view to the bottom
  React.useLayoutEffect(() => {
    const node = scrollRef.current;
    if (!node) return;
    node.scrollTop = node.scrollHeight;
    pinnedRef.current = true;
  }, [channel.id]);

  // Any content growth (new roots, replies loading in, live agent output)
  // keeps the view pinned to the bottom unless the user scrolled up.
  React.useEffect(() => {
    const content = contentRef.current;
    const scroller = scrollRef.current;
    if (!content || !scroller) return;
    const observer = new ResizeObserver(() => {
      if (pinnedRef.current) {
        scroller.scrollTop = scroller.scrollHeight;
      }
    });
    observer.observe(content);
    return () => observer.disconnect();
  }, []);

  const resolveName = React.useCallback<NameResolver>(
    (pubkey) => {
      const member = membersQuery.data?.find(
        (candidate) => candidate.pubkey === pubkey,
      );
      return member?.displayName || truncatePubkey(pubkey);
    },
    [membersQuery.data],
  );

  const roots = React.useMemo(
    () =>
      (messagesQuery.data ?? [])
        .filter(
          (event) =>
            MESSAGE_KINDS.has(event.kind) && !isThreadReply(event.tags),
        )
        .sort(byCreatedAscending),
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

  return (
    <div
      ref={scrollRef}
      className="min-h-0 flex-1 overflow-y-auto px-4 py-3 font-mono"
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
          <TranscriptThread
            key={root.localKey ?? root.id}
            autoExpand={index >= roots.length - AUTO_EXPAND_ROOT_COUNT}
            channel={channel}
            currentPubkey={currentPubkey}
            replyCount={replyCounts.get(root.id) ?? 0}
            resolveName={resolveName}
            root={root}
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
