import type { RelayEvent } from "@/shared/api/types";
import { CHANNEL_TIMELINE_CONTENT_KINDS } from "@/shared/constants/kinds";
import {
  compareRelayOrder,
  flattenChannelWindowEvents,
  type ChannelWindowStore,
} from "./channelWindowStore";
import { reconcileIncomingMessage } from "./messageMerge";
import { getThreadReference, isBroadcastReply } from "./threading";

const CHANNEL_TIMELINE_KINDS = new Set<number>(CHANNEL_TIMELINE_CONTENT_KINDS);

function retainRefetchReconciliationEvents(events: RelayEvent[]) {
  return events.filter((event) => {
    if (!CHANNEL_TIMELINE_KINDS.has(event.kind)) return false;
    if (event.pending) return true;
    const thread = getThreadReference(event.tags);
    return thread.parentId !== null && !isBroadcastReply(event.tags);
  });
}

/**
 * Project the timeline from the authoritative window while retaining local
 * pending sends and non-broadcast thread replies the window does not contain.
 */
export function reconcileChannelWindowMessages(
  window: ChannelWindowStore,
  messages: RelayEvent[],
) {
  const windowEvents = flattenChannelWindowEvents(window);
  const authoritativeIds = new Set(windowEvents.map((event) => event.id));
  const retained = retainRefetchReconciliationEvents(messages).filter(
    (event) => !authoritativeIds.has(event.id),
  );

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
