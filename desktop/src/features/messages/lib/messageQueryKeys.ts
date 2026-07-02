import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_JOB_ACCEPTED,
  KIND_JOB_CANCEL,
  KIND_JOB_ERROR,
  KIND_JOB_PROGRESS,
  KIND_JOB_REQUEST,
  KIND_JOB_RESULT,
  KIND_HUDDLE_STARTED,
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

export function isTimelineWindowContentEvent(event: RelayEvent) {
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
    event.kind === KIND_JOB_ERROR ||
    event.kind === KIND_HUDDLE_STARTED
  );
}

/**
 * The oldest timestamp the older-history pager may anchor its `until` cursor
 * on: the oldest cached content event that arrived through contiguous history
 * loading. Events merged out-of-band (`nonContiguous`: thread ancestors,
 * thread-panel subtrees) are skipped — anchoring on such an "island" pages
 * from the island backward and permanently skips the unloaded gap between the
 * island and the contiguous window (the June-14 → June-9 scrollback hole).
 * Auxiliary events are skipped too: they are always newer than the content
 * they reference, so an aux event older than every contiguous content event
 * can only reference an island — making it an island itself.
 *
 * Assumes `events` is sorted ascending (every cache write stores sorted).
 * Returns null when no contiguously loaded content event is cached.
 */
export function oldestContiguousHistoryTimestamp(
  events: RelayEvent[],
): number | null {
  for (const event of events) {
    if (!event.nonContiguous && isTimelineWindowContentEvent(event)) {
      return event.created_at;
    }
  }
  return null;
}

function capNewestTimelineMessages(normalized: RelayEvent[]) {
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
  return capNewestTimelineMessages(sortMessages(messages));
}

/**
 * Merge a batch of events (older scrollback page, reconnect/revalidation
 * window, live gap-fill) into the timeline cache. Sort + dedupe only — the
 * {@link MAX_TIMELINE_MESSAGES} cap is deliberately NOT applied here.
 *
 * Capping merges into an already-painted timeline evicts history out from
 * under a reader: page back to an old day in a channel holding more than the
 * cap, and the next capped merge (live append, revalidation) silently deletes
 * the rows being read. The cap is applied only by
 * {@link normalizeTimelineMessages} at moments when nothing is rendered —
 * the cold-snapshot paint and the channel-leave trim in `hooks.ts`.
 */
export function mergeTimelineHistoryMessages(
  current: RelayEvent[],
  history: RelayEvent[],
) {
  return sortMessages([...current, ...history]);
}

/**
 * Merge events fetched out-of-band — by id (missing thread ancestors) or by
 * thread reference (thread-panel subtrees) — into the timeline cache, marked
 * `nonContiguous` so {@link oldestContiguousHistoryTimestamp} never anchors
 * the older-history pager on them. Events already in the cache are left
 * untouched: a copy that arrived contiguously must not be downgraded to an
 * island. The mark heals itself — when contiguous paging reaches an island,
 * the history merge's last-copy-wins dedupe replaces the flagged copy with
 * the unflagged one.
 */
export function mergeNonContiguousTimelineMessages(
  current: RelayEvent[],
  events: RelayEvent[],
) {
  const knownIds = new Set(current.map((event) => event.id));
  const additions = events
    .filter((event) => !knownIds.has(event.id))
    .map((event) => ({ ...event, nonContiguous: true }));
  if (additions.length === 0) {
    return current;
  }
  return sortMessages([...current, ...additions]);
}
