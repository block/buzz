import {
  CHANNEL_AUX_EVENT_KINDS,
  CHANNEL_EVENT_KINDS,
  CHANNEL_TIMELINE_CONTENT_KINDS,
  HOME_MENTION_EVENT_KINDS,
  KIND_DELETION,
  KIND_NIP29_DELETE_EVENT,
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
 * {@link buildChannelAuxFilter}, and arrive for future messages through the
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
 * Aux-backfill filter for one chunk of loaded message ids: pulls reactions/
 * edits/deletions ({@link CHANNEL_AUX_EVENT_KINDS}) that reference those ids
 * by `#e`. Keyed by reference, not time, so a late edit/deletion for an old
 * visible message still applies — see {@link buildChannelHistoryFilter}.
 */
export function buildChannelAuxFilter(
  _channelId: string,
  messageIds: string[],
): RelaySubscriptionFilter {
  return buildChannelAuxKindFilter(messageIds, [...CHANNEL_AUX_EVENT_KINDS]);
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
