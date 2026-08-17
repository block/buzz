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

export function reconcileIncomingMessage(
  current: RelayEvent[],
  incoming: RelayEvent,
): RelayEvent[] {
  const normalizedCurrent = dedupeMessagesById(current);
  const replacedPending = normalizedCurrent.find((message) =>
    isMatchingPendingMessage(message, incoming),
  );
  const incomingWithLocalKey = replacedPending
    ? {
        ...incoming,
        localKey: replacedPending.localKey ?? replacedPending.id,
      }
    : incoming;
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
