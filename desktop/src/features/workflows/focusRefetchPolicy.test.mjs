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

test("workflow-list: does not refetch stale data on focus", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: workflowListFocusRefetchPolicy.staleTime + 1,
      policy: workflowListFocusRefetchPolicy,
    }),
    0,
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

test("workflow-runs: does not refetch stale data on focus", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: workflowRunsFocusRefetchPolicy.staleTime + 1,
      policy: workflowRunsFocusRefetchPolicy,
    }),
    0,
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

test("run-approvals: does not refetch stale data on focus", async () => {
  assert.equal(
    await focusRefetchCount({
      ageMs: runApprovalsFocusRefetchPolicy.staleTime + 1,
      policy: runApprovalsFocusRefetchPolicy,
    }),
    0,
  );
});

test("foreground refresh targets workflow gaps and repo sync", async () => {
  const { shouldRefreshQueryOnForeground } = await import("./hooks.ts");
  assert.equal(shouldRefreshQueryOnForeground(["workflows", "c"], []), true);
  assert.equal(
    shouldRefreshQueryOnForeground(["workflows-all", "c"], []),
    true,
  );
  assert.equal(
    shouldRefreshQueryOnForeground(["workflow-runs", "w"], []),
    true,
  );
  assert.equal(
    shouldRefreshQueryOnForeground(
      ["workflow-runs", "w"],
      [{ status: "completed" }],
    ),
    true,
  );
  assert.equal(
    shouldRefreshQueryOnForeground(
      ["workflow-runs", "w"],
      [{ status: "running" }],
    ),
    false,
  );
  assert.equal(
    shouldRefreshQueryOnForeground(
      ["project", "p", "repo-sync-status", "default"],
      undefined,
    ),
    true,
  );
  assert.equal(
    shouldRefreshQueryOnForeground(["project", "p", "issues"], []),
    false,
  );
  assert.equal(shouldRefreshQueryOnForeground(["run-approvals"], []), false);
});

test("foreground refresh query contract includes only stale active targets", async () => {
  const { shouldRefreshQueryOnForeground } = await import("./hooks.ts");
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const fetches = [];
  const subscribe = (queryKey, data, updatedAt) => {
    queryClient.setQueryData(queryKey, data, { updatedAt });
    const observer = new QueryObserver(queryClient, {
      queryKey,
      queryFn: async () => {
        fetches.push(queryKey.join(":"));
        return data;
      },
      refetchOnMount: false,
      staleTime: 10_000,
    });
    return observer.subscribe(() => {});
  };
  const staleAt = Date.now() - 20_000;
  const freshAt = Date.now();
  const unsubscribers = [
    subscribe(["workflows", "stale-active"], [], staleAt),
    subscribe(["workflows", "fresh-active"], [], freshAt),
    subscribe(
      ["project", "p", "repo-sync-status", "default"],
      { aheadCount: 1 },
      staleAt,
    ),
    subscribe(
      ["workflow-runs", "inactive"],
      [{ status: "completed" }],
      staleAt,
    ),
    subscribe(["workflow-runs", "active"], [{ status: "running" }], staleAt),
  ];
  queryClient.setQueryData(["workflows-all", "inactive-cache"], [], {
    updatedAt: staleAt,
  });

  await queryClient.refetchQueries({
    predicate: (query) =>
      shouldRefreshQueryOnForeground(query.queryKey, query.state.data),
    stale: true,
    type: "active",
  });

  assert.deepEqual(fetches.sort(), [
    "project:p:repo-sync-status:default",
    "workflow-runs:inactive",
    "workflows:stale-active",
  ]);
  for (const unsubscribe of unsubscribers) unsubscribe();
  queryClient.clear();
});
