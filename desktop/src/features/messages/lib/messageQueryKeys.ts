import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_JOB_ACCEPTED,
  KIND_JOB_CANCEL,
  KIND_JOB_ERROR,
  KIND_JOB_PROGRESS,
  KIND_JOB_REQUEST,
  KIND_JOB_RESULT,
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_DIFF,
  KIND_STREAM_MESSAGE_V2,
  KIND_SYSTEM_MESSAGE,
} from "@/shared/constants/kinds";

const MAX_TIMELINE_MESSAGES = 2_000;

export function channelMessagesKey(channelId: string) {
  return ["channel-messages", channelId] as const;
}

export function dedupeMessagesById(messages: RelayEvent[]) {
  const seenIds = new Set<string>();
  const deduped: RelayEvent[] = [];

  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];

    if (seenIds.has(message.id)) {
      continue;
    }

    seenIds.add(message.id);
    deduped.push(message);
  }

  return deduped.reverse();
}

export function sortMessages(messages: RelayEvent[]) {
  return dedupeMessagesById(messages).sort(
    (left, right) => left.created_at - right.created_at,
  );
}

function isTimelineWindowContentEvent(event: RelayEvent) {
  return (
    event.kind === KIND_STREAM_MESSAGE ||
    event.kind === KIND_STREAM_MESSAGE_V2 ||
    event.kind === KIND_STREAM_MESSAGE_DIFF ||
    event.kind === KIND_SYSTEM_MESSAGE ||
    event.kind === KIND_JOB_REQUEST ||
    event.kind === KIND_JOB_ACCEPTED ||
    event.kind === KIND_JOB_PROGRESS ||
    event.kind === KIND_JOB_RESULT ||
    event.kind === KIND_JOB_CANCEL ||
    event.kind === KIND_JOB_ERROR
  );
}

/**
 * Sort, dedupe, and cap the timeline at {@link MAX_TIMELINE_MESSAGES} visible
 * content events so de-virtualized rendering does not grow into an unbounded
 * DOM during long-lived channel sessions.
 *
 * Auxiliary events (reactions, edits, tombstones) are kept in cache so they can
 * still apply to retained or later-loaded content, but they must not consume the
 * visible message window and evict older loaded roots.
 */
export function normalizeTimelineMessages(messages: RelayEvent[]) {
  const normalized = sortMessages(messages);
  const contentEvents = normalized.filter(isTimelineWindowContentEvent);

  if (contentEvents.length <= MAX_TIMELINE_MESSAGES) {
    return normalized;
  }

  const retainedContentIds = new Set(
    contentEvents.slice(-MAX_TIMELINE_MESSAGES).map((event) => event.id),
  );

  return normalized.filter(
    (event) =>
      !isTimelineWindowContentEvent(event) || retainedContentIds.has(event.id),
  );
}

export function mergeTimelineHistoryMessages(
  current: RelayEvent[],
  history: RelayEvent[],
) {
  return normalizeTimelineMessages([...current, ...history]);
}
