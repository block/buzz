import { CHANNEL_EVENT_KINDS } from "@/shared/constants/kinds";
import type {
  RelaySubscription,
  RelaySubscriptionFilter,
} from "@/shared/api/relayClientShared";
import type { RelayEvent } from "@/shared/api/types";
import {
  isRateLimited,
  waitForRateLimit,
} from "@/shared/api/relayRateLimitGate";

const RECONNECT_REPLAY_SKEW_SECS = 5;
export const RECONNECT_REPLAY_PAGE_LIMIT = 500;
export const RECONNECT_REPLAY_PAGE_CONCURRENCY = 4;

/**
 * Maximum attempts for one subscription's paged history backfill.
 *
 * Backfill failures must never escape `replayLiveSubscriptions`: by the time
 * paging starts, every live REQ has already been re-established on a healthy,
 * authenticated socket. Letting a history rejection propagate makes the
 * session tear that socket down (`resetConnection`) and reconnect straight
 * into the same rate-limit window — the "briefly connected → can't reach the
 * relay" flap loop. Instead each sub retries behind the rate-limit gate a
 * bounded number of times, then degrades to live-only for this connection.
 * The window's lower bound is pinned in `pendingReplaySince` while unresolved
 * (live events advance `lastSeenCreatedAt` regardless of backfill success),
 * so the next reconnect still requests the missed window.
 */
export const PAGE_REPLAY_MAX_ATTEMPTS = 3;

/**
 * Maximum live subscriptions sent per relay REQ burst during reconnect.
 *
 * Capping the initial blast prevents admission-control bursts on degraded
 * networks where the relay is already near its per-pubkey quota.
 */
export const REPLAY_BATCH_SIZE = 8;

/**
 * Delay between consecutive replay batches (milliseconds).
 *
 * Spreads the REQ storm across time so the relay's sliding quota window
 * can absorb each batch without triggering rate-limiting on the next.
 */
export const REPLAY_INTER_BATCH_DELAY_MS = 50;

async function runWithConcurrency<T>(
  items: T[],
  concurrency: number,
  worker: (item: T) => Promise<void>,
) {
  const workerCount = Math.min(Math.max(1, concurrency), items.length);
  let nextIndex = 0;

  await Promise.all(
    Array.from({ length: workerCount }, async () => {
      while (nextIndex < items.length) {
        const item = items[nextIndex++];
        await worker(item);
      }
    }),
  );
}

export function buildReconnectReplayFilter(
  filter: RelaySubscriptionFilter,
  since?: number,
  until?: number,
  limit = Math.min(filter.limit, RECONNECT_REPLAY_PAGE_LIMIT),
) {
  if (since === undefined) return filter;

  const replayFilter: RelaySubscriptionFilter = {
    ...filter,
    limit,
    since: filter.since === undefined ? since : Math.max(filter.since, since),
  };

  if (until !== undefined) {
    replayFilter.until =
      filter.until === undefined ? until : Math.min(filter.until, until);
  }

  return replayFilter;
}

export function shouldPageReconnectReplay(filter: RelaySubscriptionFilter) {
  return (
    filter.limit > 0 &&
    Array.isArray(filter["#h"]) &&
    filter["#h"].length === 1 &&
    CHANNEL_EVENT_KINDS.every((kind) => filter.kinds.includes(kind))
  );
}

/**
 * Page one subscription's missed-window history.
 *
 * Returns `true` only when the window was genuinely completed (short page or
 * boundary reached). Returns `false` when the pass aborted because the
 * connection went stale (`isActive()` false) — callers must NOT treat that as
 * completion: the same subscription object is shared with the superseding
 * connection, and clearing its pinned `pendingReplaySince` on a stale abort
 * would erase the floor the new connection still needs.
 */
export async function replayReconnectHistoryPages({
  subscription,
  since,
  until,
  isActive,
  requestHistory,
}: {
  subscription: Extract<RelaySubscription, { mode: "live" }>;
  since: number;
  until: number;
  isActive: () => boolean;
  requestHistory: (filter: RelaySubscriptionFilter) => Promise<RelayEvent[]>;
}): Promise<boolean> {
  let pageUntil = until;

  while (pageUntil >= since) {
    if (!isActive()) return false;

    const events = await requestHistory(
      buildReconnectReplayFilter(
        subscription.filter,
        since,
        pageUntil,
        RECONNECT_REPLAY_PAGE_LIMIT,
      ),
    );

    if (!isActive()) return false;

    for (const event of events) subscription.onEvent(event);
    if (events.length < RECONNECT_REPLAY_PAGE_LIMIT) return true;

    const oldestCreatedAt = events[0]?.created_at;
    if (oldestCreatedAt === undefined || oldestCreatedAt <= since) return true;

    pageUntil =
      oldestCreatedAt < pageUntil ? oldestCreatedAt : oldestCreatedAt - 1;
  }
  return true;
}

export async function replayLiveSubscriptions({
  subscriptions,
  sendRaw,
  requestHistory,
  now = Math.floor(Date.now() / 1_000),
  pageReplayConcurrency = RECONNECT_REPLAY_PAGE_CONCURRENCY,
  visibleChannelId = null,
  replayBatchSize = REPLAY_BATCH_SIZE,
  interBatchDelayMs = REPLAY_INTER_BATCH_DELAY_MS,
  setTimeoutFn = (fn: () => void, ms: number) =>
    window.setTimeout(fn, ms) as unknown as number,
  isActive = () => true,
}: {
  subscriptions: Map<string, RelaySubscription>;
  sendRaw: (payload: unknown[]) => Promise<void>;
  requestHistory: (filter: RelaySubscriptionFilter) => Promise<RelayEvent[]>;
  now?: number;
  pageReplayConcurrency?: number;
  /** Channel currently visible in the UI — its subscriptions go in the first batch. */
  visibleChannelId?: string | null;
  /** Max subscriptions per REQ burst (injectable for tests). */
  replayBatchSize?: number;
  /** Milliseconds between bursts (injectable for tests). */
  interBatchDelayMs?: number;
  /** setTimeout implementation (injectable for tests). */
  setTimeoutFn?: (fn: () => void, ms: number) => number;
  /**
   * Returns false when the connection that initiated this replay has been
   * superseded by a newer one. After the gate await resumes, a stale replay
   * must not double-send REQs on the live socket.
   */
  isActive?: () => boolean;
}) {
  // If the relay has signalled back-pressure, wait for the gate to clear
  // before blasting a full set of REQs that would immediately be rate-limited.
  if (isRateLimited()) await waitForRateLimit();

  // A newer connection may have replayed while this one was suspended at the
  // gate — abort silently to avoid double-sending every REQ on the live socket.
  if (!isActive()) return;

  const replayRequests = Array.from(subscriptions.entries())
    .filter(
      (
        entry,
      ): entry is [string, Extract<RelaySubscription, { mode: "live" }>] =>
        entry[1].mode === "live",
    )
    .map(([subId, subscription]) => {
      const cursorSince =
        subscription.lastSeenCreatedAt === undefined
          ? undefined
          : Math.max(
              0,
              subscription.lastSeenCreatedAt - RECONNECT_REPLAY_SKEW_SECS,
            );
      // A pinned floor from a previously failed backfill takes precedence
      // over the cursor: live events kept advancing `lastSeenCreatedAt`
      // while the older window stayed unresolved, and starting from the
      // cursor would skip it permanently.
      const replaySince =
        cursorSince === undefined
          ? subscription.pendingReplaySince
          : Math.min(cursorSince, subscription.pendingReplaySince ?? Infinity);
      const shouldPageReplay =
        replaySince !== undefined &&
        shouldPageReconnectReplay(subscription.filter);

      return { subId, subscription, replaySince, shouldPageReplay };
    });

  // Sort the visible channel's subscriptions first so the user sees their
  // active channel recover before others on degraded networks.
  if (visibleChannelId !== null) {
    replayRequests.sort((a, b) => {
      const aVis =
        (a.subscription.filter["#h"] as string[] | undefined)?.includes(
          visibleChannelId,
        ) ?? false;
      const bVis =
        (b.subscription.filter["#h"] as string[] | undefined)?.includes(
          visibleChannelId,
        ) ?? false;
      if (aVis === bVis) return 0;
      return aVis ? -1 : 1;
    });
  }

  // Send live REQs in capped batches with inter-batch delays to avoid
  // triggering per-pubkey admission control on degraded/recovering connections.
  for (let i = 0; i < replayRequests.length; i += replayBatchSize) {
    // Re-check the gate before every batch: a previous batch may have triggered
    // admission control and armed the gate mid-replay. Wait for it to clear,
    // then verify the connection is still current — a newer connection may have
    // replayed while we were suspended.
    if (isRateLimited()) await waitForRateLimit();
    if (!isActive()) return;
    const batch = replayRequests.slice(i, i + replayBatchSize);
    await Promise.all(
      batch.map(({ subId, subscription, replaySince, shouldPageReplay }) =>
        sendRaw([
          "REQ",
          subId,
          shouldPageReplay
            ? subscription.filter
            : buildReconnectReplayFilter(subscription.filter, replaySince),
        ]),
      ),
    );
    if (i + replayBatchSize < replayRequests.length) {
      await new Promise<void>((resolve) =>
        setTimeoutFn(resolve, interBatchDelayMs),
      );
    }
  }

  await runWithConcurrency(
    replayRequests.filter(
      (
        request,
      ): request is typeof request & {
        replaySince: number;
        shouldPageReplay: true;
      } => request.shouldPageReplay && request.replaySince !== undefined,
    ),
    pageReplayConcurrency,
    async ({ subId, subscription, replaySince }) => {
      // Backfill is best-effort: a failure here (typically a `rate-limited:`
      // CLOSED on a history REQ) must never escape to the session and tear
      // down the healthy, authenticated socket carrying the live REQs — that
      // is the connect→drop flap loop. Retry behind the gate a bounded number
      // of times, then degrade to live-only for this connection.
      //
      // Pin the window's lower bound before the first attempt: events on the
      // already-restored live REQ advance `lastSeenCreatedAt` independently
      // of backfill success, so without the pin an exhausted backfill
      // followed by one live event would make the next reconnect skip the
      // unresolved window permanently. Cleared only on a completed pass.
      subscription.pendingReplaySince = replaySince;
      for (let attempt = 1; attempt <= PAGE_REPLAY_MAX_ATTEMPTS; attempt++) {
        try {
          const completed = await replayReconnectHistoryPages({
            subscription,
            since: replaySince,
            until: now,
            // Both guards are required. The identity check catches the sub
            // being torn down/replaced; the outer isActive() catches
            // connection supersession, which bumps the generation while the
            // SAME subscription key and object survive in the map — identity
            // alone stays true and a stale pass could complete and clear the
            // floor the superseding connection needs.
            isActive: () =>
              isActive() && subscriptions.get(subId) === subscription,
            requestHistory,
          });
          // A stale-connection abort is NOT completion: the superseding
          // connection shares this subscription object and still needs the
          // pinned floor for its own replay. Only a genuinely completed
          // window may release it.
          if (completed) subscription.pendingReplaySince = undefined;
          return;
        } catch (error) {
          console.warn(
            `[reconnect replay] history backfill attempt ${attempt}/${PAGE_REPLAY_MAX_ATTEMPTS} failed for ${subId}:`,
            error,
          );
          if (attempt === PAGE_REPLAY_MAX_ATTEMPTS) return;
          // The failed REQ's CLOSED handler arms the rate-limit gate before
          // rejecting; wait for it (no-op when the failure wasn't back-pressure)
          // and re-check that this replay's connection is still current.
          if (isRateLimited()) await waitForRateLimit();
          if (subscriptions.get(subId) !== subscription || !isActive()) return;
        }
      }
    },
  );
}
