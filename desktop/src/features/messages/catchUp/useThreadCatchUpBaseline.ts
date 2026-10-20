import * as React from "react";

import {
  msgContextKey,
  THREAD_PREFIX,
} from "@/features/channels/readState/readStateFormat";
import type { CatchUpBaselineSnapshot } from "./threadCatchUp";

type ChannelCapture = {
  channelId: string;
  scopeKey: string;
  channelReadAt: number | null;
  channelReady: boolean;
  inboxSnapshot?: CatchUpBaselineSnapshot;
  threads: Map<
    string,
    {
      threadReadAt: number | null;
      threadReady: boolean;
      replyReadAt: Map<string, number | null>;
      repliesReady: boolean;
    }
  >;
};

/**
 * Freeze the reader's own read markers for this channel visit before any
 * mark-read effect can move them.
 *
 * Runs during render: opening a channel advances its marker, opening a thread
 * marks visible replies, and expanding a branch marks revealed replies, all
 * from effects or event handlers that run after the render in which the
 * relevant marker is first observed here. Each value is captured once — the
 * channel on first render of the visit, the thread when it first opens, and
 * each reply the first time it appears in the loaded thread (including
 * replies that load later or sit in collapsed branches) — so later reads in
 * the same visit cannot masquerade as the previous read position.
 *
 * Captures taken before read state finished loading are flagged so the recap
 * asks for an explicit lookback instead of trusting a partial marker.
 */
export function useThreadCatchUpBaseline({
  channelId,
  rootEventId,
  threadMessageIds,
  getOwnReadAt,
  isReadStateReady,
  inboxSnapshot,
  scopeKey = "",
}: {
  channelId: string | null;
  rootEventId: string | null;
  threadMessageIds: readonly string[];
  getOwnReadAt: (contextId: string) => number | null;
  isReadStateReady: boolean;
  inboxSnapshot?: CatchUpBaselineSnapshot;
  scopeKey?: string;
}): CatchUpBaselineSnapshot | null {
  const captureRef = React.useRef<ChannelCapture | null>(null);

  if (!channelId) {
    captureRef.current = null;
    return null;
  }
  if (
    captureRef.current?.channelId !== channelId ||
    captureRef.current.scopeKey !== scopeKey
  ) {
    captureRef.current = {
      channelId,
      scopeKey,
      channelReadAt: inboxSnapshot
        ? inboxSnapshot.channelReadAt
        : getOwnReadAt(channelId),
      channelReady: inboxSnapshot
        ? inboxSnapshot.channelReady
        : isReadStateReady,
      inboxSnapshot,
      threads: new Map(),
    };
  }
  const capture = captureRef.current;
  if (!rootEventId) return null;

  let thread = capture.threads.get(rootEventId);
  if (!thread) {
    const previous =
      capture.inboxSnapshot?.rootEventId === rootEventId
        ? capture.inboxSnapshot
        : null;
    thread = {
      threadReadAt: previous
        ? previous.threadReadAt
        : getOwnReadAt(`${THREAD_PREFIX}${rootEventId}`),
      threadReady: previous ? previous.threadReady : isReadStateReady,
      replyReadAt: new Map(previous?.replyReadAt),
      repliesReady: previous ? previous.repliesReady : true,
    };
    capture.threads.set(rootEventId, thread);
  }
  for (const id of threadMessageIds) {
    if (id === rootEventId || thread.replyReadAt.has(id)) continue;
    thread.replyReadAt.set(id, getOwnReadAt(msgContextKey(id)));
    if (!isReadStateReady) thread.repliesReady = false;
  }

  return {
    channelId,
    rootEventId,
    channelReadAt: capture.channelReadAt,
    channelReady: capture.channelReady,
    threadReadAt: thread.threadReadAt,
    threadReady: thread.threadReady,
    replyReadAt: thread.replyReadAt,
    repliesReady: thread.repliesReady,
  };
}
