import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import {
  focusManager,
  QueryClient,
  QueryObserver,
} from "@tanstack/react-query";

import {
  channelsFocusRefetchPolicy,
  CHANNELS_REFETCH_INTERVAL_MS,
} from "@/features/channels/hooks.ts";
import {
  homeFeedFocusRefetchPolicy,
  HOME_FEED_REFETCH_INTERVAL_MS,
} from "./hooks.ts";

afterEach(() => {
  focusManager.setFocused(undefined);
});

async function focusRefetchCount({ ageMs, policy }) {
  focusManager.setFocused(false);
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  queryClient.mount();

  const queryKey = ["focus-refetch-policy", policy.staleTime, ageMs];
  queryClient.setQueryData(queryKey, "cached", {
    updatedAt: Date.now() - ageMs,
  });
  let fetchCount = 0;
  const observer = new QueryObserver(queryClient, {
    queryKey,
    queryFn: async () => {
      fetchCount += 1;
      return "refetched";
    },
    // Keep setup from fetching stale cache before the simulated focus return.
    refetchOnMount: false,
    ...policy,
  });
  const unsubscribe = observer.subscribe(() => {});

  focusManager.setFocused(true);
  await new Promise((resolve) => setImmediate(resolve));

  unsubscribe();
  queryClient.unmount();
  return fetchCount;
}

for (const entry of [
  {
    name: "channels",
    focusPolicy: channelsFocusRefetchPolicy,
    refetchInterval: CHANNELS_REFETCH_INTERVAL_MS,
    expectedRefetchInterval: 60_000,
  },
  {
    name: "home feed",
    focusPolicy: homeFeedFocusRefetchPolicy,
    refetchInterval: HOME_FEED_REFETCH_INTERVAL_MS,
    expectedRefetchInterval: 30_000,
  },
]) {
  test(`${entry.name} skips fresh focus refetch and preserves polling`, async () => {
    assert.equal(entry.refetchInterval, entry.expectedRefetchInterval);
    assert.equal(
      await focusRefetchCount({
        ageMs: entry.focusPolicy.staleTime - 1_000,
        policy: entry.focusPolicy,
      }),
      0,
    );
  });

  test(`${entry.name} refetches genuinely stale data on focus`, async () => {
    assert.equal(
      await focusRefetchCount({
        ageMs: entry.focusPolicy.staleTime + 1,
        policy: entry.focusPolicy,
      }),
      1,
    );
  });
}
