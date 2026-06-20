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
  return dedupeMessagesById(messages).sort((left, right) => {
    if (left.created_at !== right.created_at) {
      return left.created_at - right.created_at;
    }
    // Tiebreak same-second events on id so the merge order is deterministic.
    // Without this, two events sharing a created_at can land in a different
    // position depending on which REQ (history vs live-sub) delivered them
    // first — reading as a "missing"/shuffled message at a fixed scroll offset.
    return left.id < right.id ? -1 : left.id > right.id ? 1 : 0;
  });
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
 * Which end of the timeline to keep when the visible-content window overflows
 * {@link MAX_TIMELINE_MESSAGES}.
 *
 * - `"newest"` (default): keep the most recent rows, evict the oldest. Correct
 *   for cold-load and live-tailing — new messages arrive at the bottom, so the
 *   far-older head can age off without the reader noticing.
 * - `"oldest"`: keep the earliest rows, evict the newest. Correct for
 *   scrollback — the reader is moving *toward* the head, so evicting the head
 *   would delete the very rows they're scrolling to (e.g. the first messages of
 *   the channel never render once history exceeds the cap).
 */
type TimelineWindowEnd = "newest" | "oldest";

/**
 * Sort, dedupe, and cap the timeline at {@link MAX_TIMELINE_MESSAGES} visible
 * content events so de-virtualized rendering does not grow into an unbounded
 * DOM during long-lived channel sessions.
 *
 * `keep` selects which end survives when over the cap — see
 * {@link TimelineWindowEnd}. Defaults to `"newest"` so live-tail and cold-load
 * behaviour is unchanged; scrollback merges pass `"oldest"` so paging back to
 * the start of a >cap channel does not evict the head out from under the reader.
 *
 * Auxiliary events (reactions, edits, tombstones) are kept in cache so they can
 * still apply to retained or later-loaded content, but they must not consume the
 * visible message window and evict loaded roots.
 */
export function normalizeTimelineMessages(
  messages: RelayEvent[],
  keep: TimelineWindowEnd = "newest",
) {
  const normalized = sortMessages(messages);
  const contentEvents = normalized.filter(isTimelineWindowContentEvent);

  if (contentEvents.length <= MAX_TIMELINE_MESSAGES) {
    return normalized;
  }

  const retainedWindow =
    keep === "oldest"
      ? contentEvents.slice(0, MAX_TIMELINE_MESSAGES)
      : contentEvents.slice(-MAX_TIMELINE_MESSAGES);
  const retainedContentIds = new Set(retainedWindow.map((event) => event.id));

  return normalized.filter(
    (event) =>
      !isTimelineWindowContentEvent(event) || retainedContentIds.has(event.id),
  );
}

export function mergeTimelineHistoryMessages(
  current: RelayEvent[],
  history: RelayEvent[],
  keep: TimelineWindowEnd = "newest",
) {
  return normalizeTimelineMessages([...current, ...history], keep);
}
