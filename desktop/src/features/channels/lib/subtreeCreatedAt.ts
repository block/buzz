/**
 * Reply-graph builders for the per-message thread badge model (LP4 v3). Each
 * maps the loaded timeline into an index a badge consumer reads in O(1): direct
 * children by parent, replies by their resolved thread root, createdAt by id,
 * and the descendant id walk. The old `subtreeMaxCreatedAt` frontier-advance
 * helper is gone — read state is now per-message (`effective(msg:<id>)`), so no
 * subtree ceiling is computed.
 */

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
