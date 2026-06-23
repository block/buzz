import {
  CHANNEL_EVENT_KINDS,
  CHANNEL_TIMELINE_CONTENT_KINDS,
  HOME_MENTION_EVENT_KINDS,
  KIND_DELETION,
  KIND_NIP29_DELETE_EVENT,
  KIND_REACTION,
  KIND_STREAM_MESSAGE_EDIT,
} from "@/shared/constants/kinds";
import type { RelaySubscriptionFilter } from "@/shared/api/relayClientShared";

// Auxiliary-event backfill: `#e` filters reference loaded message ids to pull
// their reactions/edits/deletions. Chunk the ids so each REQ stays within
// relay filter limits, and let each chunk return up to the relay's WS cap —
// a single reaction-heavy message can have many aux events.
export const AUX_BACKFILL_CHUNK_SIZE = 100;
export const MAX_HISTORICAL_LIMIT = 10_000;

/**
 * Live-subscription filter for an open channel: the broad
 * {@link CHANNEL_EVENT_KINDS} set so the tail delivers reactions/edits/
 * deletions for future messages as well as new message rows.
 */
export function buildChannelFilter(
  channelId: string,
  limit: number,
  until?: number,
): RelaySubscriptionFilter {
  const filter: RelaySubscriptionFilter = {
    kinds: [...CHANNEL_EVENT_KINDS],
    "#h": [channelId],
    limit,
  };

  if (until !== undefined) {
    filter.until = until;
  }

  return filter;
}

/**
 * History filter for cold-load and scrollback: message kinds *only*, so the
 * `limit` budget buys visible message depth. Auxiliary events (reactions,
 * edits, deletions) are backfilled separately by `#e` reference via
 * {@link buildChannelReactionAuxFilter} and
 * {@link buildChannelStructuralAuxFilter}, and arrive for future messages through the
 * live subscription ({@link buildChannelFilter}, which keeps the broad
 * {@link CHANNEL_EVENT_KINDS} set).
 */
export function buildChannelHistoryFilter(
  channelId: string,
  limit: number,
  until?: number,
): RelaySubscriptionFilter {
  const filter: RelaySubscriptionFilter = {
    kinds: [...CHANNEL_TIMELINE_CONTENT_KINDS],
    "#h": [channelId],
    limit,
  };

  if (until !== undefined) {
    filter.until = until;
  }

  return filter;
}

/**
 * Reactions-only aux filter (kind:7 by `#e`). Reactions load on their own fast
 * REQ, isolated from the slow kind:5 deletion scan they were previously bundled
 * with. On a busy workspace the bundled query queued past the history-load
 * timeout and the all-or-nothing catch dropped every reaction on cold-load;
 * kind:7 alone is ~5x faster (measured against staging), so it reliably beats
 * the timeout. Keyed by `#e` reference, not time, so an old reaction on a
 * visible message still applies — see {@link buildChannelHistoryFilter}.
 */
export function buildChannelReactionAuxFilter(
  _channelId: string,
  messageIds: string[],
): RelaySubscriptionFilter {
  return buildChannelAuxKindFilter(messageIds, [KIND_REACTION]);
}

/**
 * Structural aux overlay filter: edits + deletions ({@link KIND_STREAM_MESSAGE_EDIT},
 * {@link KIND_DELETION}, {@link KIND_NIP29_DELETE_EVENT}) by `#e`. The slow
 * half of the old bundle — fetched separately so a stale/slow deletion scan
 * can't strand reactions. A missed edit/deletion only renders a message
 * un-edited until the next backfill; a missed reaction looks like data loss.
 */
export function buildChannelStructuralAuxFilter(
  _channelId: string,
  messageIds: string[],
): RelaySubscriptionFilter {
  return buildChannelAuxKindFilter(messageIds, [
    KIND_STREAM_MESSAGE_EDIT,
    KIND_DELETION,
    KIND_NIP29_DELETE_EVENT,
  ]);
}

export function buildChannelAuxDeletionFilter(
  _channelId: string,
  auxEventIds: string[],
): RelaySubscriptionFilter {
  return buildChannelAuxKindFilter(auxEventIds, [
    KIND_DELETION,
    KIND_NIP29_DELETE_EVENT,
  ]);
}

// No `#h`: reaction/reaction-removal events carry only an `e` tag, so an
// `#h`-scoped query misses them; `#e` over unique ids is already specific.
function buildChannelAuxKindFilter(
  referencedEventIds: string[],
  kinds: number[],
): RelaySubscriptionFilter {
  return {
    kinds,
    "#e": referencedEventIds,
    limit: MAX_HISTORICAL_LIMIT,
  };
}

export function buildGlobalStreamFilter(
  limit: number,
): RelaySubscriptionFilter {
  return {
    kinds: [...CHANNEL_EVENT_KINDS],
    limit,
  };
}

export function buildChannelMentionFilter(
  channelId: string,
  pubkey: string,
  limit: number,
): RelaySubscriptionFilter {
  return {
    kinds: [...HOME_MENTION_EVENT_KINDS],
    "#h": [channelId],
    "#p": [pubkey],
    limit,
    since: Math.floor(Date.now() / 1_000),
  };
}
