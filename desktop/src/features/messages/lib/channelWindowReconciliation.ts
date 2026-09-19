import type { RelayEvent } from "@/shared/api/types";
import { CHANNEL_TIMELINE_CONTENT_KINDS } from "@/shared/constants/kinds";
import {
  channelWindowThreadRepliesInChannel,
  compareRelayOrder,
  flattenChannelWindowEvents,
  type ChannelWindowStore,
} from "./channelWindowStore";
import { reconcileIncomingMessage } from "./messageMerge";
import { getThreadReference, isBroadcastReply } from "./threading";

const CHANNEL_TIMELINE_KINDS = new Set<number>(CHANNEL_TIMELINE_CONTENT_KINDS);

function retainRefetchReconciliationEvents(
  events: RelayEvent[],
  threadRepliesInChannel: boolean,
) {
  return events.filter((event) => {
    if (!CHANNEL_TIMELINE_KINDS.has(event.kind)) return false;
    if (event.pending) return true;
    if (threadRepliesInChannel) return false;
    const thread = getThreadReference(event.tags);
    return thread.parentId !== null && !isBroadcastReply(event.tags);
  });
}

/**
 * Project the timeline from the authoritative window while retaining local
 * pending sends. When the community does not project thread replies into the
 * channel timeline, retain non-broadcast thread replies as thread-panel cache.
 */
export function reconcileChannelWindowMessages(
  window: ChannelWindowStore,
  messages: RelayEvent[],
) {
  const windowEvents = flattenChannelWindowEvents(window);
  const threadRepliesInChannel = channelWindowThreadRepliesInChannel(window);
  if (window.pages.length === 0) {
    // A pageless window is unresolved, not authoritative. This state can exist
    // briefly when the companion window query mounts beside an already-cached
    // rendered timeline. Preserve that cache while admitting live events;
    // otherwise the first live event projects a one-row overlay over the
    // entire conversation until reload refetches page zero.
    let merged = messages;
    for (const event of windowEvents) {
      merged = reconcileIncomingMessage(merged, event);
    }
    return [...merged].sort((left, right) => compareRelayOrder(right, left));
  }
  const authoritativeIds = new Set(windowEvents.map((event) => event.id));
  const retained = retainRefetchReconciliationEvents(
    messages,
    threadRepliesInChannel,
  ).filter((event) => !authoritativeIds.has(event.id));

  // Reconcile acknowledgements against cache-only rows without changing the
  // authoritative window's order. The render key moves from an optimistic row
  // to its relay acknowledgement while the relay row remains in its original
  // cursor position.
  let cacheOnly = retained;
  const authoritative = windowEvents.map((event) => {
    const reconciled = reconcileIncomingMessage(cacheOnly, event);
    const incoming = reconciled.at(-1);
    cacheOnly = reconciled.slice(0, -1);
    return incoming ?? event;
  });

  return mergeChronologicalMessages(cacheOnly, authoritative);
}

function mergeChronologicalMessages(
  cacheOnly: RelayEvent[],
  authoritative: RelayEvent[],
) {
  const retained = [...cacheOnly].sort((left, right) =>
    compareRelayOrder(right, left),
  );
  const merged: RelayEvent[] = [];
  let retainedIndex = 0;
  let authoritativeIndex = 0;

  while (
    retainedIndex < retained.length &&
    authoritativeIndex < authoritative.length
  ) {
    const retainedEvent = retained[retainedIndex];
    const authoritativeEvent = authoritative[authoritativeIndex];
    if (compareRelayOrder(retainedEvent, authoritativeEvent) > 0) {
      merged.push(retainedEvent);
      retainedIndex += 1;
    } else {
      merged.push(authoritativeEvent);
      authoritativeIndex += 1;
    }
  }

  return merged.concat(
    retained.slice(retainedIndex),
    authoritative.slice(authoritativeIndex),
  );
}
