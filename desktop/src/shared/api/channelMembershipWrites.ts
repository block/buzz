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

/** Comfortably above the relay's replica-lag ceiling (1s in production). */
const WRITER_READ_WINDOW_MS = 5_000;

const writerReadDeadlines = new Map<string, number>();
const listeners = new Set<(channelId: string) => void>();

/**
 * Records a membership change for `channelId`: its member-list reads go to
 * the writer for the window, and listeners (the app's roster cache) refresh.
 */
export function noteChannelMembershipChange(channelId: string) {
  const now = Date.now();
  for (const [id, deadline] of writerReadDeadlines) {
    if (deadline <= now) writerReadDeadlines.delete(id);
  }
  writerReadDeadlines.set(channelId, now + WRITER_READ_WINDOW_MS);
  for (const listener of listeners) listener(channelId);
}

export function shouldReadChannelMembersFromWriter(channelId: string) {
  return (writerReadDeadlines.get(channelId) ?? 0) > Date.now();
}

export function onChannelMembershipChange(
  listener: (channelId: string) => void,
): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
