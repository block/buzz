import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import {
  focusManager,
  QueryClient,
  QueryObserver,
} from "@tanstack/react-query";

import { CHANNEL_TEMPLATES_FOCUS_STALE_TIME_MS } from "./hooks.ts";

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

test("channel-templates: skips fresh focus refetch", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: CHANNEL_TEMPLATES_FOCUS_STALE_TIME_MS - 1_000,
      staleTime: CHANNEL_TEMPLATES_FOCUS_STALE_TIME_MS,
    }),
    0,
  );
});

test("channel-templates: refetches genuinely stale data on focus", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: CHANNEL_TEMPLATES_FOCUS_STALE_TIME_MS + 1,
      staleTime: CHANNEL_TEMPLATES_FOCUS_STALE_TIME_MS,
    }),
    1,
  );
});
