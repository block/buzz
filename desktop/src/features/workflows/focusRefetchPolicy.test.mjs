import assert from "node:assert/strict";
import { afterEach, test } from "node:test";

import {
  focusManager,
  QueryClient,
  QueryObserver,
} from "@tanstack/react-query";

import {
  workflowListFocusRefetchPolicy,
  workflowRunsFocusRefetchPolicy,
  runApprovalsFocusRefetchPolicy,
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

test("workflow-list: skips focus refetch when data is fresh (< 10s)", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: workflowListFocusRefetchPolicy.staleTime - 1_000,
      policy: workflowListFocusRefetchPolicy,
    }),
    0,
  );
});

test("workflow-list: refetches on focus when data is stale (> 10s)", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: workflowListFocusRefetchPolicy.staleTime + 1,
      policy: workflowListFocusRefetchPolicy,
    }),
    1,
  );
});

test("workflow-runs: skips focus refetch when data is fresh (< 10s)", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: workflowRunsFocusRefetchPolicy.staleTime - 1_000,
      policy: workflowRunsFocusRefetchPolicy,
    }),
    0,
  );
});

test("workflow-runs: refetches on focus when data is stale (> 10s)", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: workflowRunsFocusRefetchPolicy.staleTime + 1,
      policy: workflowRunsFocusRefetchPolicy,
    }),
    1,
  );
});

test("run-approvals: skips focus refetch when data is fresh (< 5 min)", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: runApprovalsFocusRefetchPolicy.staleTime - 1_000,
      policy: runApprovalsFocusRefetchPolicy,
    }),
    0,
  );
});

test("run-approvals: refetches on focus when data is stale (> 5 min)", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: runApprovalsFocusRefetchPolicy.staleTime + 1,
      policy: runApprovalsFocusRefetchPolicy,
    }),
    1,
  );
});
