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
