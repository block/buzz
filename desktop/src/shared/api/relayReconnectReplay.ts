import { CHANNEL_EVENT_KINDS } from "@/shared/constants/kinds";
import type {
  RelaySubscription,
  RelaySubscriptionFilter,
} from "@/shared/api/relayClientShared";
import type { RelayEvent } from "@/shared/api/types";

const RECONNECT_REPLAY_SKEW_SECS = 5;
export const RECONNECT_REPLAY_PAGE_LIMIT = 500;

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
}: {
  subscriptions: Map<string, RelaySubscription>;
  sendRaw: (payload: unknown[]) => Promise<void>;
  requestHistory: (filter: RelaySubscriptionFilter) => Promise<RelayEvent[]>;
  now?: number;
}) {
  for (const [subId, subscription] of subscriptions) {
    if (subscription.mode !== "live") continue;

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
    await sendRaw([
      "REQ",
      subId,
      shouldPageReplay
        ? subscription.filter
        : buildReconnectReplayFilter(subscription.filter, replaySince),
    ]);

    if (shouldPageReplay) {
      await replayReconnectHistoryPages({
        subscription,
        since: replaySince,
        until: now,
        isActive: () => subscriptions.get(subId) === subscription,
        requestHistory,
      });
    }
  }
}
