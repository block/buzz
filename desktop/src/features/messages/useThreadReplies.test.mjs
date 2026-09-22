/**
 * Behavioral tests for loadThreadReplies / useThreadReplies validation contract.
 *
 * These tests exercise the real fetch logic and terminal-policy paths through
 * an injected fake fetcher — no Tauri bridge required. Each test is
 * load-bearing: removing the throw in loadThreadReplies OR removing the
 * exhaustion-set check causes a specific test to fail.
 *
 * Hook-level tests (mounted-thread target change and exhaustion terminal state)
 * use a real QueryClientProvider and renderHook so mutations to the production
 * wiring — the useEffect invalidation and the query-fn exhaustion write —
 * turn the suite red.
 */

import assert from "node:assert/strict";
import { mock } from "node:test";
import test from "node:test";
import { registerHooks } from "node:module";

import { JSDOM } from "jsdom";

// ── Tauri stub ────────────────────────────────────────────────────────────────
// Stub @/shared/api/tauri before any module that imports it loads.
// Hook-level tests set globalThis.__tauriGetThreadReplies before each run;
// the stub delegates to that global so tests control the fetcher without
// changing the production hook signature.

globalThis.__tauriGetThreadReplies = async () => ({
  events: [],
  nextCursor: null,
});

registerHooks({
  resolve(specifier, context, nextResolve) {
    if (specifier === "@/shared/api/tauri") {
      return { shortCircuit: true, url: "buzz-thread-stub:tauri" };
    }
    if (specifier.startsWith("buzz-thread-stub:")) {
      return { shortCircuit: true, url: specifier };
    }
    return nextResolve(specifier, context);
  },
  load(url, context, nextLoad) {
    if (url === "buzz-thread-stub:tauri") {
      return {
        format: "module",
        shortCircuit: true,
        // Delegate to test-controlled global so each hook test can swap the
        // fetcher without reloading the cached module.
        source: `
export async function getThreadReplies(rootId, channelId, options) {
  return globalThis.__tauriGetThreadReplies(rootId, channelId, options);
}
export default {};
`,
      };
    }
    return nextLoad(url, context);
  },
});

// ── DOM setup ─────────────────────────────────────────────────────────────────

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

Object.assign(globalThis, {
  IS_REACT_ACT_ENVIRONMENT: true,
  document: dom.window.document,
  HTMLElement: dom.window.HTMLElement,
  window: dom.window,
});

// ── Fake event builder ────────────────────────────────────────────────────────

let seq = 0;
function fakeEvent(id) {
  return {
    id,
    pubkey: "a".repeat(64),
    kind: 9,
    created_at: ++seq,
    content: "test",
    tags: [],
    sig: "sig",
  };
}

function singlePage(events) {
  return { events, nextCursor: null };
}

// ── Fake QueryClient (for loadThreadReplies unit tests) ───────────────────────

function makeQueryClient(initial = undefined) {
  let store = initial;
  return {
    getQueryData: () => store,
    setQueryData: (_key, value) => {
      store = value;
    },
    invalidateQueries: () => Promise.resolve(),
  };
}

// ── Test: throws ThreadExpectedEventMissingError when target is absent ────────
//
// Load-bearing: if the `throw new ThreadExpectedEventMissingError(...)` line in
// loadThreadReplies is removed, this test FAILS because no error is thrown.

test("loadThreadReplies throws ThreadExpectedEventMissingError when expected event is absent", async () => {
  const { loadThreadReplies, ThreadExpectedEventMissingError } = await import(
    "./useThreadReplies.ts"
  );

  const qc = makeQueryClient();
  const reply = fakeEvent("reply-1");
  const fetcher = async () => singlePage([reply]);

  await assert.rejects(
    () =>
      loadThreadReplies(
        qc,
        "chan-1",
        "root-1",
        "evt-missing",
        new Set(),
        fetcher,
      ),
    (err) => {
      assert.ok(
        err instanceof ThreadExpectedEventMissingError,
        `Expected ThreadExpectedEventMissingError, got ${err.constructor.name}`,
      );
      assert.equal(err.expectedEventId, "evt-missing");
      return true;
    },
  );
});

// ── Test: does NOT throw when expected event is present ───────────────────────

test("loadThreadReplies resolves normally when expected event is present", async () => {
  const { loadThreadReplies } = await import("./useThreadReplies.ts");

  const qc = makeQueryClient();
  const target = fakeEvent("evt-present");
  const fetcher = async () => singlePage([target]);

  const result = await loadThreadReplies(
    qc,
    "chan-2",
    "root-2",
    "evt-present",
    new Set(),
    fetcher,
  );

  assert.ok(
    result.some((e) => e.id === "evt-present"),
    "result must contain the expected event",
  );
});

// ── Test: exhaustedTargets suppresses throw — returns available replies ────────
//
// Load-bearing: if the `exhaustedTargets?.has(expectedEventId)` guard is removed
// from loadThreadReplies, this test FAILS because the function throws instead of
// resolving.

test("loadThreadReplies returns fetched replies when target is in exhaustedTargets", async () => {
  const { loadThreadReplies } = await import("./useThreadReplies.ts");

  const qc = makeQueryClient();
  const reply = fakeEvent("reply-2");
  const fetcher = async () => singlePage([reply]);
  const exhausted = new Set(["missing-target"]);

  const result = await loadThreadReplies(
    qc,
    "chan-3",
    "root-3",
    "missing-target",
    exhausted,
    fetcher,
  );

  assert.ok(Array.isArray(result), "must return an array");
  assert.ok(
    result.some((e) => e.id === "reply-2"),
    "must include fetched replies",
  );
});

// ── Test: no expectedEventId — always resolves without throw ─────────────────

test("loadThreadReplies resolves normally with no expectedEventId", async () => {
  const { loadThreadReplies } = await import("./useThreadReplies.ts");

  const qc = makeQueryClient();
  const reply = fakeEvent("reply-3");
  const fetcher = async () => singlePage([reply]);

  const result = await loadThreadReplies(
    qc,
    "chan-4",
    "root-4",
    null,
    new Set(),
    fetcher,
  );

  assert.ok(result.some((e) => e.id === "reply-3"));
});

// ── Test: multi-page fetch assembles results from all pages ──────────────────

test("loadThreadReplies aggregates events across multiple pages", async () => {
  const { loadThreadReplies } = await import("./useThreadReplies.ts");

  const qc = makeQueryClient();
  const page1Events = [fakeEvent("p1-a"), fakeEvent("p1-b")];
  const page2Events = [fakeEvent("p2-a"), fakeEvent("p2-b")];
  let calls = 0;
  const fetcher = async (_rootId, _channelId, { cursor }) => {
    calls += 1;
    if (cursor === null) {
      return {
        events: page1Events,
        nextCursor: { createdAt: 1, eventId: "p1-b" },
      };
    }
    return singlePage(page2Events);
  };

  const result = await loadThreadReplies(
    qc,
    "chan-5",
    "root-5",
    null,
    new Set(),
    fetcher,
  );

  assert.equal(calls, 2, "fetcher must be called for each page");
  const ids = new Set(result.map((e) => e.id));
  for (const evt of [...page1Events, ...page2Events]) {
    assert.ok(ids.has(evt.id), `result must include ${evt.id}`);
  }
});

// ── Test: query-fn writes exhaustion on the terminal attempt ──────────────────
//
// Load-bearing for the query-fn exhaustion write in useThreadReplies.
//
// The query-fn adds expectedEventId to exhaustedTargetsRef when attemptCount
// reaches the threshold (>= 3). loadThreadReplies then sees the target in
// exhaustedTargets and returns data instead of throwing. If the exhaustion
// write (`exhaustedTargetsRef.current.add(expectedEventId)`) is removed from
// the query-fn, the terminal attempt still throws and the hook stays in error.
//
// This test exercises the complete production path: real QueryClientProvider,
// real useThreadReplies hook, fake fetcher via globalThis stub, timer-driven
// backoff (retry: 3, retryDelay 1s/2s/4s). After the two retries, attempt 3
// writes exhaustion and loadThreadReplies resolves data synchronously.

test("useThreadReplies resolves to data after exhausting missing-event retries", async () => {
  mock.timers.enable({ apis: ["setTimeout"] });

  let queryClient;
  let unmount;
  let cleanup;

  try {
    const imported = await import("@testing-library/react");
    const act = imported.act;
    cleanup = imported.cleanup;
    const renderHook = imported.renderHook;
    const { createElement } = await import("react");
    const { QueryClient, QueryClientProvider } = await import(
      "@tanstack/react-query"
    );
    const { useThreadReplies } = await import("./useThreadReplies.ts");

    const reply = fakeEvent("reply-exhaustion");
    // Fetcher returns a reply but never the expected event — permanently absent.
    globalThis.__tauriGetThreadReplies = async () => singlePage([reply]);

    queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const channel = { id: "chan-ex", channelType: "group" };
    const wrapper = ({ children }) =>
      createElement(QueryClientProvider, { client: queryClient }, children);

    const hook = renderHook(
      () => useThreadReplies(channel, "root-ex", "evt-permanently-absent"),
      { wrapper },
    );
    unmount = hook.unmount;
    const { result } = hook;

    // Drive through the two backoff delays for attempts 1 and 2.
    // Attempt 1 throws → retryDelay 1s. Attempt 2 throws → retryDelay 2s.
    // Attempt 3 writes exhaustion pre-fetch → loadThreadReplies returns data.
    // Tick 5s per iteration (covers each delay); flush microtasks between
    // ticks so React and TanStack Query advance their state machines.
    for (let i = 0; i < 4; i++) {
      mock.timers.tick(5_000);
      await act(async () => {
        await new Promise((resolve) => setImmediate(resolve));
      });
    }

    // After the terminal attempt the query must be in success (data) state.
    assert.ok(
      !result.current.isError,
      `hook must not be in error state after exhaustion (isError=${result.current.isError})`,
    );
    assert.ok(
      Array.isArray(result.current.data),
      `hook must expose reply data after exhaustion (data=${JSON.stringify(result.current.data)})`,
    );
    assert.ok(
      result.current.data?.some((e) => e.id === reply.id),
      "data must include the fetched replies rather than being empty or errored",
    );
  } finally {
    // Always dispose — a failure before this point must not leak timers,
    // mounted hooks, or QueryClient cache timers that hold the event loop.
    try {
      unmount?.();
    } catch (_) {}
    try {
      cleanup?.();
    } catch (_) {}
    try {
      queryClient?.clear();
    } catch (_) {}
    mock.timers.reset();
  }
});

// ── Test: settled null→target change triggers retry and lands target data ──────
//
// Load-bearing for the useEffect invalidation seam AND for the TanStack effect-
// ordering fix in useThreadReplies.
//
// Required shape (Carl's requirement, P2 finding on PR #7188):
//   1. Mount with expectedEventId=null. Wait for the initial fetch to settle
//      with a non-target reply — query is success with data, no target active.
//   2. Rerender with expectedEventId="target-evt-settled" (same root, same key).
//      The useEffect must invalidate; the refetch must use the new queryFn
//      closure (capturing "target-evt-settled"), not the stale null closure.
//   3. First post-invalidation fetch returns a page WITHOUT the target —
//      loadThreadReplies throws ThreadExpectedEventMissingError. This is the
//      failure path the pre-fix code silently passed: old null closure
//      validated against null, settled success([]) without retrying.
//   4. TanStack retries; second post-invalidation fetch returns the target.
//      Hook settles success with the target event in data.
//
// Mutation checks:
//   - Remove invalidateQueries from the useEffect → hook never re-fetches,
//     data stays at initial non-target reply → target-bearing assertion red.
//   - Restore effect but revert useQuery back to before the effect (pre-fix
//     ordering) → refetch uses old null closure, settles success([]) without
//     the target or a retry → target-bearing assertion red.
//
// Also load-bearing for the ChannelScreen wiring: the source assertion verifies
// that ChannelScreen.tsx passes threadScrollTargetId as the third argument to
// useThreadReplies. Removing that argument fails the source check, catching the
// exact bypass that allowed notification routing to skip the missing-event check.

test("useThreadReplies null-to-target change retries on missing-target page and lands target data", async () => {
  let queryClient;
  let unmount;
  let cleanup;
  try {
    const imported = await import("@testing-library/react");
    const act = imported.act;
    cleanup = imported.cleanup;
    const renderHook = imported.renderHook;
    const { createElement } = await import("react");
    const { QueryClient, QueryClientProvider } = await import(
      "@tanstack/react-query"
    );
    const { useThreadReplies } = await import("./useThreadReplies.ts");
    const { readFile } = await import("node:fs/promises");

    const targetEvent = fakeEvent("target-evt-settled");
    const otherReply = fakeEvent("other-reply-settled");

    // Phase tracking:
    //   fetch 1 — initial null-target fetch: returns otherReply (no target active)
    //   fetch 2 — first post-invalidation fetch: returns otherReply only (target
    //             absent), triggering ThreadExpectedEventMissingError under the
    //             new closure (captures "target-evt-settled"). Under the old
    //             null closure this would silently settle without throwing.
    //   fetch 3+ — retry fetch: returns targetEvent (relay caught up)
    let fetchCount = 0;
    globalThis.__tauriGetThreadReplies = async () => {
      fetchCount += 1;
      if (fetchCount === 1) {
        // Initial null-target settle — no target validation needed.
        return singlePage([otherReply]);
      }
      if (fetchCount === 2) {
        // First post-invalidation fetch — target absent. Under the fixed code
        // the closure captures "target-evt-settled" and loadThreadReplies
        // throws ThreadExpectedEventMissingError → TanStack retries.
        // Under the pre-fix code (null closure) → settles success without retry.
        return singlePage([otherReply]);
      }
      // Retry fetch — relay has caught up, target available.
      return singlePage([targetEvent, otherReply]);
    };

    queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const channel = { id: "chan-settled", channelType: "group" };
    const wrapper = ({ children }) =>
      createElement(QueryClientProvider, { client: queryClient }, children);

    const hook = renderHook(
      ({ expectedEventId }) =>
        useThreadReplies(channel, "root-settled", expectedEventId),
      { wrapper, initialProps: { expectedEventId: null } },
    );
    unmount = hook.unmount;
    const { rerender, result } = hook;

    // Wait for the initial null-target fetch to settle.
    for (let i = 0; i < 20; i++) {
      await act(async () => {
        await new Promise((resolve) => setImmediate(resolve));
      });
      if (!result.current.isPending) break;
    }

    assert.ok(fetchCount >= 1, "initial fetch must have occurred");
    assert.equal(
      result.current.isError,
      false,
      "hook must be in success state after initial null-target settle",
    );

    // Supply the target — hook must invalidate, refetch with current closure,
    // detect missing target, retry, and land target-bearing data.
    // The hook uses retryDelay: (attempt) => Math.min(1_000 * 2 ** attempt, 30_000).
    // Attempt 0 retries after 1_000ms (real time). Wait up to 3s for the retry.
    await act(async () => {
      rerender({ expectedEventId: "target-evt-settled" });
    });

    // Poll for target-bearing success, waiting up to 3 seconds total.
    // The retry delay is 1s (attempt 0), so the hook should settle in ~1.1s.
    let settled = false;
    for (let i = 0; i < 40; i++) {
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 100));
      });
      if (
        !result.current.isPending &&
        !result.current.isError &&
        result.current.data?.some((e) => e.id === targetEvent.id)
      ) {
        settled = true;
        break;
      }
    }

    assert.ok(
      settled,
      `hook must settle with target data after retry within 4s ` +
        `(isError=${result.current.isError}, ` +
        `data ids: ${result.current.data?.map((e) => e.id).join(",")}, ` +
        `fetchCount=${fetchCount})`,
    );

    // Additional assertions on the settled state.
    assert.ok(
      !result.current.isError,
      `hook must not be in error state (error=${result.current.error})`,
    );
    assert.ok(
      Array.isArray(result.current.data),
      `hook must expose reply data (data=${JSON.stringify(result.current.data)})`,
    );
    assert.ok(
      result.current.data?.some((e) => e.id === targetEvent.id),
      `data must include the target event after retry ` +
        `(got ids: ${result.current.data?.map((e) => e.id).join(",")})`,
    );

    // At minimum two more fetches must have occurred: the invalidation fetch
    // (fetch 2, missing target → throw) and the retry (fetch 3+, target present).
    assert.ok(
      fetchCount >= 3,
      `missing-target retry must have fired (fetchCount=${fetchCount})`,
    );

    // ── ChannelScreen wiring assertion ────────────────────────────────────────
    // Fails if ChannelScreen stops passing threadScrollTargetId as the third
    // argument to useThreadReplies.
    const channelScreenSource = await readFile(
      new URL("../channels/ui/ChannelScreen.tsx", import.meta.url),
      "utf8",
    );
    assert.match(
      channelScreenSource,
      /useThreadReplies\s*\([^)]*threadScrollTargetId/s,
      "ChannelScreen.tsx must pass threadScrollTargetId as the third argument to useThreadReplies",
    );
  } finally {
    // Always dispose — a failure before this point must not leak mounted
    // hooks or QueryClient cache timers (gcTime: 1h) that hold the event loop.
    try {
      unmount?.();
    } catch (_) {}
    try {
      cleanup?.();
    } catch (_) {}
    try {
      queryClient?.clear();
    } catch (_) {}
  }
});

// ── Test: cold-fetch cancel-then-invalidate behavioral regression ─────────────
//
// Load-bearing real-hook test for the cancel-then-invalidate branch.
//
// Race: thread is cold (no cached data), a fetch is in-flight, then
// expectedEventId arrives from notification routing before the first page
// returns. The effect detects fetchStatus=fetching + status=pending and must
// cancel the in-flight fetch before invalidating, so the obsolete empty-page
// response cannot settle as authoritative before the new target's validation
// closure is active.
//
// Test shape (Thufir's deterministic probe):
//   1. Mount with expectedEventId=null and a gated fetcher (blocks until
//      released). Query is cold — fetchStatus=fetching, status=pending.
//   2. Rerender with expectedEventId="target-evt". Effect fires, sees pending
//      cold fetch, calls cancelQueries().then(invalidateQueries).
//   3. Release the gated fetcher returning [] (stale empty result). TanStack
//      discards the cancelled retryer — [] must not settle as success.
//   4. The replacement fetch (new closure, target active) runs; its fetcher
//      returns [targetEvent]. Hook settles success with the target.
//
// Mutation: replacing the cancel+invalidate branch with plain invalidateQueries
// causes the stale [] to settle before the replacement runs → hook is either
// stuck retrying (with retry:3) or resolves without the target → assertion red.

test("useThreadReplies cancels stale in-flight cold fetch when target arrives", async () => {
  let queryClient;
  let unmount;
  let cleanup;
  try {
    const imported = await import("@testing-library/react");
    const act = imported.act;
    cleanup = imported.cleanup;
    const renderHook = imported.renderHook;
    const { createElement } = await import("react");
    const { QueryClient, QueryClientProvider } = await import(
      "@tanstack/react-query"
    );
    const { useThreadReplies } = await import("./useThreadReplies.ts");

    // Gated fetcher: first call blocks until released; subsequent calls return
    // the target event immediately.
    const targetEvent = fakeEvent("target-evt");
    let releaseFirstFetch;
    let fetchCount = 0;
    globalThis.__tauriGetThreadReplies = async () => {
      fetchCount += 1;
      if (fetchCount === 1) {
        // First fetch blocks until the test releases it, representing the
        // relay being slow — target arrives before it returns.
        await new Promise((resolve) => {
          releaseFirstFetch = resolve;
        });
        // Return stale empty: this is what the relay delivers before the
        // target event has replicated. Without cancelQueries the hook would
        // settle success([]) here.
        return singlePage([]);
      }
      // Replacement fetch: relay has caught up, target is now available.
      return singlePage([targetEvent]);
    };

    queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const channel = { id: "chan-cancel", channelType: "group" };
    const wrapper = ({ children }) =>
      createElement(QueryClientProvider, { client: queryClient }, children);

    const hook = renderHook(
      ({ expectedEventId }) =>
        useThreadReplies(channel, "root-cancel", expectedEventId),
      { wrapper, initialProps: { expectedEventId: null } },
    );
    unmount = hook.unmount;
    const { rerender, result } = hook;

    // Let the first fetch start — it is now in-flight (fetchStatus=fetching,
    // status=pending). Yield the microtask queue without releasing the gate.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

    // Supply the target: the effect must detect the cold-fetch in-flight state
    // and call cancelQueries before invalidating.
    await act(async () => {
      rerender({ expectedEventId: "target-evt" });
    });

    // Release the stale first fetch AFTER the effect fires. With the fix the
    // cancelled retryer discards its result. Without the fix, [] settles first
    // and the hook either stays in error (the new closure sees [] and throws
    // ThreadExpectedEventMissingError with retry:false) or resolves [] —
    // neither is the target-bearing success state we require.
    await act(async () => {
      releaseFirstFetch?.();
      await new Promise((resolve) => setTimeout(resolve, 50));
    });

    // Hook must settle success with the target event.
    assert.ok(
      !result.current.isError,
      `hook must not be in error state (isError=${result.current.isError}, error=${result.current.error})`,
    );
    assert.ok(
      Array.isArray(result.current.data),
      `hook must expose reply data (data=${JSON.stringify(result.current.data)})`,
    );
    assert.ok(
      result.current.data?.some((e) => e.id === targetEvent.id),
      `data must include the target event (got ids: ${result.current.data?.map((e) => e.id).join(",")})`,
    );
  } finally {
    try {
      unmount?.();
    } catch (_) {}
    try {
      cleanup?.();
    } catch (_) {}
    try {
      queryClient?.clear();
    } catch (_) {}
  }
});

// ── Test: thread aux closure is not fetched (anti-regression) ─────────────────

test("thread replies trust the relay-provided aux closure", async () => {
  const { readFile } = await import("node:fs/promises");
  const source = await readFile(
    new URL("./useThreadReplies.ts", import.meta.url),
    "utf8",
  );
  assert.doesNotMatch(
    source,
    /withThreadAux|fetchStructuralAuxForMessages|fetchAuxEventsByReference/,
  );
  assert.match(source, /replies\.push\(\.\.\.response\.events\)/);
});
