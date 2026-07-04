/**
 * Unit tests for RelayReconnectController.
 *
 * Tests the production controller class with injected dependencies — no
 * React, no Tauri, no DOM required. Covers the three-phase strategy,
 * cancellation token correctness, single-flight guard, and state transitions.
 */

import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { RelayReconnectController } from "./relayReconnectController.ts";

// ── Dep builder helpers ───────────────────────────────────────────────────────

/** Build a minimal deps object with controllable preconnect behaviour. */
function makeDeps({
  preconnectResult = async () => {},
  hookConfiguredResult = async () => false,
  runHookResult = async () => {},
  connectionStateListener = null,
} = {}) {
  const timers = [];
  const intervals = [];
  let timerSeq = 0;
  let intervalSeq = 0;

  const deps = {
    preconnect: mock.fn(preconnectResult),
    hookConfigured: mock.fn(hookConfiguredResult),
    runHook: mock.fn(runHookResult),
    subscribeToConnectionState: mock.fn((listener) => {
      if (connectionStateListener) connectionStateListener.set(listener);
      return () => {};
    }),
    onSuccess: mock.fn(),
    onBackstop: mock.fn(),
    setTimeout: mock.fn((fn, ms) => {
      const id = ++timerSeq;
      timers.push({ id, fn, ms });
      return id;
    }),
    clearTimeout: mock.fn((id) => {
      const idx = timers.findIndex((t) => t.id === id);
      if (idx !== -1) timers.splice(idx, 1);
    }),
    setInterval: mock.fn((fn, ms) => {
      const id = ++intervalSeq;
      intervals.push({ id, fn, ms });
      return id;
    }),
    clearInterval: mock.fn((id) => {
      const idx = intervals.findIndex((t) => t.id === id);
      if (idx !== -1) intervals.splice(idx, 1);
    }),
    // Test helpers to fire timers/intervals.
    _fireTimer: (id) => timers.find((t) => t.id === id)?.fn(),
    _fireInterval: (id) => intervals.find((t) => t.id === id)?.fn(),
    _timers: timers,
    _intervals: intervals,
  };
  return deps;
}

// ── Phase 1: fast path ────────────────────────────────────────────────────────

test("fast-path success — hook never invoked, onSuccess fires, state resets", async () => {
  const ctrl = new RelayReconnectController();
  const deps = makeDeps({ preconnectResult: async () => {} });

  const result = await ctrl.start(deps);

  assert.equal(result, true, "start() returns true on fast-path success");
  assert.equal(deps.hookConfigured.mock.calls.length, 0, "hook not queried");
  assert.equal(deps.runHook.mock.calls.length, 0, "hook not run");
  assert.equal(deps.onSuccess.mock.calls.length, 1, "onSuccess fired");
  assert.deepEqual(ctrl.getState(), {
    isPending: false,
    isWaitingOnReconnectHook: false,
  });
});

test("fast-path success — no poll interval or backstop scheduled", async () => {
  const ctrl = new RelayReconnectController();
  const deps = makeDeps({ preconnectResult: async () => {} });

  await ctrl.start(deps);

  assert.equal(deps.setInterval.mock.calls.length, 0, "no poll interval");
  // The fast-path withDeadline schedules one timeout for the deadline, but
  // that is cleared by finally(). After success no backstop should remain.
  assert.equal(
    deps._timers.length,
    0,
    "no outstanding timers after fast-path win",
  );
});

// ── Phase 2: escalation ───────────────────────────────────────────────────────

test("escalation fires only when fast path fails and hook is configured", async () => {
  const ctrl = new RelayReconnectController();
  let callIdx = 0;
  const deps = makeDeps({
    preconnectResult: async () => {
      // First call (fast path) fails; subsequent calls (poll) succeed.
      if (++callIdx === 1) throw new Error("relay unreachable");
    },
    hookConfiguredResult: async () => true,
  });

  // Don't await — it enters phase 3 and returns false.
  const promise = ctrl.start(deps);

  // Let the async phases run.
  await promise;

  assert.equal(
    deps.hookConfigured.mock.calls.length,
    1,
    "hook-configured checked once",
  );
  assert.equal(deps.runHook.mock.calls.length, 1, "hook run once");
});

test("escalation skipped when hook not configured", async () => {
  const ctrl = new RelayReconnectController();
  const deps = makeDeps({
    preconnectResult: async () => {
      throw new Error("fail");
    },
    hookConfiguredResult: async () => false,
  });

  await ctrl.start(deps);

  assert.equal(
    deps.runHook.mock.calls.length,
    0,
    "hook not run when not configured",
  );
});

test("hook failure is non-fatal — polling still starts", async () => {
  const ctrl = new RelayReconnectController();
  const deps = makeDeps({
    preconnectResult: async () => {
      throw new Error("fail");
    },
    hookConfiguredResult: async () => true,
    runHookResult: async () => {
      throw new Error("hook blew up");
    },
  });

  // Should not throw.
  let threw = false;
  try {
    await ctrl.start(deps);
  } catch {
    threw = true;
  }

  assert.equal(threw, false, "hook failure does not propagate");
  assert.ok(
    deps._intervals.length > 0 || deps._timers.length > 0,
    "phase 3 timers scheduled",
  );
});

// ── Phase 3: poll-until-connected ─────────────────────────────────────────────

test("poll preempts backstop — onSuccess fires when poll wins", async () => {
  const ctrl = new RelayReconnectController();
  let callIdx = 0;
  const deps = makeDeps({
    preconnectResult: async () => {
      if (++callIdx <= 2) throw new Error("not yet");
      // Third call succeeds — simulates poll winning.
    },
    hookConfiguredResult: async () => false,
  });

  // Phase 1 fails, enters phase 3. start() returns false.
  const promise = ctrl.start(deps);
  await promise;

  // Fire the poll interval twice to reach the succeeding call.
  const intervalId = deps._intervals[0]?.id;
  assert.ok(intervalId !== undefined, "poll interval is active");

  await deps._fireInterval(intervalId); // call 2 — fails
  await deps._fireInterval(intervalId); // call 3 — succeeds

  assert.equal(
    deps.onSuccess.mock.calls.length,
    1,
    "onSuccess fired after poll win",
  );
  assert.deepEqual(ctrl.getState(), {
    isPending: false,
    isWaitingOnReconnectHook: false,
  });
});

test("connection-state emitter fires onSuccess and cancels poll/backstop", async () => {
  let capturedListener = null;
  const ctrl = new RelayReconnectController();
  const deps = makeDeps({
    preconnectResult: async () => {
      throw new Error("fail");
    },
    hookConfiguredResult: async () => false,
    connectionStateListener: {
      set: (l) => {
        capturedListener = l;
      },
    },
  });

  await ctrl.start(deps);

  // Emitter fires "connected" — should resolve without waiting for poll.
  assert.ok(capturedListener !== null, "connection-state listener registered");
  capturedListener("connected");

  assert.equal(
    deps.onSuccess.mock.calls.length,
    1,
    "onSuccess fired via emitter",
  );
  assert.deepEqual(ctrl.getState(), {
    isPending: false,
    isWaitingOnReconnectHook: false,
  });
  assert.equal(deps._intervals.length, 0, "poll interval cancelled");
  assert.equal(deps._timers.length, 0, "backstop cancelled");
});

test("backstop fires onBackstop, not onSuccess, and resets state", async () => {
  const ctrl = new RelayReconnectController();
  const deps = makeDeps({
    preconnectResult: async () => {
      throw new Error("always fails");
    },
    hookConfiguredResult: async () => false,
  });

  await ctrl.start(deps);

  // Locate and fire the backstop timer (largest ms).
  const backstopTimer = deps._timers.reduce(
    (max, t) => (t.ms >= max.ms ? t : max),
    { ms: 0, id: -1, fn: null },
  );
  assert.ok(backstopTimer.id !== -1, "backstop timer exists");
  backstopTimer.fn();

  assert.equal(deps.onBackstop.mock.calls.length, 1, "onBackstop called");
  assert.equal(deps.onSuccess.mock.calls.length, 0, "onSuccess NOT called");
  assert.deepEqual(ctrl.getState(), {
    isPending: false,
    isWaitingOnReconnectHook: false,
  });
});

// ── Cancellation token ────────────────────────────────────────────────────────

test("cancellation token — superseded poll callback does not call onSuccess", async () => {
  const ctrl = new RelayReconnectController();
  let preconnectCallCount = 0;
  let resolveFirstPreconnect;
  const deps = makeDeps({
    preconnectResult: async () => {
      preconnectCallCount++;
      if (preconnectCallCount === 1) {
        // First fast-path call: stall so we can cancel mid-flight.
        await new Promise((resolve) => {
          resolveFirstPreconnect = resolve;
        });
        throw new Error("cancelled");
      }
    },
    hookConfiguredResult: async () => false,
  });

  // Start attempt #1 — stalls in fast path.
  const attempt1 = ctrl.start(deps);

  // Cancel the attempt before the stalled preconnect resolves.
  ctrl.cancel();
  // Now resolve the stalled promise — it should be ignored.
  resolveFirstPreconnect?.();
  await attempt1;

  // Start attempt #2 — clean state.
  const deps2 = makeDeps({ preconnectResult: async () => {} });
  const result2 = await ctrl.start(deps2);

  assert.equal(result2, true, "second attempt succeeds on fast path");
  assert.equal(
    deps2.onSuccess.mock.calls.length,
    1,
    "only second attempt's onSuccess fires",
  );
  assert.equal(
    deps.onSuccess.mock.calls.length,
    0,
    "first attempt's onSuccess never fires",
  );
});

test("cancel mid fast-path — preconnect resolves SUCCESSFULLY after cancel but onSuccess never fires", async () => {
  const ctrl = new RelayReconnectController();
  let resolvePreconnect;
  const deps = makeDeps({
    // preconnect stalls then SUCCEEDS — worst case: cancel races a fast-path win.
    preconnectResult: () =>
      new Promise((resolve) => {
        resolvePreconnect = resolve;
      }),
    hookConfiguredResult: async () => false,
  });

  const attempt = ctrl.start(deps);

  // Cancel while fast-path is in flight.
  ctrl.cancel();

  // Let preconnect resolve successfully after cancel.
  resolvePreconnect?.();
  await attempt;

  assert.equal(
    deps.onSuccess.mock.calls.length,
    0,
    "onSuccess never fires after cancel",
  );
  assert.deepEqual(
    ctrl.getState(),
    { isPending: false, isWaitingOnReconnectHook: false },
    "state is idle",
  );
  assert.equal(deps._timers.length, 0, "no timers outstanding after cancel");
  assert.equal(
    deps._intervals.length,
    0,
    "no intervals outstanding after cancel",
  );
});

test("last subscriber unsubscribe cancels in-flight attempt", async () => {
  const ctrl = new RelayReconnectController();
  let resolvePreconnect;
  const deps = makeDeps({
    preconnectResult: () =>
      new Promise((resolve) => {
        resolvePreconnect = resolve;
      }),
    hookConfiguredResult: async () => false,
  });

  // Two subscribers.
  const unsub1 = ctrl.subscribe(() => {});
  const unsub2 = ctrl.subscribe(() => {});

  const attempt = ctrl.start(deps);

  // First unsub — still one subscriber, should NOT cancel.
  unsub1();
  assert.equal(
    ctrl.getState().isPending,
    true,
    "still pending after first unsub",
  );

  // Last unsub — should cancel.
  unsub2();
  assert.equal(ctrl.getState().isPending, false, "cancelled after last unsub");

  // Let preconnect resolve successfully — should be ignored.
  resolvePreconnect?.();
  await attempt;

  assert.equal(
    deps.onSuccess.mock.calls.length,
    0,
    "onSuccess never fires after last-subscriber cancel",
  );
});

// ── Single-flight guard ───────────────────────────────────────────────────────

test("second start() while first is pending returns false immediately", async () => {
  const ctrl = new RelayReconnectController();
  let resolvePreconnect;
  const deps = makeDeps({
    preconnectResult: () =>
      new Promise((resolve) => {
        resolvePreconnect = resolve;
      }),
  });
  const deps2 = makeDeps({ preconnectResult: async () => {} });

  // Start attempt #1 — stalls in fast path.
  const attempt1 = ctrl.start(deps);

  // Second start should be rejected immediately.
  const result2 = await ctrl.start(deps2);
  assert.equal(
    result2,
    false,
    "second start returns false while first pending",
  );
  assert.equal(
    deps2.preconnect.mock.calls.length,
    0,
    "second attempt never calls preconnect",
  );

  // Finish the first attempt.
  resolvePreconnect?.();
  await attempt1;
});

// ── State listener ────────────────────────────────────────────────────────────

test("subscribers receive state transitions and can unsubscribe", async () => {
  const ctrl = new RelayReconnectController();
  const states = [];
  const unsub = ctrl.subscribe((s) => states.push({ ...s }));

  const deps = makeDeps({ preconnectResult: async () => {} });
  await ctrl.start(deps);

  unsub();

  // Should have seen: initial idle, pending=true, pending=false (success).
  assert.ok(states.length >= 2, "at least 2 state transitions observed");
  assert.equal(
    states[states.length - 1].isPending,
    false,
    "last state is not pending",
  );

  // After unsubscribe, no further updates.
  const countBefore = states.length;
  const deps2 = makeDeps({ preconnectResult: async () => {} });
  await ctrl.start(deps2);
  assert.equal(states.length, countBefore, "no updates after unsubscribe");
});

// ── OSS build boundary ────────────────────────────────────────────────────────

test("OSS build (hookConfigured returns false) — runHook never called", async () => {
  const ctrl = new RelayReconnectController();
  const deps = makeDeps({
    preconnectResult: async () => {
      throw new Error("relay unreachable");
    },
    hookConfiguredResult: async () => false,
  });

  await ctrl.start(deps);

  assert.equal(
    deps.runHook.mock.calls.length,
    0,
    "runHook not called in OSS build",
  );
});

// ── Synchronous connected emission ───────────────────────────────────────────

test("sync connected emission — onSuccess fires once, no interval/backstop installed, subscription cleaned up", async () => {
  const ctrl = new RelayReconnectController();

  // subscribeToConnectionState fake that invokes the listener synchronously
  // with "connected" BEFORE returning the cleanup handle. This models the
  // production subscribeToConnectionState documented behaviour: it fires the
  // listener with the current state before returning.
  let cleanupCalled = false;
  const deps = makeDeps({
    preconnectResult: async () => {
      throw new Error("relay unreachable");
    },
    hookConfiguredResult: async () => false,
  });

  // Override subscribeToConnectionState with the synchronous-emission fake.
  deps.subscribeToConnectionState = mock.fn((listener) => {
    // Invoke immediately — simulates "already connected" at subscribe time.
    listener("connected");
    // Return cleanup handle (production unsubscribe fn).
    const cleanup = () => {
      cleanupCalled = true;
    };
    return cleanup;
  });

  await ctrl.start(deps);

  assert.equal(
    deps.onSuccess.mock.calls.length,
    1,
    "onSuccess fires exactly once",
  );
  assert.equal(deps._intervals.length, 0, "no poll interval installed");
  assert.equal(deps._timers.length, 0, "no backstop timer installed");
  assert.equal(cleanupCalled, true, "subscription cleanup handle was called");
  assert.deepEqual(
    ctrl.getState(),
    { isPending: false, isWaitingOnReconnectHook: false },
    "state is idle after sync success",
  );
});
