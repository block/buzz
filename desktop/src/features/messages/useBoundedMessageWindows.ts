import type { QueryCacheNotifyEvent, QueryClient } from "@tanstack/react-query";
import * as React from "react";

import { scheduleMessageWindowSweep } from "./lib/boundMessageWindows";

const BOUNDED_MESSAGE_KEYS = new Set([
  "channel-window",
  "channel-messages",
  "thread-replies",
]);

function isBoundedMessageEvent(queryKey: unknown): boolean {
  return (
    Array.isArray(queryKey) &&
    typeof queryKey[0] === "string" &&
    BOUNDED_MESSAGE_KEYS.has(queryKey[0])
  );
}

/**
 * Whether a cache event marks the release of a *fetch* pin — an observer
 * unmounting (channel switch) or a real fetch settling. A manual `setQueryData`
 * (`action.manual`) is excluded: it is the shape a live merge and the
 * optimistic pending-send drain both take, and firing a sweep on every live
 * merge would reintroduce the per-frame churn this cache bound removes. The
 * pending-send pin release is handled explicitly by the send mutation instead.
 */
function isFetchPinRelease(event: QueryCacheNotifyEvent): boolean {
  if (event.type === "observerRemoved") return true;
  return (
    event.type === "updated" &&
    (event.action.type === "error" ||
      (event.action.type === "success" && !event.action.manual))
  );
}

/**
 * Bound the number of channel timelines and thread subtrees retained in the
 * query cache. Every visited channel/thread lingers for `gcTime` (one hour)
 * after its view unmounts, so a session that roams many channels accumulates
 * every timeline's window store and flattened array at once — the renderer
 * memory leak behind the v0.5.9 lag. This mounts one cache subscription per
 * QueryClient that evicts the least-recently-updated unpinned timelines past a
 * fixed cap (see `enforceMessageWindowBounds`).
 *
 * Mounted once inside `CommunityQueryProvider`, so the bound applies to every
 * window that renders it — the main app, the huddle room, and the companion —
 * each on its own client.
 *
 * Two classes of event can leave a unit newly evictable and so must trigger a
 * sweep, both coalesced to one run per microtask via `scheduleMessageWindowSweep`:
 *   - `added`: a new bounded key can push the count over its cap. A channel
 *     open adds two keys at once; the coalescer runs the sweep once.
 *   - a pin release: an observer unmounting (`observerRemoved`, on channel
 *     switch) or a real fetch settling (`updated` with a non-manual `success`/
 *     `error` action — the fetch pin drops) makes a formerly-pinned unit
 *     evictable. Without this, a unit that went over cap while pinned would
 *     linger until the next unrelated `added` — the `added`-only gap Thufir
 *     flagged. The optimistic pending-send pin drains via a manual
 *     `setQueryData`, indistinguishable here from an ordinary live merge, so
 *     the send mutation calls `scheduleMessageWindowSweep` itself on success
 *     rather than being detected here — this keeps per-frame live merges (also
 *     manual updates) from triggering a sweep on every relay event.
 * Removals emit `removed`, never `added` or a pin release, so eviction cannot
 * re-trigger itself. The sweep runs synchronously in a single tick — collect
 * then remove — so a view mounting mid-sweep cannot be misread as inactive.
 */
export function useBoundedMessageWindows(queryClient: QueryClient): void {
  React.useEffect(() => {
    return queryClient.getQueryCache().subscribe((event) => {
      if (!isBoundedMessageEvent(event.query.queryKey)) return;
      if (event.type !== "added" && !isFetchPinRelease(event)) return;
      scheduleMessageWindowSweep(queryClient);
    });
  }, [queryClient]);
}
