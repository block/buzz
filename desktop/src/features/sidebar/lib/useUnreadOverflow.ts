import * as React from "react";

type VisibilityEntry = {
  element: HTMLElement;
  isIntersecting: boolean;
};

type UnreadDirection = "above" | "below";

type UnreadOverflowCounts = {
  unreadAboveCount: number;
  unreadBelowCount: number;
  unreadAboveChannelIds: string[];
  unreadBelowChannelIds: string[];
};

const EMPTY_COUNTS: UnreadOverflowCounts = {
  unreadAboveCount: 0,
  unreadBelowCount: 0,
  unreadAboveChannelIds: [],
  unreadBelowChannelIds: [],
};

function getChannelId(element: Element): string | null {
  return element.getAttribute("data-channel-id");
}

function getUnreadElements(
  root: HTMLDivElement,
  unreadChannelIds: ReadonlySet<string>,
): HTMLElement[] {
  return Array.from(
    root.querySelectorAll<HTMLElement>("[data-channel-id]"),
  ).filter((element) => {
    const channelId = getChannelId(element);
    return channelId !== null && unreadChannelIds.has(channelId);
  });
}

function getRelativeTop(element: HTMLElement, root: HTMLDivElement): number {
  return element.getBoundingClientRect().top - root.getBoundingClientRect().top;
}

function findNextUnreadElement({
  direction,
  root,
  unreadChannelIds,
}: {
  direction: UnreadDirection;
  root: HTMLDivElement;
  unreadChannelIds: ReadonlySet<string>;
}): HTMLElement | null {
  const rootHeight = root.getBoundingClientRect().height;
  let nextElement: HTMLElement | null = null;
  let nextTop =
    direction === "above" ? Number.NEGATIVE_INFINITY : Number.POSITIVE_INFINITY;

  for (const element of getUnreadElements(root, unreadChannelIds)) {
    const top = getRelativeTop(element, root);

    if (direction === "above") {
      if (top < 0 && top > nextTop) {
        nextElement = element;
        nextTop = top;
      }
      continue;
    }

    if (top > rootHeight && top < nextTop) {
      nextElement = element;
      nextTop = top;
    }
  }

  return nextElement;
}

export function deriveUnreadOverflow(
  entries: Iterable<VisibilityEntry>,
  root: HTMLDivElement,
): UnreadOverflowCounts {
  const unreadAbove = new Map<string, number>();
  const unreadBelow = new Map<string, number>();
  const visibleChannelIds = new Set<string>();
  const rootHeight = root.getBoundingClientRect().height;

  for (const entry of entries) {
    const channelId = getChannelId(entry.element);
    if (!channelId) continue;
    if (entry.isIntersecting) {
      visibleChannelIds.add(channelId);
      continue;
    }

    const top = getRelativeTop(entry.element, root);

    if (top < 0) {
      unreadAbove.set(
        channelId,
        Math.max(unreadAbove.get(channelId) ?? -Infinity, top),
      );
    } else if (top > rootHeight) {
      unreadBelow.set(
        channelId,
        Math.min(unreadBelow.get(channelId) ?? Infinity, top),
      );
    }
  }

  for (const channelId of visibleChannelIds) {
    unreadAbove.delete(channelId);
    unreadBelow.delete(channelId);
  }

  const byPosition = (left: [string, number], right: [string, number]) =>
    left[1] - right[1];
  const unreadAboveChannelIds = [...unreadAbove.entries()]
    .sort((left, right) => byPosition(right, left))
    .map(([channelId]) => channelId);
  const unreadBelowChannelIds = [...unreadBelow.entries()]
    .sort(byPosition)
    .map(([channelId]) => channelId);

  return {
    unreadAboveCount: unreadAboveChannelIds.length,
    unreadBelowCount: unreadBelowChannelIds.length,
    unreadAboveChannelIds,
    unreadBelowChannelIds,
  };
}

export function useUnreadOverflow(args: {
  scrollRef: React.RefObject<HTMLDivElement | null>;
  unreadChannelIds: ReadonlySet<string>;
}): UnreadOverflowCounts & {
  scrollToChannel: (channelId: string) => void;
  scrollToNextAbove: () => void;
  scrollToNextBelow: () => void;
} {
  const { scrollRef, unreadChannelIds } = args;
  const unreadChannelIdsRef = React.useRef(unreadChannelIds);
  unreadChannelIdsRef.current = unreadChannelIds;

  const [counts, setCounts] = React.useState(EMPTY_COUNTS);

  React.useEffect(() => {
    const root = scrollRef.current;

    if (!root) {
      setCounts(EMPTY_COUNTS);
      return;
    }

    let intersectionObserver: IntersectionObserver | null = null;
    const visibilityByElement = new Map<HTMLElement, VisibilityEntry>();

    const updateCounts = () => {
      setCounts(deriveUnreadOverflow(visibilityByElement.values(), root));
    };

    const bindUnreadRows = () => {
      intersectionObserver?.disconnect();
      visibilityByElement.clear();

      intersectionObserver = new IntersectionObserver(
        (entries) => {
          for (const entry of entries) {
            const channelId = getChannelId(entry.target);
            if (!channelId || !unreadChannelIds.has(channelId)) {
              continue;
            }

            const element = entry.target as HTMLElement;
            visibilityByElement.set(element, {
              element,
              isIntersecting: entry.isIntersecting,
            });
          }

          updateCounts();
        },
        { root, threshold: 0 },
      );

      for (const element of getUnreadElements(root, unreadChannelIds)) {
        const channelId = getChannelId(element);
        if (!channelId) continue;

        visibilityByElement.set(element, {
          element,
          isIntersecting: false,
        });
        intersectionObserver.observe(element);
      }

      updateCounts();
    };

    const mutationObserver = new MutationObserver(bindUnreadRows);
    mutationObserver.observe(root, { childList: true, subtree: true });
    bindUnreadRows();

    return () => {
      intersectionObserver?.disconnect();
      mutationObserver.disconnect();
    };
  }, [scrollRef, unreadChannelIds]);

  const scrollToNextAbove = React.useCallback(() => {
    const root = scrollRef.current;
    if (!root) return;

    findNextUnreadElement({
      direction: "above",
      root,
      unreadChannelIds: unreadChannelIdsRef.current,
    })?.scrollIntoView({ behavior: "smooth", block: "center" });
  }, [scrollRef]);

  const scrollToNextBelow = React.useCallback(() => {
    const root = scrollRef.current;
    if (!root) return;

    findNextUnreadElement({
      direction: "below",
      root,
      unreadChannelIds: unreadChannelIdsRef.current,
    })?.scrollIntoView({ behavior: "smooth", block: "center" });
  }, [scrollRef]);

  const scrollToChannel = React.useCallback(
    (channelId: string) => {
      const root = scrollRef.current;
      if (!root) return;
      getUnreadElements(root, unreadChannelIdsRef.current)
        .find((element) => getChannelId(element) === channelId)
        ?.scrollIntoView({ behavior: "smooth", block: "center" });
    },
    [scrollRef],
  );

  return {
    ...counts,
    scrollToChannel,
    scrollToNextAbove,
    scrollToNextBelow,
  };
}
