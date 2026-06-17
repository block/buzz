/**
 * Newest `createdAt` across a thread branch: the message itself plus every
 * descendant, walked through the direct-children adjacency map. Drilling into a
 * branch advances the thread read frontier to this value, so it determines how
 * far "expanding consumes unread" reaches. Returns null when the message is
 * absent from the timeline so the caller can skip the read-state write.
 */
export function subtreeMaxCreatedAt(
  messageId: string,
  directReplyIdsByParentId: ReadonlyMap<string, string[]>,
  createdAtByMessageId: ReadonlyMap<string, number>,
): number | null {
  const ownCreatedAt = createdAtByMessageId.get(messageId);
  if (ownCreatedAt === undefined) return null;

  let maxCreatedAt = ownCreatedAt;
  const pendingIds = [...(directReplyIdsByParentId.get(messageId) ?? [])];
  while (pendingIds.length > 0) {
    const currentId = pendingIds.pop();
    if (!currentId) continue;
    const createdAt = createdAtByMessageId.get(currentId);
    if (createdAt !== undefined && createdAt > maxCreatedAt) {
      maxCreatedAt = createdAt;
    }
    pendingIds.push(...(directReplyIdsByParentId.get(currentId) ?? []));
  }
  return maxCreatedAt;
}

/**
 * Newest `createdAt` across a thread head and its DIRECT replies only — the
 * content visible the instant the panel opens, before any branch is expanded.
 * Opening a thread advances the read frontier to this, mirroring channel-open
 * parity: you see (and thus consume) the top-level replies on open, while
 * deeper collapsed branches stay unread until drilled into. Returns null when
 * the head is absent so the caller can skip the read-state write.
 */
export function directRepliesMaxCreatedAt(
  messageId: string,
  directReplyIdsByParentId: ReadonlyMap<string, string[]>,
  createdAtByMessageId: ReadonlyMap<string, number>,
): number | null {
  const ownCreatedAt = createdAtByMessageId.get(messageId);
  if (ownCreatedAt === undefined) return null;

  let maxCreatedAt = ownCreatedAt;
  for (const replyId of directReplyIdsByParentId.get(messageId) ?? []) {
    const createdAt = createdAtByMessageId.get(replyId);
    if (createdAt !== undefined && createdAt > maxCreatedAt) {
      maxCreatedAt = createdAt;
    }
  }
  return maxCreatedAt;
}

/** Minimal timeline shape the adjacency/createdAt builders read. */
interface ReplyGraphMessage {
  id: string;
  parentId?: string | null;
  createdAt: number;
}

/** Maps each parent message id to its direct-reply ids in timeline order. */
export function buildDirectReplyIdsByParentId(
  messages: readonly ReplyGraphMessage[],
): Map<string, string[]> {
  const map = new Map<string, string[]>();
  for (const message of messages) {
    if (!message.parentId) continue;
    const currentReplies = map.get(message.parentId) ?? [];
    currentReplies.push(message.id);
    map.set(message.parentId, currentReplies);
  }
  return map;
}

/** Maps each message id to its `createdAt`. */
export function buildCreatedAtByMessageId(
  messages: readonly ReplyGraphMessage[],
): Map<string, number> {
  const map = new Map<string, number>();
  for (const message of messages) {
    map.set(message.id, message.createdAt);
  }
  return map;
}

/** Every descendant reply id under a message, walked breadth-first. */
export function collectReplyDescendantIds(
  messageId: string,
  directReplyIdsByParentId: ReadonlyMap<string, string[]>,
): string[] {
  const descendantIds: string[] = [];
  const pendingIds = [...(directReplyIdsByParentId.get(messageId) ?? [])];
  while (pendingIds.length > 0) {
    const currentId = pendingIds.pop();
    if (!currentId) continue;
    descendantIds.push(currentId);
    pendingIds.push(...(directReplyIdsByParentId.get(currentId) ?? []));
  }
  return descendantIds;
}
