import {
  reactionsEqual,
  tagsEqual,
} from "@/features/messages/lib/messageRowEquality";
import type { TimelineMessage } from "@/features/messages/types";

/**
 * Value equality for a formatted timeline row. Used to structure-share
 * `TimelineMessage` objects across `formatTimelineMessages` runs so
 * `MessageRow` / markdown memos and WeakMap caches keyed on message identity
 * (thread depth normalization, video-review contexts) can hit when only a
 * sibling row changed.
 */
export function timelineMessagesEqual(
  a: TimelineMessage,
  b: TimelineMessage,
): boolean {
  if (a === b) return true;
  return (
    a.id === b.id &&
    a.renderKey === b.renderKey &&
    a.createdAt === b.createdAt &&
    a.pubkey === b.pubkey &&
    a.signerPubkey === b.signerPubkey &&
    a.author === b.author &&
    a.isAgent === b.isAgent &&
    a.ownerPubkey === b.ownerPubkey &&
    a.ownerLabel === b.ownerLabel &&
    a.avatarUrl === b.avatarUrl &&
    a.role === b.role &&
    a.personaDisplayName === b.personaDisplayName &&
    a.respondTo === b.respondTo &&
    a.time === b.time &&
    a.body === b.body &&
    a.parentId === b.parentId &&
    a.rootId === b.rootId &&
    a.depth === b.depth &&
    a.accent === b.accent &&
    a.pending === b.pending &&
    a.edited === b.edited &&
    a.highlighted === b.highlighted &&
    a.kind === b.kind &&
    tagsEqual(a.tags, b.tags) &&
    reactionsEqual(a.reactions, b.reactions)
  );
}

/**
 * Reuse previous `TimelineMessage` object identities wherever the newly
 * formatted row is value-equal. When every element matches (same length and
 * order), returns the previous array reference so parent `React.memo` /
 * `useMemo` deps can bail without a deep walk.
 *
 * Fast path: live appends keep the prior prefix in order — reuse those refs
 * without building an id map.
 */
export function structureShareTimelineMessages(
  previous: readonly TimelineMessage[] | undefined,
  next: TimelineMessage[],
): TimelineMessage[] {
  if (!previous || previous.length === 0) {
    return next;
  }

  const prevLen = previous.length;
  const nextLen = next.length;
  if (nextLen >= prevLen) {
    let prefixMatches = true;
    for (let i = 0; i < prevLen; i += 1) {
      if (!timelineMessagesEqual(previous[i], next[i])) {
        prefixMatches = false;
        break;
      }
    }
    if (prefixMatches) {
      if (nextLen === prevLen) {
        return previous as TimelineMessage[];
      }
      const shared = previous.slice() as TimelineMessage[];
      for (let i = prevLen; i < nextLen; i += 1) {
        shared.push(next[i]);
      }
      return shared;
    }
  }

  const prevById = new Map(previous.map((message) => [message.id, message]));
  let allSharedInOrder = prevLen === nextLen;
  const shared = next.map((message, index) => {
    const prev = prevById.get(message.id);
    if (prev && timelineMessagesEqual(prev, message)) {
      if (previous[index] !== prev) {
        allSharedInOrder = false;
      }
      return prev;
    }
    allSharedInOrder = false;
    return message;
  });

  if (allSharedInOrder) {
    return previous as TimelineMessage[];
  }
  return shared;
}
