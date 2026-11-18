/**
 * Deadline-retry contract, exercised under the production QueryClient:
 * a read that hit the relay's `query timed out` deadline is fetched once; an
 * ordinary failure, including a client-side request timeout, keeps its retries.
 * Removing the predicate from `createBuzzQueryClient` or `useThreadReplies`
 * makes the deadline cases here fetch more than once.
 */

import assert from "node:assert/strict";
import { mock } from "node:test";
import test from "node:test";
import { registerHooks } from "node:module";

import { JSDOM } from "jsdom";

globalThis.__tauriGetThreadReplies = async () => ({
  events: [],
  nextCursor: null,
});

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier === "@/shared/api/tauri") {
      return { shortCircuit: true, url: "buzz-deadline-stub:tauri" };
    }
    return nextResolve(specifier, context);
  },
  load(url, context, nextLoad) {
    if (url === "buzz-deadline-stub:tauri") {
      return {
        format: "module",
        shortCircuit: true,
        source: `export async function getThreadReplies(...args) {
  return globalThis.__tauriGetThreadReplies(...args);
}
export default {};`,
      };
    }
    return nextLoad(url, context);
  },
});

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
  pretendToBeVisual: true,
});
dom.window.document.hasFocus = () => true;
Object.assign(globalThis, {
  IS_REACT_ACT_ENVIRONMENT: true,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  window: dom.window,
});

const { act, cleanup, renderHook } = await import("@testing-library/react");
const { createElement } = await import("react");
const { QueryClientProvider, useQuery } = await import("@tanstack/react-query");
const { createBuzzQueryClient } = await import("@/shared/api/queryClient");
const { useThreadReplies, useThreadRepliesForRoots } = await import(
  "./useThreadReplies.ts"
);

const CASES = [
  {
    name: "client request timeout",
    message: "relay unreachable: request timed out",
    deadline: false,
  },
  {
    name: "relay deadline",
    message: "relay returned 503 Service Unavailable: query timed out",
    deadline: true,
  },
  {
    name: "ordinary error",
    message: "relay returned 500: internal server error",
    deadline: false,
  },
];

// Mount `useHook` under the production client, drive every retry delay with
// fake timers until the query settles in error, and return the fetch count.
async function fetchCountUntilError(useHook, isSettled) {
  mock.timers.enable({ apis: ["setTimeout"] });
  const queryClient = createBuzzQueryClient();
  const wrapper = ({ children }) =>
    createElement(QueryClientProvider, { client: queryClient }, children);
  try {
    const { result, unmount } = renderHook(useHook, { wrapper });
    for (let i = 0; i < 20 && !isSettled(result.current); i++) {
      mock.timers.tick(60_000);
      await act(async () => {
        await new Promise((resolve) => setImmediate(resolve));
      });
    }
    assert.ok(isSettled(result.current), "query never settled in error");
    unmount();
  } finally {
    cleanup();
    queryClient.clear();
    mock.timers.reset();
  }
}

for (const { name, message, deadline } of CASES) {
  test(`createBuzzQueryClient default retry: ${name}`, async () => {
    let calls = 0;
    await fetchCountUntilError(
      () =>
        useQuery({
          queryKey: ["deadline-default", name],
          queryFn: async () => {
            calls += 1;
            throw new Error(message);
          },
        }),
      (r) => r.isError,
    );
    assert.equal(calls, deadline ? 1 : 2);
  });

  test(`useThreadRepliesForRoots (3 roots) retry: ${name}`, async () => {
    const calls = new Map();
    globalThis.__tauriGetThreadReplies = async (rootId) => {
      calls.set(rootId, (calls.get(rootId) ?? 0) + 1);
      throw new Error(message);
    };
    const channel = { id: `chan-roots-${name}`, channelType: "group" };
    const roots = ["root-a", "root-b", "root-c"];
    await fetchCountUntilError(
      () => useThreadRepliesForRoots(channel, roots),
      (r) => r.isError && !r.isPending,
    );
    assert.deepEqual(
      Object.fromEntries(calls),
      Object.fromEntries(roots.map((root) => [root, deadline ? 1 : 2])),
    );
  });

  test(`useThreadReplies single-root retry: ${name}`, async () => {
    let calls = 0;
    globalThis.__tauriGetThreadReplies = async () => {
      calls += 1;
      throw new Error(message);
    };
    const channel = { id: `chan-single-${name}`, channelType: "group" };
    await fetchCountUntilError(
      () => useThreadReplies(channel, "root-single"),
      (r) => r.isError,
    );
    // useThreadReplies keeps its own `< 3` retries for ordinary failures.
    assert.equal(calls, deadline ? 1 : 4);
  });
}
