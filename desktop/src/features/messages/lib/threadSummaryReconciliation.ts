import type { QueryClient } from "@tanstack/react-query";

import { channelWindowKey, threadRepliesKey } from "./messageQueryKeys";
import {
  emptyChannelWindowStore,
  mergeLiveThreadSummary,
  type ChannelWindowStore,
} from "./channelWindowStore";
import { parseLiveThreadSummary } from "./channelWindowResponse";
import { CHANNEL_TIMELINE_CONTENT_KINDS } from "@/shared/constants/kinds";
import type { RelayEvent } from "@/shared/api/types";

const THREAD_CONTENT_KINDS = new Set<number>(CHANNEL_TIMELINE_CONTENT_KINDS);

/**
 * Whether an authoritative relay summary disagrees with an already-loaded
 * thread cache. Auxiliary events and optimistic rows do not contribute to the
 * relay's committed descendant count.
 */
export function shouldInvalidateThreadReplies(
  cachedReplies: RelayEvent[] | undefined,
  descendantCount: number,
): boolean {
  if (!cachedReplies) return false;

  const committedContentCount = cachedReplies.reduce(
    (count, event) =>
      !event.pending && THREAD_CONTENT_KINDS.has(event.kind)
        ? count + 1
        : count,
    0,
  );

  return committedContentCount !== descendantCount;
}

/**
 * Apply one relay-pushed summary to the channel window and reconcile an
 * already-loaded thread cache against its authoritative descendant count.
 */
export function reconcileLiveThreadSummary(
  queryClient: Pick<
    QueryClient,
    "getQueryData" | "setQueryData" | "invalidateQueries"
  >,
  channelId: string,
  event: RelayEvent,
): boolean {
  const parsed = parseLiveThreadSummary(event);
  if (!parsed) return false;

  const windowKey = channelWindowKey(channelId);
  const current =
    queryClient.getQueryData<ChannelWindowStore>(windowKey) ??
    emptyChannelWindowStore();
  const existing = current.liveSummaries[parsed.rootId];
  const next = mergeLiveThreadSummary(current, parsed.rootId, parsed.live);
  const accepted =
    next !== current || existing?.eventId === parsed.live.eventId;
  if (!accepted) return false;

  if (next !== current) queryClient.setQueryData(windowKey, next);

  const threadKey = threadRepliesKey(channelId, parsed.rootId);
  const cachedReplies = queryClient.getQueryData<RelayEvent[]>(threadKey);
  if (
    shouldInvalidateThreadReplies(
      cachedReplies,
      parsed.live.summary.descendantCount,
    )
  ) {
    void queryClient.invalidateQueries({ queryKey: threadKey, exact: true });
  }

  return true;
}
