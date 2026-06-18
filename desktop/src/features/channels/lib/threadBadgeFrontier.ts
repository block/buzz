import type { TimelineMessage } from "@/features/messages/types";

// Decide the next value for a thread's badge frontier snapshot. The snapshot is
// seeded once at channel-open (reflecting "what was unread on open") and then
// advanced monotonically toward the live thread read marker as the user reads,
// so the badge clears after a read without waiting for channel re-entry.
//
// The advance target is ALWAYS the live marker (what the user actually
// consumed), never "latest reply": a subsequent reply newer than the marker
// re-raises the badge, and a collapsed-branch reply the marker never covered
// stays unread. Monotonic `Math.max` guards against a stale lower marker.
//
// Returns the value the snapshot should hold:
//   - `stored === undefined` (unseeded): seed to the live marker.
//   - otherwise: the greater of the stored snapshot and the live marker, where
//     `null` (never read) is the lowest possible frontier.
export function nextThreadBadgeFrontier(
  stored: number | null | undefined,
  liveMarker: number | null,
): number | null {
  if (stored === undefined) {
    return liveMarker;
  }
  if (liveMarker === null) {
    return stored;
  }
  if (stored === null) {
    return liveMarker;
  }
  return Math.max(stored, liveMarker);
}

// Seed/advance the per-root badge frontier snapshots for one channel, in place.
// Captures only top-level notified threads that have replies; each entry is
// seeded once at open then advanced toward the live marker on subsequent reads
// (see nextThreadBadgeFrontier). Called during render so snapshots reflect
// "what was unread on open," matching the openFrontierRef pattern.
export function seedThreadBadgeFrontiers(
  channelFrontiers: Map<string, number | null>,
  messages: TimelineMessage[],
  directRepliesByParentId: ReadonlyMap<string, TimelineMessage[]>,
  isNotified: (rootId: string) => boolean,
  getReadAt: (rootId: string) => number | null,
): void {
  for (const message of messages) {
    if (message.parentId) continue;
    if (!isNotified(message.id)) continue;
    if (!directRepliesByParentId.has(message.id)) continue;
    channelFrontiers.set(
      message.id,
      nextThreadBadgeFrontier(
        channelFrontiers.get(message.id),
        getReadAt(message.id),
      ),
    );
  }
}
