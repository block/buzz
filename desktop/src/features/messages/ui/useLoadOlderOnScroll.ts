import * as React from "react";

type UseLoadOlderOnScrollOptions = {
  fetchOlder?: () => Promise<void>;
  hasOlderMessages: boolean;
  isLoading: boolean;
  scrollContainerRef: React.RefObject<HTMLDivElement | null>;
  sentinelRef: React.RefObject<HTMLDivElement | null>;
};

type ScrollAnchor = {
  id: string;
  top: number;
};

type ActiveScrollAnchorLock = {
  cleanup: () => void;
  restore: () => void;
  scheduleReleaseAfterQuietLayout: () => void;
  trackPendingImages: () => void;
};

function captureScrollAnchor(container: HTMLDivElement): ScrollAnchor | null {
  const containerRect = container.getBoundingClientRect();
  const messages = Array.from(
    container.querySelectorAll<HTMLElement>("[data-message-id]"),
  );

  for (const message of messages) {
    const rect = message.getBoundingClientRect();
    if (rect.bottom <= containerRect.top || rect.top >= containerRect.bottom) {
      continue;
    }

    return {
      id: message.dataset.messageId ?? "",
      top: rect.top - containerRect.top,
    };
  }

  return null;
}

function restoreScrollAnchor(
  container: HTMLDivElement,
  anchor: ScrollAnchor | null,
): number | null {
  if (!anchor?.id) {
    return null;
  }

  const message = container.querySelector<HTMLElement>(
    `[data-message-id="${CSS.escape(anchor.id)}"]`,
  );
  if (!message) {
    return null;
  }

  const currentTop =
    message.getBoundingClientRect().top - container.getBoundingClientRect().top;
  container.scrollTop += currentTop - anchor.top;
  return container.scrollTop;
}

/**
 * Triggers `fetchOlder` when a sentinel element near the top of the scroll
 * container enters the viewport, then keeps the viewport locked to the first
 * visible message until the prepended content and its media have settled.
 */
export function useLoadOlderOnScroll({
  fetchOlder,
  hasOlderMessages,
  isLoading,
  scrollContainerRef,
  sentinelRef,
}: UseLoadOlderOnScrollOptions) {
  const activeLockRef = React.useRef<ActiveScrollAnchorLock | null>(null);
  const loadStateRef = React.useRef({
    fetchOlder,
    hasOlderMessages,
    isLoading,
  });

  React.useEffect(() => {
    loadStateRef.current = { fetchOlder, hasOlderMessages, isLoading };
  }, [fetchOlder, hasOlderMessages, isLoading]);

  React.useLayoutEffect(() => {
    const lock = activeLockRef.current;
    if (!lock) {
      return;
    }

    lock.trackPendingImages();
    lock.restore();
    lock.scheduleReleaseAfterQuietLayout();
  });

  React.useEffect(() => {
    const sentinel = sentinelRef.current;
    const container = scrollContainerRef.current;
    if (!sentinel || !container) {
      return;
    }

    let disposed = false;
    let currentObserver: IntersectionObserver | null = null;

    const observe = () => {
      if (disposed) {
        return;
      }

      currentObserver = new IntersectionObserver(
        ([entry]) => {
          const { fetchOlder, hasOlderMessages, isLoading } =
            loadStateRef.current;
          if (
            !entry.isIntersecting ||
            disposed ||
            activeLockRef.current ||
            !fetchOlder ||
            isLoading ||
            !hasOlderMessages
          ) {
            return;
          }

          currentObserver?.disconnect();

          let anchor = captureScrollAnchor(container);
          let fetchSettled = false;
          let pendingImages = 0;
          let restoreFrame: number | null = null;
          let releaseTimer: number | null = null;
          let maxReleaseTimer: number | null = null;
          let resizeObserver: ResizeObserver | null = null;
          let mutationObserver: MutationObserver | null = null;
          let isRestoringAnchor = false;
          let lastRestoredScrollTop: number | null = null;
          const trackedImages = new WeakSet<HTMLImageElement>();
          const imageCleanups: Array<() => void> = [];

          const restoreAcrossFrames = (remainingFrames: number) => {
            isRestoringAnchor = true;
            lastRestoredScrollTop = restoreScrollAnchor(container, anchor);

            if (remainingFrames <= 0) {
              requestAnimationFrame(() => {
                isRestoringAnchor = false;
              });
              return;
            }

            restoreFrame = requestAnimationFrame(() => {
              restoreFrame = null;
              restoreAcrossFrames(remainingFrames - 1);
            });
          };

          const scheduleRestore = () => {
            if (restoreFrame !== null) {
              return;
            }

            restoreFrame = requestAnimationFrame(() => {
              restoreFrame = null;
              restoreAcrossFrames(2);
            });
          };

          const cleanupLock = () => {
            container.removeEventListener("scroll", updateAnchor);
            resizeObserver?.disconnect();
            mutationObserver?.disconnect();
            for (const cleanupImage of imageCleanups) {
              cleanupImage();
            }
            imageCleanups.length = 0;
            if (restoreFrame !== null) {
              cancelAnimationFrame(restoreFrame);
              restoreFrame = null;
            }
            if (releaseTimer !== null) {
              window.clearTimeout(releaseTimer);
              releaseTimer = null;
            }
            if (maxReleaseTimer !== null) {
              window.clearTimeout(maxReleaseTimer);
              maxReleaseTimer = null;
            }
            if (activeLockRef.current === lock) {
              activeLockRef.current = null;
            }
          };

          const releaseLock = () => {
            cleanupLock();
            observe();
          };

          const scheduleReleaseAfterQuietLayout = () => {
            if (!fetchSettled || pendingImages > 0) {
              return;
            }
            if (releaseTimer !== null) {
              window.clearTimeout(releaseTimer);
            }
            releaseTimer = window.setTimeout(releaseLock, 250);
          };

          const settleImage = () => {
            pendingImages = Math.max(0, pendingImages - 1);
            scheduleRestore();
            scheduleReleaseAfterQuietLayout();
          };

          const trackPendingImages = () => {
            const images = Array.from(container.querySelectorAll("img"));
            for (const image of images) {
              if (trackedImages.has(image)) {
                continue;
              }
              trackedImages.add(image);
              if (image.complete) {
                continue;
              }

              pendingImages += 1;
              image.addEventListener("load", settleImage, { once: true });
              image.addEventListener("error", settleImage, { once: true });
              imageCleanups.push(() => {
                image.removeEventListener("load", settleImage);
                image.removeEventListener("error", settleImage);
              });
            }
          };

          const updateAnchor = () => {
            if (
              isRestoringAnchor &&
              container.scrollTop === lastRestoredScrollTop
            ) {
              return;
            }
            anchor = captureScrollAnchor(container) ?? anchor;
          };

          const lock: ActiveScrollAnchorLock = {
            cleanup: cleanupLock,
            restore: () => {
              restoreAcrossFrames(2);
            },
            scheduleReleaseAfterQuietLayout,
            trackPendingImages,
          };

          container.addEventListener("scroll", updateAnchor, { passive: true });
          activeLockRef.current = lock;

          const content = container.firstElementChild;
          if (content instanceof HTMLElement) {
            if (typeof ResizeObserver !== "undefined") {
              resizeObserver = new ResizeObserver(() => {
                scheduleRestore();
                scheduleReleaseAfterQuietLayout();
              });
              resizeObserver.observe(content);
            }

            if (typeof MutationObserver !== "undefined") {
              mutationObserver = new MutationObserver(() => {
                trackPendingImages();
                scheduleRestore();
                scheduleReleaseAfterQuietLayout();
              });
              mutationObserver.observe(content, {
                childList: true,
                subtree: true,
              });
            }
          }

          void fetchOlder().finally(() => {
            fetchSettled = true;
            requestAnimationFrame(() => {
              if (disposed) {
                cleanupLock();
                return;
              }

              trackPendingImages();
              scheduleRestore();
              scheduleReleaseAfterQuietLayout();
              maxReleaseTimer = window.setTimeout(releaseLock, 10_000);
            });
          });
        },
        { root: container, rootMargin: "200px 0px 0px 0px" },
      );

      currentObserver.observe(sentinel);
    };

    observe();
    return () => {
      disposed = true;
      activeLockRef.current?.cleanup();
      currentObserver?.disconnect();
    };
  }, [scrollContainerRef, sentinelRef]);
}
