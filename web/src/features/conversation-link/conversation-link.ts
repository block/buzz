/** Channel ids the Buzz apps accept in a conversation link. */
const CHANNEL_ID = /^[a-zA-Z0-9][a-zA-Z0-9._-]{0,255}$/;
/** A Nostr event id. */
const EVENT_ID = /^[0-9a-f]{64}$/i;

/**
 * The `buzz://` link an https conversation link stands for, or `null` when
 * its ids are malformed. Channels use `buzz://channel/<id>`; messages use the
 * canonical `buzz://message` form every Buzz client parses, with nothing
 * added, so a client that rejects unknown parameters still opens it.
 */
export function conversationAppLink({
  channelId,
  messageId,
  threadRootId,
}: {
  channelId: string;
  messageId?: string;
  threadRootId?: string | null;
}): string | null {
  if (!CHANNEL_ID.test(channelId)) return null;
  if (messageId === undefined)
    return threadRootId == null
      ? `buzz://channel/${encodeURIComponent(channelId)}`
      : null;
  if (!EVENT_ID.test(messageId)) return null;
  if (threadRootId != null && !EVENT_ID.test(threadRootId)) return null;
  const query = new URLSearchParams({
    channel: channelId,
    id: messageId.toLowerCase(),
  });
  if (threadRootId != null) query.set("thread", threadRootId.toLowerCase());
  return `buzz://message?${query}`;
}
