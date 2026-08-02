import type { RelayEvent } from "@/shared/api/types";
import {
  dedupeMessagesById,
  normalizeTimelineMessages,
  sortMessages,
} from "./messageQueryKeys";
import { getChannelIdFromTags, getThreadReference } from "./threading";

function getLocalRenderKey(message: RelayEvent) {
  return message.localKey ?? message.id;
}

function isMatchingPendingMessage(pending: RelayEvent, incoming: RelayEvent) {
  if (
    !pending.pending ||
    incoming.pending ||
    pending.content !== incoming.content ||
    pending.kind !== incoming.kind ||
    pending.pubkey.toLowerCase() !== incoming.pubkey.toLowerCase() ||
    getChannelIdFromTags(pending.tags) !== getChannelIdFromTags(incoming.tags)
  ) {
    return false;
  }

  const pendingThread = getThreadReference(pending.tags);
  const incomingThread = getThreadReference(incoming.tags);

  return (
    pendingThread.parentId === incomingThread.parentId &&
    pendingThread.rootId === incomingThread.rootId
  );
}

/**
 * Resolve the render key (localKey) the incoming event should carry, without
 * ever colliding with a confirmed row's key. Concurrent identical sends make
 * pending rows semantically indistinguishable, so key assignment must be
 * ownership-aware or a later echo/success evicts the other send's confirmed
 * row (the concurrent-identical-replies blocker):
 *
 *   1. A confirmed row with the same id already exists → the event was
 *      reconciled once (its echo or REST success won the race); inherit that
 *      row's key and never re-match a pending.
 *   2. The caller supplied a localKey (onSuccess binding) and no *other*
 *      confirmed row owns it → honor it. If a different confirmed row owns
 *      it (an echo claimed that pending first), fall through and claim a
 *      remaining pending instead.
 *   3. Otherwise claim the first semantically matching pending row.
 *
 * Pending and confirmed rows never share a key (claiming a pending removes
 * it in the same merge), so a claimed pending key cannot evict a confirmed
 * row.
 */
function resolveIncomingLocalKey(
  normalizedCurrent: RelayEvent[],
  incoming: RelayEvent,
): string | undefined {
  const confirmedExisting = normalizedCurrent.find(
    (message) => message.id === incoming.id && !message.pending,
  );
  if (confirmedExisting) {
    return getLocalRenderKey(confirmedExisting);
  }

  const supplied = incoming.localKey;
  if (supplied != null) {
    const ownedByOtherConfirmed = normalizedCurrent.some(
      (message) =>
        !message.pending &&
        message.id !== incoming.id &&
        getLocalRenderKey(message) === supplied,
    );
    if (!ownedByOtherConfirmed) {
      return supplied;
    }
  }

  const replacedPending = normalizedCurrent.find((message) =>
    isMatchingPendingMessage(message, incoming),
  );
  return replacedPending ? getLocalRenderKey(replacedPending) : undefined;
}

export function reconcileIncomingMessage(
  current: RelayEvent[],
  incoming: RelayEvent,
): RelayEvent[] {
  const normalizedCurrent = dedupeMessagesById(current);
  const localKey = resolveIncomingLocalKey(normalizedCurrent, incoming);
  const incomingWithLocalKey =
    localKey === incoming.localKey ? incoming : { ...incoming, localKey };
  const incomingLocalKey = getLocalRenderKey(incomingWithLocalKey);
  const deduped = normalizedCurrent.filter(
    (message) =>
      message.id !== incoming.id &&
      getLocalRenderKey(message) !== incomingLocalKey,
  );

  return [...deduped, incomingWithLocalKey];
}

function mergeMessagesWithNormalizer(
  current: RelayEvent[],
  incoming: RelayEvent,
  normalize: (messages: RelayEvent[]) => RelayEvent[],
): RelayEvent[] {
  return normalize(reconcileIncomingMessage(current, incoming));
}

export function mergeMessages(
  current: RelayEvent[],
  incoming: RelayEvent,
): RelayEvent[] {
  return mergeMessagesWithNormalizer(current, incoming, sortMessages);
}

export function mergeTimelineCacheMessages(
  current: RelayEvent[],
  incoming: RelayEvent,
): RelayEvent[] {
  return mergeMessagesWithNormalizer(
    current,
    incoming,
    normalizeTimelineMessages,
  );
}
