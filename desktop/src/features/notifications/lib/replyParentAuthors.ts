import {
  getThreadReference,
  normalizeEventId,
} from "@/features/messages/lib/threading";
import type { RelayEvent } from "@/shared/api/types";

/**
 * Ids per parent-lookup REQ. Mirrors `AUX_BACKFILL_CHUNK_SIZE` — the same relay
 * filter limit applies, and this query has the same shape (many ids, one kind
 * set).
 */
const PARENT_LOOKUP_CHUNK_SIZE = 100;

/**
 * Authors of the events a batch of replies answers, keyed by event id.
 *
 * Catch-up windows start strictly after the read marker, so the parent of a
 * reply — typically our own older message — is never in the batch that
 * contains the reply. `shouldNotifyForEvent` needs that author to tell a real
 * mention from a reply's addressing `p` tag.
 *
 * `shouldResolveParent` decides which replies are worth the extra relay round
 * trip. The useful rule is "does this event `p`-tag the current user?" — that
 * is the only case where the parent's author changes any answer, and it is
 * needed whether or not the channel is muted, because high-priority marking
 * depends on it too. A caller that cannot cheaply tell may resolve everything.
 */
export async function collectReplyParentAuthors({
  events,
  fetchEvents,
  kinds,
  shouldResolveParent,
}: {
  events: readonly RelayEvent[];
  fetchEvents: (filter: {
    kinds: number[];
    ids: string[];
    limit: number;
  }) => Promise<RelayEvent[]>;
  kinds: readonly number[];
  shouldResolveParent: (
    ref: { parentId: string | null; rootId: string | null },
    event: RelayEvent,
  ) => boolean;
}): Promise<Map<string, string>> {
  const authorByEventId = new Map(
    events.map((event) => [event.id, event.pubkey]),
  );
  const missing = [
    ...new Set(
      events
        .map((event) => ({ event, ref: getThreadReference(event.tags) }))
        .filter(
          ({ event, ref }) =>
            ref.parentId !== null &&
            !authorByEventId.has(ref.parentId) &&
            shouldResolveParent(ref, event),
        )
        // A tag value the relay stored but cannot parse as an event id would make
        // it answer a bare NOTICE, which this client never resolves — the request
        // hangs the history timeout, then rejects, and the caller retries the whole
        // channel forever. Skip it: an unlookupable parent is simply unresolved.
        .map(({ ref }) => normalizeEventId(ref.parentId))
        .filter((id): id is string => id !== null),
    ),
  ];
  if (missing.length === 0) {
    return authorByEventId;
  }

  // Deliberately not caught. Swallowing the failure and returning a
  // batch-only map looks like a graceful degrade but is not one: an
  // unresolved parent is guessed at, and both callers persist or cache that
  // guess. Both of them already treat a throw as "retry this channel", which
  // is the only outcome that self-corrects.
  //
  // Chunked for the same reason as `AUX_BACKFILL_CHUNK_SIZE`: an `ids` filter
  // this wide exceeds the relay's filter limits. `useUnreadChannels` can reach
  // `CATCH_UP_LIMIT` (1000) ids and the community observer 150 per channel, and
  // a truncated or rejected REQ is worse here than a slow one — an unreturned
  // parent reads as a mention, which inflates the mention count in one caller
  // and silently clears `highPriority` in the other, where it is persisted.
  //
  // Sequential, not `Promise.all`: these run behind the same rate-limit gate as
  // foreground channel history, and a 10-chunk burst per channel would starve
  // the UI. A throw propagates and the caller retries the whole channel.
  //
  // `kinds` is required — an open-ended filter hits the relay p-gate (403).
  for (
    let index = 0;
    index < missing.length;
    index += PARENT_LOOKUP_CHUNK_SIZE
  ) {
    const ids = missing.slice(index, index + PARENT_LOOKUP_CHUNK_SIZE);
    const parents = await fetchEvents({
      kinds: [...kinds],
      ids,
      limit: ids.length,
    });
    for (const parent of parents) {
      authorByEventId.set(parent.id, parent.pubkey);
    }
  }
  return authorByEventId;
}
