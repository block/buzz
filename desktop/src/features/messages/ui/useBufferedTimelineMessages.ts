import * as React from "react";

/**
 * Keeps the logical tail stable while a reader is away from the bottom.
 *
 * Virtua remains the sole owner of mounting, measurement, and pixel anchoring.
 * This hook only controls when live output joins its keyed data model: older
 * history before the retained tail is admitted immediately, while newer output
 * is released atomically when the reader returns to the bottom.
 */
export function selectBufferedTimelineMessages<T extends { id: string }>({
  frozenMessageIds,
  isAtBottom,
  messages,
}: {
  frozenMessageIds: readonly string[] | null;
  isAtBottom: boolean;
  messages: T[];
}): T[] {
  if (isAtBottom || frozenMessageIds === null) return messages;
  if (frozenMessageIds.length === 0) return [];

  const currentById = new Map(messages.map((message) => [message.id, message]));
  if (frozenMessageIds.some((id) => !currentById.has(id))) {
    // A deletion or authoritative replacement removed part of the frozen
    // snapshot. Keeping stale message objects would be worse than accepting it.
    return messages;
  }

  // Freeze a tail boundary, not membership. A reconnect can fill a gap
  // between retained rows (including an old live overlay); those are history,
  // not newer output, and must not vanish behind the live-message buffer.
  const tailIndex = messages.findIndex(
    (message) => message.id === frozenMessageIds[frozenMessageIds.length - 1],
  );
  return tailIndex === messages.length - 1
    ? messages
    : messages.slice(0, tailIndex + 1);
}

export function useBufferedTimelineMessages<T extends { id: string }>({
  channelId,
  isAtBottom,
  messages,
}: {
  channelId?: string | null;
  isAtBottom: boolean;
  messages: T[];
}): { messages: T[]; pendingCount: number } {
  const frozenMessageIdsRef = React.useRef<string[] | null>(null);
  const previousChannelIdRef = React.useRef(channelId);

  if (previousChannelIdRef.current !== channelId) {
    previousChannelIdRef.current = channelId;
    frozenMessageIdsRef.current = null;
  }

  if (isAtBottom) {
    frozenMessageIdsRef.current = messages.map((message) => message.id);
  } else if (frozenMessageIdsRef.current === null) {
    frozenMessageIdsRef.current = messages.map((message) => message.id);
  }

  const buffered = selectBufferedTimelineMessages({
    frozenMessageIds: frozenMessageIdsRef.current,
    isAtBottom,
    messages,
  });
  const previousBufferedRef = React.useRef<T[]>(buffered);
  const stableBuffered =
    previousBufferedRef.current.length === buffered.length &&
    previousBufferedRef.current.every(
      (message, index) => message === buffered[index],
    )
      ? previousBufferedRef.current
      : buffered;
  previousBufferedRef.current = stableBuffered;
  return {
    messages: stableBuffered,
    pendingCount: Math.max(0, messages.length - stableBuffered.length),
  };
}
