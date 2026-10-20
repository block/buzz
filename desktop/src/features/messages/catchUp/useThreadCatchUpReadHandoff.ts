import * as React from "react";
import {
  msgContextKey,
  THREAD_PREFIX,
} from "@/features/channels/readState/readStateFormat";

import type { CatchUpBaselineSnapshot } from "./threadCatchUp";

type Handoff = { key: string; snapshot: CatchUpBaselineSnapshot };
const HANDOFF_TTL_MS = 5 * 60 * 1_000;
const MAX_HANDOFFS = 16;

export type ThreadCatchUpReadHandoff = {
  capture: (input: {
    channelId: string;
    rootEventId: string;
    messageIds: string[];
  }) => void;
  peek: (channelId: string) => Handoff | null;
  consume: (key: string) => void;
};

export const EMPTY_CATCH_UP_READ_HANDOFF: ThreadCatchUpReadHandoff = {
  capture: () => {},
  peek: () => null,
  consume: () => {},
};

/** Carry one Inbox selection's markers across navigation, before it marks read. */
export function useThreadCatchUpReadHandoff({
  relayUrl,
  pubkey,
  getOwnReadAt,
  isReadStateReady,
}: {
  relayUrl?: string;
  pubkey?: string;
  getOwnReadAt: (contextId: string) => number | null;
  isReadStateReady: boolean;
}): ThreadCatchUpReadHandoff {
  const pending = React.useRef(
    new Map<string, Handoff & { capturedAt: number }>(),
  );
  const selected = React.useRef<string | null>(null);
  const scope = JSON.stringify([relayUrl ?? "", pubkey?.toLowerCase() ?? ""]);
  const previousScope = React.useRef(scope);
  if (previousScope.current !== scope) {
    pending.current.clear();
    selected.current = null;
    previousScope.current = scope;
  }
  return React.useMemo(
    () => ({
      capture: ({ channelId, rootEventId, messageIds }) => {
        if (!relayUrl || !pubkey) return;
        const now = Date.now();
        for (const [id, value] of pending.current) {
          if (now - value.capturedAt >= HANDOFF_TTL_MS)
            pending.current.delete(id);
        }
        rootEventId = rootEventId.toLowerCase();
        const id = `${channelId}|${rootEventId}`;
        selected.current = id;
        const existing = pending.current.get(id);
        if (existing) {
          for (const messageId of messageIds) {
            const normalizedId = messageId.toLowerCase();
            if (existing.snapshot.replyReadAt.has(normalizedId)) continue;
            existing.snapshot.replyReadAt.set(
              normalizedId,
              getOwnReadAt(msgContextKey(normalizedId)),
            );
            if (!isReadStateReady) existing.snapshot.repliesReady = false;
          }
          return;
        }
        if (pending.current.size >= MAX_HANDOFFS) {
          const oldest = pending.current.keys().next().value;
          if (oldest) pending.current.delete(oldest);
        }
        pending.current.set(id, {
          key: crypto.randomUUID(),
          capturedAt: now,
          snapshot: {
            channelId,
            rootEventId,
            channelReadAt: getOwnReadAt(channelId),
            channelReady: isReadStateReady,
            threadReadAt: getOwnReadAt(`${THREAD_PREFIX}${rootEventId}`),
            threadReady: isReadStateReady,
            replyReadAt: new Map(
              messageIds.map((id) => [
                id.toLowerCase(),
                getOwnReadAt(msgContextKey(id.toLowerCase())),
              ]),
            ),
            repliesReady: isReadStateReady,
          },
        });
      },
      peek: (channelId) => {
        const value = selected.current
          ? pending.current.get(selected.current)
          : null;
        return value &&
          value.snapshot.channelId === channelId &&
          Date.now() - value.capturedAt < HANDOFF_TTL_MS
          ? value
          : null;
      },
      consume: (key) => {
        for (const [id, value] of pending.current) {
          if (value.key !== key) continue;
          pending.current.delete(id);
          if (selected.current === id) selected.current = null;
          break;
        }
      },
    }),
    [relayUrl, pubkey, getOwnReadAt, isReadStateReady],
  );
}
