import * as React from "react";

const TOP_ANCHOR_OFFSET = 12;

type VisibleMessageAnchor = {
  id: string;
  topOffset: number;
};

type PendingPrependRestore = {
  anchor: VisibleMessageAnchor | null;
  previousFirstMessageId: string | null;
  previousMessageCount: number;
  previousScrollHeight: number;
};

type UseLoadOlderOnScrollOptions = {
  fetchOlder?: () => Promise<void>;
  hasOlderMessages: boolean;
  isLoading: boolean;
  renderedFirstMessageId: string | null;
  renderedMessageCount: number;
  restoreScrollBy: (delta: number) => void;
  scrollContainerRef: React.RefObject<HTMLDivElement | null>;
  sentinelRef: React.RefObject<HTMLDivElement | null>;
};

function findVisibleMessageAnchor(
  container: HTMLDivElement,
): VisibleMessageAnchor | null {
  const containerRect = container.getBoundingClientRect();
  const preferredTop = Math.min(TOP_ANCHOR_OFFSET, containerRect.height / 2);
  let bestAnchor: (VisibleMessageAnchor & { distance: number }) | null = null;

  for (const row of Array.from(
    container.querySelectorAll<HTMLElement>("[data-message-id]"),
  )) {
    const rect = row.getBoundingClientRect();
    if (rect.bottom <= containerRect.top || rect.top >= containerRect.bottom) {
      continue;
    }

    const id = row.dataset.messageId;
    if (!id) {
      continue;
    }

    const topOffset = rect.top - containerRect.top;
    const distance = Math.abs(topOffset - preferredTop);
    if (!bestAnchor || distance < bestAnchor.distance) {
      bestAnchor = { distance, id, topOffset };
    }
  }

  return bestAnchor
    ? { id: bestAnchor.id, topOffset: bestAnchor.topOffset }
    : null;
}

/**
 * Triggers `fetchOlder` when a sentinel element near the top of the scroll
 * container enters the viewport, then restores the scroll position so the
 * visible message row doesn't jump when older rows are prepended.
 */
export function useLoadOlderOnScroll({
  fetchOlder,
  hasOlderMessages,
  isLoading,
  renderedFirstMessageId,
  renderedMessageCount,
  restoreScrollBy,
  scrollContainerRef,
  sentinelRef,
}: UseLoadOlderOnScrollOptions) {
  const fetchOlderRef = React.useRef(fetchOlder);
  const pendingRestoreRef = React.useRef<PendingPrependRestore | null>(null);
  const renderedFirstMessageIdRef = React.useRef(renderedFirstMessageId);
  const renderedMessageCountRef = React.useRef(renderedMessageCount);
  const restoreScrollByRef = React.useRef(restoreScrollBy);
  const resumeObservingRef = React.useRef<(() => void) | null>(null);

  React.useEffect(() => {
    fetchOlderRef.current = fetchOlder;
  });

  React.useEffect(() => {
    renderedFirstMessageIdRef.current = renderedFirstMessageId;
    renderedMessageCountRef.current = renderedMessageCount;
  });

  React.useEffect(() => {
    restoreScrollByRef.current = restoreScrollBy;
  });

  React.useLayoutEffect(() => {
    const pending = pendingRestoreRef.current;
    if (!pending) {
      return;
    }

    const hasCommittedPrepend =
      renderedFirstMessageId !== pending.previousFirstMessageId ||
      renderedMessageCount > pending.previousMessageCount;
    if (!hasCommittedPrepend) {
      return;
    }

    pendingRestoreRef.current = null;

    const container = scrollContainerRef.current;
    if (!container) {
      resumeObservingRef.current?.();
      return;
    }

    if (pending.anchor) {
      const anchor = container.querySelector<HTMLElement>(
        `[data-message-id="${CSS.escape(pending.anchor.id)}"]`,
      );
      if (anchor) {
        const nextAnchorOffset =
          anchor.getBoundingClientRect().top -
          container.getBoundingClientRect().top;
        restoreScrollByRef.current(nextAnchorOffset - pending.anchor.topOffset);
        resumeObservingRef.current?.();
        return;
      }
    }

    const scrollHeightDelta =
      container.scrollHeight - pending.previousScrollHeight;
    if (scrollHeightDelta > 0) {
      restoreScrollByRef.current(scrollHeightDelta);
    }
    resumeObservingRef.current?.();
  }, [renderedFirstMessageId, renderedMessageCount, scrollContainerRef]);

  React.useEffect(() => {
    const sentinel = sentinelRef.current;
    const container = scrollContainerRef.current;
    if (
      !sentinel ||
      !container ||
      !fetchOlder ||
      isLoading ||
      !hasOlderMessages
    ) {
      return;
    }

    let disposed = false;
    let currentObserver: IntersectionObserver | null = null;

    const observe = () => {
      if (disposed || pendingRestoreRef.current) {
        return;
      }

      currentObserver = new IntersectionObserver(
        ([entry]) => {
          if (!entry.isIntersecting || disposed || pendingRestoreRef.current) {
            return;
          }

          currentObserver?.disconnect();
          pendingRestoreRef.current = {
            anchor: findVisibleMessageAnchor(container),
            previousFirstMessageId: renderedFirstMessageIdRef.current,
            previousMessageCount: renderedMessageCountRef.current,
            previousScrollHeight: container.scrollHeight,
          };

          void fetchOlderRef.current?.().catch(() => {
            pendingRestoreRef.current = null;
            observe();
          });
        },
        { root: container, rootMargin: "200px 0px 0px 0px" },
      );

      currentObserver.observe(sentinel);
    };

    resumeObservingRef.current = observe;
    observe();
    return () => {
      disposed = true;
      resumeObservingRef.current = null;
      currentObserver?.disconnect();
    };
  }, [
    fetchOlder,
    hasOlderMessages,
    isLoading,
    scrollContainerRef,
    sentinelRef,
  ]);
}
