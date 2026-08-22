import type { QueryClient } from "@tanstack/react-query";

import type { RelayEvent } from "@/shared/api/types";
import { channelMessagesKey } from "./messageQueryKeys";
import { normalizeEventId } from "./threading";

/**
 * Every cached event we might be replying to in `channelId`.
 *
 * The channel timeline is not the only place a reply target lives: the thread
 * panel keeps its replies in a separate `["thread-replies", channelId, rootId]`
 * cache, and the Inbox can answer a message in a channel the user never opened
 * this session. Looking only at the channel cache silently misses both, which
 * costs the reply its parent-author `p` tag and collapses its root to the
 * parent id.
 */
export function getReplyContextEvents(
  queryClient: QueryClient,
  channelId: string,
): RelayEvent[] {
  const channelMessages =
    queryClient.getQueryData<RelayEvent[]>(channelMessagesKey(channelId)) ?? [];
  const threadReplies = queryClient
    .getQueriesData<RelayEvent[]>({ queryKey: ["thread-replies", channelId] })
    .flatMap(([, events]) => events ?? []);

  return threadReplies.length === 0
    ? channelMessages
    : [...channelMessages, ...threadReplies];
}

/**
 * Author of the message a reply answers, or `null` when it is not cached.
 */
export function findReplyParentAuthor(
  events: readonly RelayEvent[],
  parentEventId: string | null | undefined,
): string | null {
  if (!parentEventId) {
    return null;
  }
  return (
    events.find((event) => event.id === parentEventId)?.pubkey?.trim() || null
  );
}

/**
 * Parent author for a single reply, without materializing the whole context.
 *
 * The channel timeline holds the parent for nearly every reply, so the
 * thread-reply caches — whose lookup scans the entire query cache — are only
 * consulted on a miss. Returns `null` immediately when there is no parent, so
 * the common non-reply message costs nothing.
 */
export function lookupReplyParentAuthor(
  queryClient: QueryClient,
  channelId: string,
  parentEventId: string | null | undefined,
): string | null {
  if (!parentEventId) {
    return null;
  }
  const fromChannel = findReplyParentAuthor(
    queryClient.getQueryData<RelayEvent[]>(channelMessagesKey(channelId)) ?? [],
    parentEventId,
  );
  if (fromChannel !== null) {
    return fromChannel;
  }
  for (const [, events] of queryClient.getQueriesData<RelayEvent[]>({
    queryKey: ["thread-replies", channelId],
  })) {
    const found = findReplyParentAuthor(events ?? [], parentEventId);
    if (found !== null) {
      return found;
    }
  }
  return null;
}

/**
 * Outcome of resolving the author a reply answers.
 *
 * `absent` and `unavailable` have to stay distinct. Ownership of a reply
 * notification is decided twice — once here and once by the Inbox feed's
 * server-side lookup — and the two only agree if a lookup that *failed* is
 * treated differently from a parent that genuinely is not there. Collapsing
 * both to `null` makes one relay hiccup drop the event on both sides.
 */
export type ReplyParentAuthorResult =
  | { pubkey: string; status: "resolved" }
  | { pubkey: null; status: "absent" }
  | { pubkey: null; status: "unavailable" };

const PARENT_ABSENT = { pubkey: null, status: "absent" } as const;
const PARENT_UNAVAILABLE = { pubkey: null, status: "unavailable" } as const;

/**
 * In-flight and settled parent lookups, keyed by parent event id.
 *
 * A parent is answered once, not once per reply. Thirty replies to the same
 * message would otherwise issue thirty identical `#ids` REQs, each a fresh
 * subscription queued behind the rate-limit gate that foreground channel
 * history also uses. Failures are deliberately not cached — `unavailable`
 * means "try again", and caching it would make one relay flap sticky.
 *
 * Community-scoped: cleared by `resetCommunityState()`.
 */
const parentAuthorCache = new Map<string, Promise<ReplyParentAuthorResult>>();
const PARENT_AUTHOR_CACHE_LIMIT = 500;

/** Backoff between parent-lookup attempts. Length is the retry count. */
const PARENT_FETCH_RETRY_DELAYS_MS = [500, 2_000] as const;

const sleep = (ms: number) =>
  new Promise<void>((resolve) => setTimeout(resolve, ms));

/** Clears the parent-author cache. Wired into `resetCommunityState()`. */
export function resetReplyParentAuthorCache() {
  parentAuthorCache.clear();
}

/**
 * Parent author for a single reply, falling back to the relay on a cache miss.
 *
 * The query caches only cover channels the user has opened this session, and
 * `useLiveChannelUpdates` deliberately does not seed them. Without the relay
 * fallback, "did this reply answer me?" is unanswerable for every unopened
 * channel — which is where the reply notifications matter most.
 */
export async function resolveReplyParentAuthor({
  channelId,
  fetchEvents,
  kinds,
  parentEventId,
  queryClient,
}: {
  channelId: string;
  fetchEvents: (filter: {
    kinds: number[];
    ids: string[];
    limit: number;
  }) => Promise<RelayEvent[]>;
  kinds: readonly number[];
  parentEventId: string | null | undefined;
  queryClient: QueryClient;
}): Promise<ReplyParentAuthorResult> {
  // Validated, not just presence-checked. The relay stores an `e` value it cannot
  // parse as an event id, and puts into an `ids` filter it answers with a bare
  // NOTICE — which this client never resolves, so the request hangs the history
  // timeout before rejecting. Treat an unlookupable parent as absent instead.
  parentEventId = normalizeEventId(parentEventId);
  if (!parentEventId) {
    return PARENT_ABSENT;
  }
  const cached = lookupReplyParentAuthor(queryClient, channelId, parentEventId);
  if (cached !== null) {
    return { pubkey: cached, status: "resolved" };
  }

  const inFlight = parentAuthorCache.get(parentEventId);
  if (inFlight) {
    return inFlight;
  }

  const pending = (async (): Promise<ReplyParentAuthorResult> => {
    for (let attempt = 0; ; attempt += 1) {
      let events: RelayEvent[];
      try {
        // `kinds` is required — an open-ended filter hits the relay p-gate.
        events = await fetchEvents({
          kinds: [...kinds],
          ids: [parentEventId],
          limit: 1,
        });
      } catch {
        // Worth retrying rather than guessing. The caller's verdict is
        // persisted per event id and never recomputed, so a guess made during
        // a two-second relay blip is permanent for as long as the channel
        // stays unread — the toast and the mention badge would disagree
        // forever. Retries are per parent id, not per reply, because
        // concurrent callers share this promise.
        if (attempt < PARENT_FETCH_RETRY_DELAYS_MS.length) {
          await sleep(PARENT_FETCH_RETRY_DELAYS_MS[attempt] as number);
          continue;
        }
        parentAuthorCache.delete(parentEventId);
        return PARENT_UNAVAILABLE;
      }

      const author = findReplyParentAuthor(events, parentEventId);
      if (author !== null) {
        return { pubkey: author, status: "resolved" };
      }
      // A parent the relay does not return is not necessarily gone — it may
      // simply be a kind outside `kinds`. Caching that would poison every
      // later reply to the same parent, and the two consumers read a null
      // author in opposite directions.
      parentAuthorCache.delete(parentEventId);
      return PARENT_ABSENT;
    }
  })();

  if (parentAuthorCache.size >= PARENT_AUTHOR_CACHE_LIMIT) {
    const oldest = parentAuthorCache.keys().next().value;
    if (oldest !== undefined) {
      parentAuthorCache.delete(oldest);
    }
  }
  parentAuthorCache.set(parentEventId, pending);
  return pending;
}
