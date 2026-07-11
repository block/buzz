import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { QueryClient, QueryObserver } from "@tanstack/react-query";

import {
  CHANNEL_INTENT_DWELL_MS,
  bindChannelIntentLifecycle,
  clearChannelIntentOnContextChange,
  createChannelIntentEventHandlers,
  createChannelIntentScheduler,
  resolveIntentChannel,
  shouldClearChannelIntent,
} from "./channelIntentPrefetch.ts";

const channel = (id, channelType = "stream") => ({ id, channelType });

function harness() {
  const timers = new Map();
  let nextId = 0;
  const calls = [];
  const scheduler = createChannelIntentScheduler(
    (value) => calls.push(value.id),
    {
      setTimeout(callback, delay) {
        const id = ++nextId;
        timers.set(id, { callback, delay });
        return id;
      },
      clearTimeout(id) {
        timers.delete(id);
      },
    },
  );
  return {
    calls,
    scheduler,
    pending: () => [...timers.values()],
    flush: () => {
      const pending = [...timers.values()];
      timers.clear();
      for (const timer of pending) timer.callback();
    },
  };
}

describe("channel intent scheduling", () => {
  it("does not dispatch before the dwell", () => {
    const h = harness();
    h.scheduler.schedule(channel("a"));
    assert.deepEqual(h.calls, []);
    assert.equal(h.pending()[0].delay, CHANNEL_INTENT_DWELL_MS);
  });

  it("dispatches once after the dwell", () => {
    const h = harness();
    h.scheduler.schedule(channel("a"));
    h.flush();
    assert.deepEqual(h.calls, ["a"]);
  });

  it("supersedes one unstarted candidate", () => {
    const h = harness();
    h.scheduler.schedule(channel("a"));
    h.scheduler.schedule(channel("b"));
    h.flush();
    assert.deepEqual(h.calls, ["b"]);
  });

  it("does not restart the same candidate", () => {
    const h = harness();
    h.scheduler.schedule(channel("a"));
    const timer = h.pending()[0];
    h.scheduler.schedule(channel("a"));
    assert.equal(h.pending()[0], timer);
  });

  it("pointer-down dispatches immediately and retires the timer", () => {
    const h = harness();
    h.scheduler.schedule(channel("a"));
    h.scheduler.dispatch(channel("a"));
    h.flush();
    assert.deepEqual(h.calls, ["a"]);
  });

  it("clears only the matching candidate", () => {
    const h = harness();
    h.scheduler.schedule(channel("a"));
    h.scheduler.clear("b");
    h.flush();
    assert.deepEqual(h.calls, ["a"]);
  });

  it("disposal clears a pending candidate", () => {
    const h = harness();
    h.scheduler.schedule(channel("a"));
    h.scheduler.dispose();
    h.flush();
    assert.deepEqual(h.calls, []);
  });

  it("channel-set replacement clears an outgoing workspace candidate", () => {
    const h = harness();
    h.scheduler.schedule(channel("old-workspace"));
    clearChannelIntentOnContextChange(h.scheduler);
    h.flush();
    assert.deepEqual(h.calls, []);
  });

  it("selection replacement clears a candidate that became active", () => {
    const h = harness();
    h.scheduler.schedule(channel("new-active"));
    clearChannelIntentOnContextChange(h.scheduler);
    h.flush();
    assert.deepEqual(h.calls, []);
  });
});

describe("channel intent row transitions", () => {
  it("keeps intent across pointer or focus transitions within one row", () => {
    assert.equal(shouldClearChannelIntent("a", "a"), false);
  });

  it("clears intent when pointer or focus leaves the row", () => {
    assert.equal(shouldClearChannelIntent("a", null), true);
    assert.equal(shouldClearChannelIntent("a", "b"), true);
  });
});

it("pointer-down prefetch and pending mount share one queryFn invocation", async () => {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  let resolve;
  const pending = new Promise((done) => {
    resolve = done;
  });
  let invocations = 0;
  const options = {
    queryKey: ["channel-messages", "a"],
    queryFn: async () => {
      invocations += 1;
      return pending;
    },
    staleTime: 5 * 60 * 1_000,
  };

  const pointerDownPrefetch = queryClient.prefetchQuery(options);
  const observer = new QueryObserver(queryClient, options);
  const unsubscribe = observer.subscribe(() => {});
  assert.equal(invocations, 1);

  resolve([]);
  await pointerDownPrefetch;
  assert.deepEqual(observer.getCurrentResult().data, []);
  assert.equal(invocations, 1);
  unsubscribe();
  queryClient.clear();
});

describe("delegated pointer and focus behavior", () => {
  function delegatedHarness() {
    const h = harness();
    const a = channel("a");
    const rows = new Map([
      ["a-child", { channelId: "a" }],
      ["a-action", { channelId: "a" }],
      ["outside", null],
    ]);
    const handlers = createChannelIntentEventHandlers(
      (id) => (id === "a" ? a : null),
      h.scheduler,
      (target) => rows.get(target) ?? null,
    );
    return { ...h, handlers };
  }

  it("keeps one dwell across delegated descendant pointer transitions", () => {
    const h = delegatedHarness();
    h.handlers.onPointerOver({ target: "a-child" });
    const timer = h.pending()[0];
    h.handlers.onPointerOut({ target: "a-child", relatedTarget: "a-action" });
    h.handlers.onPointerOver({ target: "a-action" });
    assert.equal(h.pending()[0], timer);
    h.flush();
    assert.deepEqual(h.calls, ["a"]);
  });

  it("gives delegated focus equivalent containment and exit behavior", () => {
    const h = delegatedHarness();
    h.handlers.onFocus({ target: "a-child" });
    h.handlers.onBlur({ target: "a-child", relatedTarget: "a-action" });
    assert.equal(h.pending().length, 1);
    h.handlers.onBlur({ target: "a-action", relatedTarget: "outside" });
    h.flush();
    assert.deepEqual(h.calls, []);
  });
});

describe("intent eligibility", () => {
  const stream = channel("stream");
  const forum = channel("forum", "forum");
  const channels = new Map([
    [stream.id, stream],
    [forum.id, forum],
  ]);

  it("excludes the active channel", () => {
    assert.equal(resolveIntentChannel(channels, "stream", "stream"), null);
  });

  it("excludes forums", () => {
    assert.equal(resolveIntentChannel(channels, null, "forum"), null);
  });
});

it("hook lifecycle cleanup retires its pending candidate", () => {
  const h = harness();
  h.scheduler.schedule(channel("a"));
  const unmount = bindChannelIntentLifecycle(h.scheduler);
  unmount();
  h.flush();
  assert.deepEqual(h.calls, []);
});

it("rejected best-effort prefetch does not reject its caller", async () => {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  await assert.doesNotReject(
    queryClient.prefetchQuery({
      queryKey: ["channel-messages", "rejected"],
      queryFn: async () => {
        throw new Error("relay unavailable");
      },
    }),
  );
  queryClient.clear();
});
