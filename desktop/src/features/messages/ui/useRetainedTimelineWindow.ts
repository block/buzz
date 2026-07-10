import * as React from "react";

export const RETAINED_TIMELINE_WINDOW_SIZE = 250;
export const RETAINED_TIMELINE_SHIFT_THRESHOLD = 50;

type MessageLike = { id: string };
export type RetainedTimelineWindowRange = {
  channelId: string | null;
  firstId: string | null;
  followLatest: boolean;
  lastId: string | null;
  sourceFirstId: string | null;
  sourceLastId: string | null;
};
type PendingAnchor = { messageId: string; topOffset: number };

export function retainedWindowBounds(
  messages: readonly MessageLike[],
  range: RetainedTimelineWindowRange,
): { start: number; end: number } {
  const count = messages.length;
  if (count <= RETAINED_TIMELINE_WINDOW_SIZE) return { start: 0, end: count };

  const latestStart = count - RETAINED_TIMELINE_WINDOW_SIZE;
  const prepended =
    range.sourceFirstId !== (messages[0]?.id ?? null) &&
    range.sourceLastId === (messages[count - 1]?.id ?? null) &&
    range.sourceFirstId !== null;
  if (range.followLatest && !prepended) {
    return { start: latestStart, end: count };
  }

  const firstIndex = range.firstId
    ? messages.findIndex((message) => message.id === range.firstId)
    : -1;
  const lastIndex = range.lastId
    ? messages.findIndex((message) => message.id === range.lastId)
    : -1;
  if (firstIndex >= 0 && lastIndex >= firstIndex) {
    return {
      start: firstIndex,
      end: Math.min(count, firstIndex + RETAINED_TIMELINE_WINDOW_SIZE),
    };
  }
  if (lastIndex >= 0) {
    return {
      start: Math.max(0, lastIndex - RETAINED_TIMELINE_WINDOW_SIZE + 1),
      end: lastIndex + 1,
    };
  }

  // A replaced/deleted boundary must never strand the reader in an empty
  // window. Falling back to the newest retained page is the least surprising
  // recovery and matches channel-open behavior.
  return { start: latestStart, end: count };
}

function rangeForBounds(
  channelId: string | null | undefined,
  messages: readonly MessageLike[],
  start: number,
  end: number,
  followLatest = end === messages.length,
): RetainedTimelineWindowRange {
  return {
    channelId: channelId ?? null,
    firstId: messages[start]?.id ?? null,
    followLatest,
    lastId: messages[end - 1]?.id ?? null,
    sourceFirstId: messages[0]?.id ?? null,
    sourceLastId: messages[messages.length - 1]?.id ?? null,
  };
}

function captureVisibleAnchor(container: HTMLElement): PendingAnchor | null {
  const containerTop = container.getBoundingClientRect().top;
  for (const row of container.querySelectorAll<HTMLElement>(
    "[data-message-id]",
  )) {
    const rect = row.getBoundingClientRect();
    if (rect.bottom > containerTop) {
      const messageId = row.dataset.messageId;
      if (messageId) return { messageId, topOffset: rect.top - containerTop };
    }
  }
  return null;
}

export function useRetainedTimelineWindow({
  channelId,
  focusMessageId,
  messages,
  scrollContainerRef,
}: {
  channelId?: string | null;
  focusMessageId?: string | null;
  messages: readonly MessageLike[];
  scrollContainerRef: React.RefObject<HTMLElement | null>;
}) {
  const initialFocusIndex = focusMessageId
    ? messages.findIndex((message) => message.id === focusMessageId)
    : -1;
  const initialStart =
    initialFocusIndex >= 0
      ? Math.max(
          0,
          Math.min(
            Math.max(0, messages.length - RETAINED_TIMELINE_WINDOW_SIZE),
            initialFocusIndex - Math.floor(RETAINED_TIMELINE_WINDOW_SIZE / 2),
          ),
        )
      : Math.max(0, messages.length - RETAINED_TIMELINE_WINDOW_SIZE);
  const initialEnd = Math.min(
    messages.length,
    initialStart + RETAINED_TIMELINE_WINDOW_SIZE,
  );
  const [range, setRange] = React.useState<RetainedTimelineWindowRange>(() =>
    rangeForBounds(channelId, messages, initialStart, initialEnd),
  );
  const pendingAnchorRef = React.useRef<PendingAnchor | null>(null);
  const [renderVersion, bumpRenderVersion] = React.useReducer(
    (version: number) => version + 1,
    0,
  );

  const effectiveRange =
    range.channelId === (channelId ?? null)
      ? range
      : rangeForBounds(
          channelId,
          messages,
          Math.max(0, messages.length - RETAINED_TIMELINE_WINDOW_SIZE),
          messages.length,
        );
  const { start, end } = retainedWindowBounds(messages, effectiveRange);

  React.useEffect(() => {
    if (range.channelId !== (channelId ?? null)) setRange(effectiveRange);
  }, [channelId, effectiveRange, range.channelId]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: start/end are the transaction commit signal; the effect intentionally restores after the retained range changes.
  React.useLayoutEffect(() => {
    const anchor = pendingAnchorRef.current;
    if (!anchor) return;
    pendingAnchorRef.current = null;
    const container = scrollContainerRef.current;
    const row = container?.querySelector<HTMLElement>(
      `[data-message-id="${CSS.escape(anchor.messageId)}"]`,
    );
    if (!container || !row) return;
    const currentOffset =
      row.getBoundingClientRect().top - container.getBoundingClientRect().top;
    const drift = currentOffset - anchor.topOffset;
    if (Math.abs(drift) > 0.5) container.scrollBy(0, drift);
  }, [end, scrollContainerRef, start]);

  const setBounds = React.useCallback(
    (nextStart: number, nextEnd: number, preserveAnchor: boolean) => {
      const container = scrollContainerRef.current;
      if (preserveAnchor && container) {
        pendingAnchorRef.current = captureVisibleAnchor(container);
      }
      setRange(
        rangeForBounds(
          channelId,
          messages,
          nextStart,
          nextEnd,
          nextEnd === messages.length && !preserveAnchor,
        ),
      );
      bumpRenderVersion();
    },
    [channelId, messages, scrollContainerRef],
  );

  const onScroll = React.useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container) return;
    const atPhysicalBottom =
      container.scrollHeight - container.scrollTop - container.clientHeight <=
      32;
    if (effectiveRange.followLatest !== atPhysicalBottom) {
      setRange(
        rangeForBounds(channelId, messages, start, end, atPhysicalBottom),
      );
    }
    if (messages.length <= RETAINED_TIMELINE_WINDOW_SIZE) return;
    const anchor = captureVisibleAnchor(container);
    if (!anchor) return;
    const visibleIndex = messages.findIndex(
      (message) => message.id === anchor.messageId,
    );
    if (visibleIndex < 0) return;

    if (start > 0 && visibleIndex - start < RETAINED_TIMELINE_SHIFT_THRESHOLD) {
      const nextStart = Math.max(
        0,
        visibleIndex - Math.floor(RETAINED_TIMELINE_WINDOW_SIZE / 2),
      );
      setBounds(
        nextStart,
        Math.min(messages.length, nextStart + RETAINED_TIMELINE_WINDOW_SIZE),
        true,
      );
    } else if (
      end < messages.length &&
      end - visibleIndex <= RETAINED_TIMELINE_SHIFT_THRESHOLD
    ) {
      const nextEnd = Math.min(
        messages.length,
        visibleIndex + Math.floor(RETAINED_TIMELINE_WINDOW_SIZE / 2),
      );
      setBounds(
        Math.max(0, nextEnd - RETAINED_TIMELINE_WINDOW_SIZE),
        nextEnd,
        true,
      );
    }
  }, [
    channelId,
    effectiveRange.followLatest,
    end,
    messages,
    scrollContainerRef,
    setBounds,
    start,
  ]);

  const ensureMessage = React.useCallback(
    (messageId: string): boolean => {
      const index = messages.findIndex((message) => message.id === messageId);
      if (index < 0) return false;
      if (index >= start && index < end) return true;
      const nextStart = Math.max(
        0,
        Math.min(
          messages.length - RETAINED_TIMELINE_WINDOW_SIZE,
          index - Math.floor(RETAINED_TIMELINE_WINDOW_SIZE / 2),
        ),
      );
      setBounds(
        nextStart,
        Math.min(messages.length, nextStart + RETAINED_TIMELINE_WINDOW_SIZE),
        false,
      );
      return false;
    },
    [end, messages, setBounds, start],
  );

  return {
    end,
    ensureMessage,
    includesStart: start === 0,
    onScroll,
    renderVersion,
    start,
  };
}
