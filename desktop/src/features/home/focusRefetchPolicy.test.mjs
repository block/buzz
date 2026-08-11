import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import {
  focusManager,
  QueryClient,
  QueryObserver,
} from "@tanstack/react-query";

import {
  CHANNELS_FOCUS_STALE_TIME_MS,
  CHANNELS_REFETCH_INTERVAL_MS,
} from "@/features/channels/hooks.ts";
import {
  HOME_FEED_FOCUS_STALE_TIME_MS,
  HOME_FEED_REFETCH_INTERVAL_MS,
} from "./hooks.ts";

afterEach(() => {
  focusManager.setFocused(undefined);
});

async function focusRefetchCount({ ageMs, staleTime }) {
  focusManager.setFocused(false);
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  queryClient.mount();

  const queryKey = ["focus-refetch-policy", staleTime, ageMs];
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
    refetchOnWindowFocus: true,
    staleTime,
  });
  const unsubscribe = observer.subscribe(() => {});

  focusManager.setFocused(true);
  await new Promise((resolve) => setImmediate(resolve));

  unsubscribe();
  queryClient.unmount();
  return fetchCount;
}

for (const policy of [
  {
    name: "channels",
    staleTime: CHANNELS_FOCUS_STALE_TIME_MS,
    refetchInterval: CHANNELS_REFETCH_INTERVAL_MS,
    expectedRefetchInterval: 60_000,
  },
  {
    name: "home feed",
    staleTime: HOME_FEED_FOCUS_STALE_TIME_MS,
    refetchInterval: HOME_FEED_REFETCH_INTERVAL_MS,
    expectedRefetchInterval: 30_000,
  },
]) {
  test(`${policy.name} skips fresh focus refetch and preserves polling`, async () => {
    assert.equal(policy.refetchInterval, policy.expectedRefetchInterval);
    assert.equal(
      await focusRefetchCount({
        ageMs: policy.staleTime - 1_000,
        staleTime: policy.staleTime,
      }),
      0,
    );
  });

  test(`${policy.name} refetches genuinely stale data on focus`, async () => {
    assert.equal(
      await focusRefetchCount({
        ageMs: policy.staleTime + 1,
        staleTime: policy.staleTime,
      }),
      1,
    );
  });
}
