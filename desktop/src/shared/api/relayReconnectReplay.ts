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
}) {
  let pageUntil = until;

  while (pageUntil >= since) {
    if (!isActive()) return;

    const events = await requestHistory(
      buildReconnectReplayFilter(
        subscription.filter,
        since,
        pageUntil,
        RECONNECT_REPLAY_PAGE_LIMIT,
      ),
    );

    if (!isActive()) return;

    for (const event of events) subscription.onEvent(event);
    if (events.length < RECONNECT_REPLAY_PAGE_LIMIT) return;

    const oldestCreatedAt = events[0]?.created_at;
    if (oldestCreatedAt === undefined || oldestCreatedAt <= since) return;

    pageUntil =
      oldestCreatedAt < pageUntil ? oldestCreatedAt : oldestCreatedAt - 1;
  }
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
      const replaySince =
        subscription.lastSeenCreatedAt === undefined
          ? undefined
          : Math.max(
              0,
              subscription.lastSeenCreatedAt - RECONNECT_REPLAY_SKEW_SECS,
            );
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
      await replayReconnectHistoryPages({
        subscription,
        since: replaySince,
        until: now,
        isActive: () => subscriptions.get(subId) === subscription,
        requestHistory,
      });
    },
  );
}
