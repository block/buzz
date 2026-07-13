import type { RelayEvent } from "@/shared/api/types";
import { CHANNEL_TIMELINE_CONTENT_KINDS } from "@/shared/constants/kinds";
import {
  flattenChannelWindowEvents,
  type ChannelWindowStore,
} from "./channelWindowStore";
import { mergeMessages } from "./messageMerge";
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

  // Merge authoritative rows last so an acknowledged relay event replaces its
  // matching optimistic row while preserving the local render key.
  return windowEvents.reduce(
    (current, event) => mergeMessages(current, event),
    retained,
  );
}
