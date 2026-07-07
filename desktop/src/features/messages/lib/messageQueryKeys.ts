import type { RelayEvent } from "@/shared/api/types";

export function channelMessagesKey(channelId: string) {
  return ["channel-messages", channelId] as const;
}

export function channelWindowKey(channelId: string) {
  return ["channel-window", channelId] as const;
}

export function threadRepliesKey(channelId: string, rootId: string) {
  return ["thread-replies", channelId, rootId] as const;
}

export function dedupeMessagesById(messages: RelayEvent[]) {
  const seenIds = new Set<string>();
  const deduped: RelayEvent[] = [];

  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];

    if (seenIds.has(message.id)) {
      continue;
    }

    seenIds.add(message.id);
    deduped.push(message);
  }

  return deduped.reverse();
}

export function sortMessages(messages: RelayEvent[]) {
  return dedupeMessagesById(messages).sort((left, right) => {
    if (left.created_at !== right.created_at) {
      return left.created_at - right.created_at;
    }
    // Tiebreak same-second events on id so the merge order is deterministic.
    // Without this, two events sharing a created_at can land in a different
    // position depending on which REQ (history vs live-sub) delivered them
    // first — reading as a "missing"/shuffled message at a fixed scroll offset.
    return left.id < right.id ? -1 : left.id > right.id ? 1 : 0;
  });
}

export function normalizeTimelineMessages(messages: RelayEvent[]) {
  return sortMessages(messages);
}

/**
 * Merge a batch of events (older scrollback page, reconnect/revalidation
 * window, live gap-fill) into the timeline cache. Sort + dedupe only — the
 * {@link MAX_TIMELINE_MESSAGES} cap is deliberately NOT applied here.
 *
 * Capping merges into an already-painted timeline evicts history out from
 * under a reader: page back to an old day in a channel holding more than the
 * cap, and the next capped merge (live append, revalidation) silently deletes
 * the rows being read. The cap is applied only by
 * {@link normalizeTimelineMessages} at moments when nothing is rendered —
 * the cold-snapshot paint and the channel-leave trim in `hooks.ts`.
 */
export function mergeTimelineHistoryMessages(
  current: RelayEvent[],
  history: RelayEvent[],
) {
  return sortMessages([...current, ...history]);
}
