/**
 * Pure helpers that read a timeline message snapshot to compute the values the
 * timeline render needs: sticky-bottom autoscroll, day dividers, jump-to-message
 * deep links, and the deferred reply-list render state.
 *
 * Keeping these out of the component render body / scroll-manager effects lets
 * them be covered by the lib-level `*.test.mjs` suite. It also enforces the key
 * correctness property: every decision must read off the SAME snapshot. If the
 * deep-link lookup reads a fresher list than the rows the DOM has actually
 * committed, a jump fires against a row that isn't there yet and silently fails.
 */

import type { TimelineMessage } from "@/features/messages/types";
import { isSameDay, startOfLocalDaySeconds } from "./dateFormatters";

/** Distance (px) from the bottom within which the timeline counts as "at bottom". */
export const BOTTOM_THRESHOLD_PX = 72;

/** Minimal scroll geometry the sticky-bottom decision needs — a pure subset of the DOM element. */
export type ScrollMetrics = {
  scrollHeight: number;
  clientHeight: number;
  scrollTop: number;
};

/**
 * Is the timeline scrolled close enough to the bottom to count as "at bottom"?
 * Pure over geometry so the threshold math is testable without a DOM.
 */
export function isNearBottomMetrics(metrics: ScrollMetrics): boolean {
  return (
    metrics.scrollHeight - metrics.clientHeight - metrics.scrollTop <=
    BOTTOM_THRESHOLD_PX
  );
}

/** Reads live scroll geometry off a container and applies the bottom-threshold rule. */
export function isNearBottom(container: HTMLDivElement): boolean {
  return isNearBottomMetrics({
    scrollHeight: container.scrollHeight,
    clientHeight: container.clientHeight,
    scrollTop: container.scrollTop,
  });
}

/**
 * Identity of the last message in a snapshot, used to detect "a new latest
 * message arrived" for autoscroll. Prefers `renderKey` (stable across optimistic
 * send-ack) and falls back to `id`. Returns `undefined` for an empty snapshot.
 */
export function selectLatestMessageKey(
  messages: readonly TimelineMessage[],
): string | undefined {
  if (messages.length === 0) {
    return undefined;
  }
  const latest = messages[messages.length - 1];
  return latest.renderKey ?? latest.id;
}

export type LatestMessageAutoScrollBehavior = "auto" | "smooth" | null;

export function selectLatestMessageAutoScrollBehavior({
  hasExplicitBottomRequest,
  isAtBottom,
  shouldStickToBottom,
  targetMessageId,
}: {
  hasExplicitBottomRequest: boolean;
  isAtBottom: boolean;
  shouldStickToBottom: boolean;
  targetMessageId?: string | null;
}): LatestMessageAutoScrollBehavior {
  if (targetMessageId) {
    return null;
  }

  if (hasExplicitBottomRequest) {
    return "smooth";
  }

  if (shouldStickToBottom || isAtBottom) {
    return "auto";
  }

  return null;
}

/** A single day boundary in the timeline: where it starts and how many messages it covers. */
export type DayGroupBoundary = {
  /**
   * Stable key for the day section: the local start-of-day of the messages it
   * covers, so prepending an older message into an already-rendered day reuses
   * the same key instead of remounting the whole `<section>`.
   */
  key: string;
  /** Index into `messages` of the first message in this day. */
  startIndex: number;
  /** Number of messages in this day group. */
  count: number;
  /** The `createdAt` (unix seconds) used to render the heading label. */
  headingTimestamp: number;
};

/**
 * Walks a snapshot in order and produces the day-group boundaries. A new group
 * starts at index 0 and whenever a message falls on a different calendar day
 * than the one before it.
 */
export function buildDayGroupBoundaries(
  messages: readonly TimelineMessage[],
): DayGroupBoundary[] {
  const boundaries: DayGroupBoundary[] = [];

  for (let i = 0; i < messages.length; i++) {
    const message = messages[i];
    const prev = i > 0 ? messages[i - 1] : null;

    if (!prev || !isSameDay(prev.createdAt, message.createdAt)) {
      boundaries.push({
        key: `day-${startOfLocalDaySeconds(message.createdAt)}`,
        startIndex: i,
        count: 1,
        headingTimestamp: message.createdAt,
      });
    } else {
      boundaries[boundaries.length - 1].count += 1;
    }
  }

  return boundaries;
}

/** Outcome of resolving a deep-link target against the current snapshot. */
export type DeepLinkResolution = {
  /** Whether the target message exists in this snapshot (i.e. a row would be committed). */
  resolved: boolean;
  /** Index of the target in `messages`, or -1 when unresolved. */
  index: number;
};

/**
 * Does a jump-to-message target resolve against THIS snapshot? The scroll-manager
 * effect only does `querySelector` + `scrollIntoView` once a target row is
 * actually committed, so the jump must read the same snapshot the list rendered
 * — otherwise it scrolls to a row that isn't there yet.
 */
export function resolveDeepLinkTarget(
  messages: readonly TimelineMessage[],
  targetMessageId: string | null | undefined,
): DeepLinkResolution {
  if (!targetMessageId) {
    return { resolved: false, index: -1 };
  }
  const index = messages.findIndex((message) => message.id === targetMessageId);
  return { resolved: index !== -1, index };
}

/**
 * Which of three states a deferred list should paint. A list gated behind
 * `useDeferredValue` lags the live one for a frame, so the deferred snapshot can
 * be empty while the live list is not. Keying the empty state off the LIVE count
 * stops us flashing an "empty" affordance over a list that's streaming in:
 *
 *   - "list"    → the deferred snapshot has rows; paint them
 *   - "empty"   → the LIVE list is genuinely empty; paint the empty state
 *   - "pending" → deferred is empty but live has content; paint nothing yet
 */
export type DeferredListRenderState = "list" | "empty" | "pending";

export function selectDeferredListRenderState(
  deferredCount: number,
  liveCount: number,
): DeferredListRenderState {
  if (deferredCount > 0) {
    return "list";
  }
  if (liveCount === 0) {
    return "empty";
  }
  return "pending";
}

export type TimelineBodySurface = "skeleton" | "empty" | "list";

export function selectTimelineBodySurface({
  deferredCount,
  isLoading,
  liveCount,
}: {
  deferredCount: number;
  isLoading: boolean;
  liveCount: number;
}): TimelineBodySurface {
  if (isLoading) {
    return "skeleton";
  }

  const renderState = selectDeferredListRenderState(deferredCount, liveCount);
  if (renderState === "pending") {
    return "skeleton";
  }
  return renderState;
}

export type TimelineIntroSurface =
  | "direct-message-intro"
  | "channel-intro"
  | null;

export function selectTimelineIntroSurface({
  hasChannelIntro,
  hasDirectMessageIntro,
  isSkeletonVisible,
}: {
  hasChannelIntro: boolean;
  hasDirectMessageIntro: boolean;
  isSkeletonVisible: boolean;
}): TimelineIntroSurface {
  if (isSkeletonVisible) {
    return null;
  }
  if (hasDirectMessageIntro) {
    return "direct-message-intro";
  }
  if (hasChannelIntro) {
    return "channel-intro";
  }
  return null;
}
