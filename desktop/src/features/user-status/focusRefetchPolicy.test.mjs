import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import {
  focusManager,
  QueryClient,
  QueryObserver,
} from "@tanstack/react-query";

import { userStatusFocusRefetchPolicy } from "./hooks.ts";

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

test("user-status: skips fresh focus refetch", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: userStatusFocusRefetchPolicy.staleTime - 1_000,
      policy: userStatusFocusRefetchPolicy,
    }),
    0,
  );
});

test("user-status: does not refetch stale data on focus", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: userStatusFocusRefetchPolicy.staleTime + 1,
      policy: userStatusFocusRefetchPolicy,
    }),
    0,
  );
});
