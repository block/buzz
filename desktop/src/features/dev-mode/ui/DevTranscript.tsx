import * as React from "react";

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
        .filter((event) => DEV_MESSAGE_KINDS.has(event.kind))
        .sort(byCreatedAscending),
    [repliesQuery.data],
  );

  return (
    <div className="mt-1 border-l border-border/60 pl-3">
      {replies.map((reply) => (
        <DevMessageRow
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

function PromptCard({
  channel,
  root,
  replyCount,
  autoExpand,
  selected,
  currentPubkey,
  resolveName,
  onSelect,
  onOpenThread,
}: {
  channel: Channel;
  root: RelayEvent;
  replyCount: number;
  autoExpand: boolean;
  selected: boolean;
  currentPubkey: string | null;
  resolveName: NameResolver;
  onSelect: () => void;
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
    // biome-ignore lint/a11y/useKeyWithClickEvents: keyboard selection is handled globally by the composer (↑↓ + Enter)
    // biome-ignore lint/a11y/useSemanticElements: a <button> card would nest the interactive replies <button>, which is invalid HTML
    <div
      ref={selected ? scrollSelectedIntoView : undefined}
      role="button"
      tabIndex={-1}
      className={cn(
        "mb-2 cursor-pointer rounded-md border px-3 py-2",
        selected
          ? "border-primary/60 bg-primary/5"
          : "border-border/40 hover:border-border",
      )}
      data-testid="dev-mode-prompt-card"
      onClick={onSelect}
      onDoubleClick={onOpenThread}
    >
      <DevMessageRow
        event={root}
        isSelf={root.pubkey === currentPubkey}
        resolveName={resolveName}
      />
      {autoExpand && replyCount > 0 ? (
        <ThreadReplies
          channel={channel}
          currentPubkey={currentPubkey}
          resolveName={resolveName}
          rootId={root.id}
        />
      ) : replyCount > 0 ? (
        <button
          className="mt-1 cursor-pointer border-l border-border/60 py-0.5 pl-3 text-sm text-muted-foreground hover:text-foreground"
          onClick={(event) => {
            event.stopPropagation();
            onOpenThread();
          }}
          type="button"
        >
          … {replyCount} {replyCount === 1 ? "reply" : "replies"}
        </button>
      ) : null}
      {selected ? (
        <div className="mt-1 select-none text-right text-xs text-primary/80">
          ⏎ side chat
        </div>
      ) : null}
    </div>
  );
}

export function DevTranscript({
  channel,
  currentPubkey,
  selectedRootId,
  onSelectRoot,
  onOpenThread,
}: {
  channel: Channel;
  currentPubkey: string | null;
  selectedRootId: string | null;
  onSelectRoot: (rootId: string | null) => void;
  onOpenThread: (rootId: string) => void;
}) {
  const messagesQuery = useChannelMessagesQuery(channel);
  const windowQuery = useChannelWindowQuery(channel);
  useChannelSubscription(channel);

  const { scrollRef, contentRef, handleScroll } = usePinnedScroll(channel.id);
  const resolveName = useMemberNameResolver(channel.id);

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
          <PromptCard
            key={root.localKey ?? root.id}
            autoExpand={index >= roots.length - AUTO_EXPAND_ROOT_COUNT}
            channel={channel}
            currentPubkey={currentPubkey}
            onOpenThread={() => onOpenThread(root.id)}
            onSelect={() =>
              onSelectRoot(selectedRootId === root.id ? null : root.id)
            }
            replyCount={replyCounts.get(root.id) ?? 0}
            resolveName={resolveName}
            root={root}
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
