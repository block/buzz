import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import {
  focusManager,
  QueryClient,
  QueryObserver,
} from "@tanstack/react-query";

import {
  AGENTS_FOCUS_STALE_TIME_MS,
  MANAGED_AGENT_LOG_FOCUS_STALE_TIME_MS,
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

test("agents: skips fresh focus refetch", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: AGENTS_FOCUS_STALE_TIME_MS - 1_000,
      staleTime: AGENTS_FOCUS_STALE_TIME_MS,
    }),
    0,
  );
});

test("agents: refetches genuinely stale data on focus", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: AGENTS_FOCUS_STALE_TIME_MS + 1,
      staleTime: AGENTS_FOCUS_STALE_TIME_MS,
    }),
    1,
  );
});

test("managed-agent-log: skips focus refetch when data is fresher than one poll tick", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: MANAGED_AGENT_LOG_FOCUS_STALE_TIME_MS - 1_000,
      staleTime: MANAGED_AGENT_LOG_FOCUS_STALE_TIME_MS,
    }),
    0,
  );
});

test("managed-agent-log: refetches on focus when data is older than one poll tick", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: MANAGED_AGENT_LOG_FOCUS_STALE_TIME_MS + 1,
      staleTime: MANAGED_AGENT_LOG_FOCUS_STALE_TIME_MS,
    }),
    1,
  );
});
