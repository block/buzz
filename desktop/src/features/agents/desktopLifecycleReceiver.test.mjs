import assert from "node:assert/strict";
import test from "node:test";
import { receiveLifecycle } from "./desktopLifecycle.ts";
import {
  ownLifecycleReceiver,
  RECEIVER_RECOVERY_DELAYS_MS,
} from "./desktopLifecycleReceiver.ts";

const scope = { owner: "owner", community: "wss://one.example" };

async function flushUntil(predicate, attempts = 40) {
  for (let attempt = 0; attempt < attempts; attempt++) {
    if (predicate()) return;
    await Promise.resolve();
  }
  assert.fail("condition did not become true before the microtask limit");
}

function timers() {
  let nextId = 1;
  const pending = new Map();
  return {
    setTimer(callback, delayMs) {
      const id = nextId++;
      pending.set(id, { callback, delayMs });
      return id;
    },
    clearTimer(id) {
      pending.delete(id);
    },
    fireNext() {
      const entry = pending.entries().next().value;
      assert.ok(entry, "expected a pending recovery timer");
      pending.delete(entry[0]);
      entry[1].callback();
    },
    pending,
  };
}

test("first subscribe failure recovers without a reconnect callback and syncs before admission", async () => {
  const clock = timers();
  let subscribeCalls = 0;
  let liveEvent;
  let activeSubscriptions = 0;
  let closed = 0;
  let ready = 0;
  const calls = [];
  const relay = {
    getSessionEpoch: () => 1,
    getConnectionGeneration: () => 1,
    subscribeLive: async (filter, onEvent, _onReady, _timeout, options) => {
      assert.deepEqual(filter, {
        kinds: [50182, 50180],
        authors: [scope.owner],
        limit: 0,
      });
      subscribeCalls++;
      if (subscribeCalls === 1)
        throw new Error("fixture first connection rejected");
      activeSubscriptions++;
      liveEvent = onEvent;
      onEvent({ id: "during-sync", kind: 50182, created_at: 1 });
      options.onState("eose");
      return () => {
        activeSubscriptions--;
        closed++;
      };
    },
    fetchEvents: async () => {
      calls.push("history");
      return [];
    },
    publishEvent: async () => {},
  };
  const ipc = async (command, args) => {
    if (command === "observe_desktop_placement") {
      calls.push(args.reconcile ? `projection:${args.events.length}` : "page");
      return;
    }
    if (command === "receive_desktop_lifecycle") {
      calls.push("admission");
      return null;
    }
    throw new Error(command);
  };
  const errors = [];
  const stop = ownLifecycleReceiver(
    scope,
    (error) => errors.push(error),
    () => ready++,
    {
      ...clock,
      waitForRateLimit: async () => {},
      startReceiver: (receiverScope, active, onError, onReady, onClosed) =>
        receiveLifecycle(
          receiverScope,
          active,
          onError,
          ipc,
          relay,
          onReady,
          onClosed,
        ),
    },
  );

  await flushUntil(() => clock.pending.size === 1);
  assert.equal(subscribeCalls, 1);
  assert.deepEqual(errors, []);
  assert.equal(clock.pending.values().next().value.delayMs, 1_000);

  clock.fireNext();
  await flushUntil(() => calls.includes("admission"));
  assert.equal(subscribeCalls, 2);
  assert.equal(activeSubscriptions, 1);
  assert.equal(ready, 1);
  assert.deepEqual(calls, [
    "history",
    "page",
    "projection:0",
    "projection:1",
    "admission",
  ]);

  liveEvent({ id: "ordinary-live", kind: 50182, created_at: 2 });
  await flushUntil(
    () => calls.filter((call) => call === "admission").length === 2,
  );
  stop();
  await flushUntil(() => activeSubscriptions === 0);
  assert.equal(activeSubscriptions, 0);
  assert.equal(closed, 1);
});

test("transient CLOSED before readiness retires and replaces the whole receiver", async () => {
  const clock = timers();
  let subscribeCalls = 0;
  let closeCount = 0;
  let ready = 0;
  const relay = {
    getSessionEpoch: () => 1,
    getConnectionGeneration: () => 1,
    subscribeLive: async (_filter, _onEvent, _onReady, _timeout, options) => {
      subscribeCalls++;
      if (subscribeCalls === 1)
        options.onState("closed", {
          classification: "retryable",
          retryAfterMs: 0,
        });
      else options.onState("eose");
      return () => closeCount++;
    },
    fetchEvents: async () => [],
    publishEvent: async () => {},
  };
  const stop = ownLifecycleReceiver(
    scope,
    () => {},
    () => ready++,
    {
      ...clock,
      waitForRateLimit: async () => {},
      startReceiver: (receiverScope, active, onError, onReady, onClosed) =>
        receiveLifecycle(
          receiverScope,
          active,
          onError,
          async () => {},
          relay,
          onReady,
          onClosed,
        ),
    },
  );

  await flushUntil(() => clock.pending.size === 1);
  await flushUntil(() => closeCount === 1);
  assert.equal(subscribeCalls, 1);
  assert.equal(closeCount, 1);
  clock.fireNext();
  await flushUntil(() => ready === 1);
  assert.equal(subscribeCalls, 2);
  stop();
  await flushUntil(() => closeCount === 2);
});

test("scope cancellation clears recovery timers and closes a late subscription", async () => {
  const timerClock = timers();
  let starts = 0;
  const stopTimerOwner = ownLifecycleReceiver(
    scope,
    () => {},
    () => {},
    {
      ...timerClock,
      waitForRateLimit: async () => {},
      startReceiver: async () => {
        starts++;
        throw new Error("transient");
      },
    },
  );
  await flushUntil(() => timerClock.pending.size === 1);
  stopTimerOwner();
  assert.equal(timerClock.pending.size, 0);
  assert.equal(starts, 1);

  let finishSubscribe;
  let lateCloseCount = 0;
  let historyCalls = 0;
  const relay = {
    getSessionEpoch: () => 1,
    getConnectionGeneration: () => 1,
    subscribeLive: () =>
      new Promise((resolve) => {
        finishSubscribe = () => resolve(() => lateCloseCount++);
      }),
    fetchEvents: async () => {
      historyCalls++;
      return [];
    },
    publishEvent: async () => {},
  };
  const stopLateOwner = ownLifecycleReceiver(
    scope,
    () => {},
    () => {},
    {
      ...timers(),
      waitForRateLimit: async () => {},
      startReceiver: (receiverScope, active, onError, onReady, onClosed) =>
        receiveLifecycle(
          receiverScope,
          active,
          onError,
          async () => {},
          relay,
          onReady,
          onClosed,
        ),
    },
  );
  await flushUntil(() => typeof finishSubscribe === "function");
  stopLateOwner();
  finishSubscribe();
  await flushUntil(() => lateCloseCount === 1);
  assert.equal(historyCalls, 0, "cancelled receiver must not begin sync");
});

test("scope cancellation during sync fences reconciliation, admission, readiness, and errors", async () => {
  let resolveHistory;
  let deliver;
  let closeCount = 0;
  let ready = 0;
  const errors = [];
  const calls = [];
  const relay = {
    getSessionEpoch: () => 1,
    getConnectionGeneration: () => 1,
    subscribeLive: async (_filter, onEvent, _onReady, _timeout, options) => {
      deliver = onEvent;
      options.onState("eose");
      return () => closeCount++;
    },
    fetchEvents: () =>
      new Promise((resolve) => {
        resolveHistory = resolve;
      }),
    publishEvent: async () => {},
  };
  const stop = ownLifecycleReceiver(
    scope,
    (error) => errors.push(error),
    () => ready++,
    {
      ...timers(),
      waitForRateLimit: async () => {},
      startReceiver: (receiverScope, active, onError, onReady, onClosed) =>
        receiveLifecycle(
          receiverScope,
          active,
          onError,
          async (command) => {
            calls.push(command);
            return null;
          },
          relay,
          onReady,
          onClosed,
        ),
    },
  );
  await flushUntil(() => resolveHistory !== undefined);
  deliver({ id: "queued", kind: 50182, created_at: 1 });
  stop();
  resolveHistory([]);
  await flushUntil(() => closeCount === 1);

  assert.deepEqual(calls, []);
  assert.deepEqual(errors, []);
  assert.equal(ready, 0);
});

test("initializer recovery exhausts the bounded budget and preserves its safe terminal outcome", async () => {
  const clock = timers();
  let starts = 0;
  const errors = [];
  ownLifecycleReceiver(
    scope,
    (error) => errors.push(error),
    () => {},
    {
      ...clock,
      waitForRateLimit: async () => {},
      startReceiver: async () => {
        starts++;
        throw new Error("raw private initializer detail");
      },
    },
  );

  for (
    let attempt = 0;
    attempt < RECEIVER_RECOVERY_DELAYS_MS.length;
    attempt++
  ) {
    await flushUntil(() => clock.pending.size === 1);
    assert.equal(
      clock.pending.values().next().value.delayMs,
      RECEIVER_RECOVERY_DELAYS_MS[attempt],
    );
    clock.fireNext();
  }
  await flushUntil(() => errors.length === 1);
  assert.equal(starts, 4);
  assert.equal(clock.pending.size, 0);
  assert.equal(
    errors[0],
    "Desktop lifecycle receiver is unavailable (initialization failed).",
  );
  assert.doesNotMatch(errors[0], /private|detail/);
});

test("transient CLOSED recovery is bounded across successful receivers; terminal stays manual", async () => {
  const clock = timers();
  const closures = [];
  let starts = 0;
  let closes = 0;
  const errors = [];
  const stop = ownLifecycleReceiver(
    scope,
    (error) => errors.push(error),
    () => {},
    {
      ...clock,
      waitForRateLimit: async () => {},
      startReceiver: async (_scope, _active, _onError, onReady, onClosed) => {
        starts++;
        onReady();
        closures.push(onClosed);
        return () => closes++;
      },
    },
  );
  await flushUntil(() => closures.length === 1);

  for (
    let attempt = 0;
    attempt < RECEIVER_RECOVERY_DELAYS_MS.length;
    attempt++
  ) {
    closures.at(-1)({ classification: "retryable", retryAfterMs: 0 });
    assert.equal(
      clock.pending.values().next().value.delayMs,
      RECEIVER_RECOVERY_DELAYS_MS[attempt],
    );
    clock.fireNext();
    await flushUntil(() => closures.length === attempt + 2);
  }
  closures.at(-1)({ classification: "retryable", retryAfterMs: 0 });
  assert.equal(clock.pending.size, 0);
  assert.equal(starts, 4);
  assert.equal(closes, 4);
  assert.equal(errors.length, 1);
  assert.match(errors[0], /Retry the receiver/);
  stop();

  const terminalClock = timers();
  let terminalClosed;
  const terminalErrors = [];
  ownLifecycleReceiver(
    scope,
    (error) => terminalErrors.push(error),
    () => {},
    {
      ...terminalClock,
      startReceiver: async (_scope, _active, _onError, _onReady, onClosed) => {
        terminalClosed = onClosed;
        return () => {};
      },
    },
  );
  await flushUntil(() => terminalClosed !== undefined);
  terminalClosed({ classification: "terminal", retryAfterMs: 0 });
  assert.equal(terminalClock.pending.size, 0);
  assert.equal(terminalErrors.length, 1);
});

test("rate-limited recovery honors both the delay and shared gate, and cancellation fences it", async () => {
  const clock = timers();
  let releaseGate;
  const gate = new Promise((resolve) => {
    releaseGate = resolve;
  });
  const closures = [];
  let starts = 0;
  const stop = ownLifecycleReceiver(
    scope,
    () => {},
    () => {},
    {
      ...clock,
      waitForRateLimit: () => gate,
      startReceiver: async (_scope, _active, _onError, _onReady, onClosed) => {
        starts++;
        closures.push(onClosed);
        return () => {};
      },
    },
  );
  await flushUntil(() => closures.length === 1);
  closures[0]({ classification: "rate-limited", retryAfterMs: 8_000 });
  assert.equal(clock.pending.values().next().value.delayMs, 8_000);
  clock.fireNext();
  await Promise.resolve();
  assert.equal(starts, 1, "fresh subscription must wait for the active gate");
  stop();
  releaseGate();
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(starts, 1, "scope cancellation must fence the late gate result");
});
