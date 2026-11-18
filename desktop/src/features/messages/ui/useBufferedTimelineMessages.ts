import * as React from "react";

type TimelineRow = { id: string; renderKey?: string };

/** A row's identity, stable across an optimistic send and its relay ack. */
function rowKey(message: TimelineRow) {
  return message.renderKey ?? message.id;
}

/**
 * Keeps the logical tail stable while a reader is away from the bottom.
 *
 * Virtua remains the sole owner of mounting, measurement, and pixel anchoring.
 * This hook only controls when live output joins its keyed data model: older
 * history before the retained tail is admitted immediately, while newer output
 * is released atomically when the reader returns to the bottom.
 */
export function selectBufferedTimelineMessages<T extends TimelineRow>({
  frozenRowKeys,
  isAtBottom,
  messages,
}: {
  frozenRowKeys: readonly string[] | null;
  isAtBottom: boolean;
  messages: T[];
}): T[] {
  if (isAtBottom || frozenRowKeys === null) return messages;
  if (frozenRowKeys.length === 0) return [];

  const currentByKey = new Map(
    messages.map((message) => [rowKey(message), message]),
  );
  // A deletion drops only its own row: the rest stays frozen, so later
  // arrivals still count as pending. Once the whole snapshot is gone (an
  // authoritative replacement), accept the new list rather than keep stale
  // message objects.
  const survivingKeys = frozenRowKeys.filter((key) => currentByKey.has(key));
  if (survivingKeys.length === 0) return messages;

  // Frozen rows can come back reordered: an acked send changes its id and so
  // its place among same-second rows. Older history ends at the first frozen
  // row of any key, so no frozen row is also prepended and counted twice.
  const frozenKeys = new Set(survivingKeys);
  const firstFrozenIndex = messages.findIndex((message) =>
    frozenKeys.has(rowKey(message)),
  );
  const prepended = messages.slice(0, firstFrozenIndex);
  const frozen = survivingKeys.map((key) => currentByKey.get(key) as T);
  const buffered = [...prepended, ...frozen];
  if (
    buffered.length === messages.length &&
    buffered.every((message, index) => message.id === messages[index]?.id)
  ) {
    // Crossing the bottom threshold without a live arrival must be a semantic
    // no-op for Virtua. Preserve the source array identity until there is
    // actually something to buffer; otherwise the threshold transition can
    // rebuild its model while a prepend is starting.
    return messages;
  }
  return buffered;
}

export function useBufferedTimelineMessages<T extends TimelineRow>({
  channelId,
  isAtBottom,
  messages,
}: {
  channelId?: string | null;
  isAtBottom: boolean;
  messages: T[];
}): { messages: T[]; pendingCount: number } {
  const frozenRowKeysRef = React.useRef<string[] | null>(null);
  const previousChannelIdRef = React.useRef(channelId);

  if (previousChannelIdRef.current !== channelId) {
    previousChannelIdRef.current = channelId;
    frozenRowKeysRef.current = null;
  }

  if (isAtBottom) {
    frozenRowKeysRef.current = messages.map(rowKey);
  } else if (frozenRowKeysRef.current === null) {
    frozenRowKeysRef.current = messages.map(rowKey);
  }

  const buffered = selectBufferedTimelineMessages({
    frozenRowKeys: frozenRowKeysRef.current,
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
