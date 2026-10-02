import {
  formatTimelineMessages,
  isTimelineContentEvent,
} from "@/features/messages/lib/formatTimelineMessages";
import { buildThreadPanelData } from "@/features/messages/lib/threadPanel";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_DELETION,
  KIND_NIP29_DELETE_EVENT,
} from "@/shared/constants/kinds";

/**
 * Aux events (edits/deletions/reactions) already loaded in the channel window
 * that reference `headId` via an `#e` tag. The thread head is the single
 * content event found by id, but its overlay events live alongside it in the
 * channel window — `formatTimelineMessages` only applies an edit/deletion when
 * the aux event sits in the SAME array as its target. Carrying them here keeps
 * the thread head byte-identical to the main timeline the instant the thread
 * opens, instead of rendering the un-edited original until the async
 * thread-aux backfill (`withThreadAux`) lands — the stale-edit-in-thread bug.
 *
 * Restricted to non-content kinds so reply content events (which also `#e` the
 * head as their parent) never leak in here — replies come from `replyEvents`.
 *
 * Deletion closure: an edit/reaction on the head can itself be deleted by a
 * kind:5/9005 that `#e`-references the *overlay's* id, not the head's. Those
 * deletions are copied too, so a deleted edit/reaction stays deleted in the
 * thread head instead of being resurrected until the async aux backfill lands
 * (or permanently if it fails) — mirroring the closure the aux-backfill paths
 * build (`mergeAuxEventsWithDeletionBackfill`).
 */
function headAuxEventsFromChannelWindow(
  channelEvents: RelayEvent[],
  headId: string,
): RelayEvent[] {
  const directAux = channelEvents.filter(
    (event) =>
      event.id !== headId &&
      !isTimelineContentEvent(event) &&
      event.tags.some((tag) => tag[0] === "e" && tag[1] === headId),
  );
  const directAuxIds = new Set(directAux.map((event) => event.id));
  const deletionsOfAux = channelEvents.filter(
    (event) =>
      (event.kind === KIND_DELETION ||
        event.kind === KIND_NIP29_DELETE_EVENT) &&
      !directAuxIds.has(event.id) &&
      event.tags.some((tag) => tag[0] === "e" && directAuxIds.has(tag[1])),
  );
  return [...directAux, ...deletionsOfAux];
}

export function buildIndependentThreadPanel(
  channelEvents: RelayEvent[],
  replyEvents: RelayEvent[],
  rootId: string | null,
  replyTargetId: string | null,
  expandedReplyIds: ReadonlySet<string>,
  ...formatArgs: Tail<Parameters<typeof formatTimelineMessages>>
) {
  if (!rootId) {
    return {
      ...buildThreadPanelData([], null, replyTargetId, expandedReplyIds),
      messages: [],
    };
  }
  const head = channelEvents.find((event) => event.id === rootId);
  // Dedup the channel-window head aux against `replyEvents`: `withThreadAux`
  // fetches the same overlays by reference, so both sources can carry an edit.
  const replyEventIds = new Set(replyEvents.map((event) => event.id));
  const headAux = head
    ? headAuxEventsFromChannelWindow(channelEvents, rootId).filter(
        (event) => !replyEventIds.has(event.id),
      )
    : [];
  const events = head ? [head, ...headAux, ...replyEvents] : replyEvents;
  const messages = formatTimelineMessages(events, ...formatArgs);
  return {
    ...buildThreadPanelData(messages, rootId, replyTargetId, expandedReplyIds),
    messages,
  };
}

type Tail<T extends readonly unknown[]> = T extends readonly [
  unknown,
  ...infer R,
]
  ? R
  : never;
