/**
 * Read-your-writes routing for channel member lists. The relay serves
 * ordinary reads from a replica that can lag the writer, so a member-list
 * read made just after a membership change can return the list from before
 * it, and the roster cache keeps that result fresh for minutes. For a short
 * window after this client changes a channel's membership (or observes a
 * live membership change), every member-list read of that channel is routed
 * to the writer, so no refresh in that window, however it is triggered, can
 * land a pre-change list.
 */

/**
 * Just over the relay's hard staleness ceiling: a replica may serve a read
 * only while its last verified sync is within the configured budget, and the
 * budget is clamped to `FENCE_STALENESS` (30s, in
 * crates/buzz-db/src/runtime/replica_fence.rs). Past this, no supported
 * setting can return a pre-change list. Must move with `FENCE_STALENESS`.
 * Measured on the monotonic clock so a wall-clock jump can't shorten it.
 */
const WRITER_READ_WINDOW_MS = 31_000;

const writerReadDeadlines = new Map<string, number>();
let generation = 0;
const listeners = new Set<(channelId: string) => void>();

/**
 * Records a membership change for `channelId`: its member-list reads go to
 * the writer for the window, and listeners (the app's roster cache) refresh.
 */
export function noteChannelMembershipChange(channelId: string) {
  const now = performance.now();
  for (const [id, deadline] of writerReadDeadlines) {
    if (deadline <= now) writerReadDeadlines.delete(id);
  }
  writerReadDeadlines.set(channelId, now + WRITER_READ_WINDOW_MS);
  for (const listener of listeners) listener(channelId);
}

/**
 * Starts a membership write. Call before the write's first await; the
 * returned recorder notes a changed channel only if no community reset has
 * happened since, so a write that settles after a switch can't mark a
 * channel in the new community.
 */
export function beginChannelMembershipWrite(): (channelId: string) => void {
  const started = generation;
  return (channelId) => {
    if (started === generation) noteChannelMembershipChange(channelId);
  };
}

export function shouldReadChannelMembersFromWriter(channelId: string) {
  return (writerReadDeadlines.get(channelId) ?? 0) > performance.now();
}

/**
 * Subscribes to membership changes, first replaying every channel whose
 * window is still open so a change recorded before registration still
 * refreshes a roster cached before it.
 */
export function onChannelMembershipChange(
  listener: (channelId: string) => void,
): () => void {
  for (const channelId of writerReadDeadlines.keys()) {
    if (shouldReadChannelMembersFromWriter(channelId)) listener(channelId);
  }
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Community reset: changes recorded on the old relay don't apply to the new one. */
export function resetChannelMembershipWrites() {
  generation++;
  writerReadDeadlines.clear();
}
