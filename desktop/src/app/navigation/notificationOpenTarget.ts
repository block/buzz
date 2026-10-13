// "Opened from a notification" flag for route targets. Notification activation marks the target message id; the
// channel route consumes it so a top-level message opens the channel scrolled to + highlighting the message, without
// opening its reply panel (thread replies still open their thread). Search and in-app links don't set it.
const marked = new Map<string, number>();
const TTL_MS = 60_000;

export function markNotificationOpenTarget(
  messageId: string | null | undefined,
) {
  if (!messageId) return;
  const now = Date.now();
  for (const [id, at] of marked) if (now - at > TTL_MS) marked.delete(id);
  marked.set(messageId, now);
}

/** True (once) if this route target came from a notification tap. */
export function consumeNotificationOpenTarget(messageId: string): boolean {
  const at = marked.get(messageId);
  if (at === undefined) return false;
  marked.delete(messageId);
  return Date.now() - at <= TTL_MS;
}
