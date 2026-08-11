import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import {
  focusManager,
  QueryClient,
  QueryObserver,
} from "@tanstack/react-query";

import {
  WORKFLOWS_FOCUS_STALE_TIME_MS,
  WORKFLOW_RUNS_FOCUS_STALE_TIME_MS,
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

test("workflows: skips fresh focus refetch", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: WORKFLOWS_FOCUS_STALE_TIME_MS - 1_000,
      staleTime: WORKFLOWS_FOCUS_STALE_TIME_MS,
    }),
    0,
  );
});

test("workflows: refetches genuinely stale data on focus", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: WORKFLOWS_FOCUS_STALE_TIME_MS + 1,
      staleTime: WORKFLOWS_FOCUS_STALE_TIME_MS,
    }),
    1,
  );
});

test("workflow-runs: skips focus refetch when data is fresh (< 10s)", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: WORKFLOW_RUNS_FOCUS_STALE_TIME_MS - 1_000,
      staleTime: WORKFLOW_RUNS_FOCUS_STALE_TIME_MS,
    }),
    0,
  );
});

test("workflow-runs: refetches on focus when data is stale (> 10s)", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: WORKFLOW_RUNS_FOCUS_STALE_TIME_MS + 1,
      staleTime: WORKFLOW_RUNS_FOCUS_STALE_TIME_MS,
    }),
    1,
  );
});
