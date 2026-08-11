import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import {
  focusManager,
  QueryClient,
  QueryObserver,
} from "@tanstack/react-query";

import {
  USER_STATUS_FOCUS_STALE_TIME_MS,
  USER_STATUS_REFETCH_INTERVAL_MS,
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

test("user-status: polling constant is locked at 2 minutes", () => {
  assert.equal(USER_STATUS_REFETCH_INTERVAL_MS, 120_000);
});

test("user-status: skips fresh focus refetch", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: USER_STATUS_FOCUS_STALE_TIME_MS - 1_000,
      staleTime: USER_STATUS_FOCUS_STALE_TIME_MS,
    }),
    0,
  );
});

test("user-status: refetches genuinely stale data on focus", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: USER_STATUS_FOCUS_STALE_TIME_MS + 1,
      staleTime: USER_STATUS_FOCUS_STALE_TIME_MS,
    }),
    1,
  );
});
