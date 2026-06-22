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
  repliesByRootId?: ReadonlyMap<string, readonly ReplyGraphMessage[]>,
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
  // Orphan-immune ceiling: also fold in replies that resolve to this id by
  // rootId. When the timeline window drops a middle ancestor, a deep reply
  // keys under its absent parent and the adjacency walk above can't reach it,
  // so the root-started ceiling stops short and the channel-root badge can
  // never clear. rootId travels with the event (getThreadReference), so a root
  // reaches its severed orphans here. A BRANCH node is no reply's rootId, so
  // its rootId-bucket is empty and the branch-scoped ceiling is unchanged.
  for (const reply of repliesByRootId?.get(messageId) ?? []) {
    if (reply.createdAt > maxCreatedAt) {
      maxCreatedAt = reply.createdAt;
    }
  }
  return maxCreatedAt;
}

/** Minimal timeline shape the adjacency/createdAt builders read. */
interface ReplyGraphMessage {
  id: string;
  parentId?: string | null;
  rootId?: string | null;
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

/**
 * Maps each parent message id to its direct-reply objects in timeline order.
 * Built once so per-thread badge consumers resolve direct replies in O(1)
 * instead of re-scanning the whole timeline per top-level message.
 */
export function buildDirectRepliesByParentId<T extends ReplyGraphMessage>(
  messages: readonly T[],
): Map<string, T[]> {
  const map = new Map<string, T[]>();
  for (const message of messages) {
    if (!message.parentId) continue;
    const currentReplies = map.get(message.parentId) ?? [];
    currentReplies.push(message);
    map.set(message.parentId, currentReplies);
  }
  return map;
}

/**
 * Maps each thread root id to every reply that resolves to it by `rootId`,
 * in timeline order. Unlike the parent-keyed maps above, this groups by the
 * reply's own `rootId` (getThreadReference: the `root` e-tag that travels with
 * the event), so a deep reply lands under its true root even when an
 * intermediate ancestor is absent from the loaded window. Root-keyed badge
 * consumers use this to roll up severed orphans the parent-chain walk misses.
 * Top-level messages (no rootId) and self-referential roots are excluded.
 */
export function buildRepliesByRootId<T extends ReplyGraphMessage>(
  messages: readonly T[],
): Map<string, T[]> {
  const map = new Map<string, T[]>();
  for (const message of messages) {
    const rootId = message.rootId;
    if (!rootId || rootId === message.id) continue;
    const currentReplies = map.get(rootId) ?? [];
    currentReplies.push(message);
    map.set(rootId, currentReplies);
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
