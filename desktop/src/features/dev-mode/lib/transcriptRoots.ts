import { isThreadReply } from "@/features/messages/lib/threading";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
} from "@/shared/constants/kinds";

export const DEV_MESSAGE_KINDS = new Set([
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
]);

export function byCreatedAscending(left: RelayEvent, right: RelayEvent) {
  return left.created_at !== right.created_at
    ? left.created_at - right.created_at
    : left.id < right.id
      ? -1
      : 1;
}

/**
 * How many of a thread's ordered replies render inline in the main chat
 * view: the whole leading run of agent replies — collapsing starts at the
 * first human response. A human-first thread still shows its first reply
 * inline. Shared by the transcript renderer and unread routing so "would
 * this reply be visible without opening the thread" has one answer.
 */
export function selectInlineVisibleCount(
  orderedReplies: readonly RelayEvent[],
  isAgent: (pubkey: string) => boolean,
): number {
  let end = 0;
  while (end < orderedReplies.length && isAgent(orderedReplies[end].pubkey)) {
    end += 1;
  }
  if (end === 0 && orderedReplies.length > 0) end = 1;
  return end;
}

/** Top-level prompt messages of a channel, oldest first. */
export function selectRootEvents(
  events: RelayEvent[] | undefined,
): RelayEvent[] {
  return (events ?? [])
    .filter(
      (event) =>
        DEV_MESSAGE_KINDS.has(event.kind) && !isThreadReply(event.tags),
    )
    .sort(byCreatedAscending);
}
