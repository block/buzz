import type { QueryClient } from "@tanstack/react-query";
import type { ChannelWindowStore } from "./channelWindowStore";
import { channelWindowKey } from "./messageQueryKeys";

// TanStack reuses one AbortSignal across a fetch's retry attempts, but creates
// a new signal for every later fetch. Never leave destructive intent in the
// durable window after the request has claimed it.
let latestRequests = new WeakSet<AbortSignal>();

/** Retire request intent along with the other community-scoped state. */
export function resetChannelWindowRefreshIntents() {
  latestRequests = new WeakSet();
}

/** Claim queued latest-only intent once, retaining it only for this fetch’s retries. */
export function consumeChannelWindowRefreshIntent(
  client: QueryClient,
  channelId: string,
  signal: AbortSignal,
) {
  signal.throwIfAborted();
  const key = channelWindowKey(channelId);
  const current = client.getQueryData<ChannelWindowStore>(key);
  if (current?.refreshLatestOnly) {
    latestRequests.add(signal);
    client.setQueryData(key, { ...current, refreshLatestOnly: undefined });
  }
  return latestRequests.has(signal);
}
