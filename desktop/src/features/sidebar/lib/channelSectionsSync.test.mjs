import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import { ChannelSectionSyncManager } from "./channelSectionsSync.ts";

function makeStore(overrides = {}) {
  return {
    version: 1,
    sections: overrides.sections ?? [],
    assignments: overrides.assignments ?? {},
    ...overrides,
  };
}

// ─── destroy() must cancel pending publish, not flush ─────────────────────────

// Regression guard for the workspace-switch cross-relay publish vector:
// edit sections in relay A → destroy() is called (relayUrl dep change) →
// no publish should fire. The scoped localStorage write is durable; when the
// user returns to relay A the seed-publish path handles it.
test("destroy: cancels pending publish without flushing to the relay", () => {
  const publishCalls = [];
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });

  // Simulate the timer scheduler with a manual clock so we can advance it.
  let timerCallback = null;
  const originalSetTimeout = globalThis.window?.setTimeout;
  const originalClearTimeout = globalThis.window?.clearTimeout;

  // Inject a fake window.setTimeout/clearTimeout if needed.
  const fakeTimers = [];
  let nextId = 1;
  if (typeof globalThis.window === "undefined") {
    globalThis.window = {};
  }
  globalThis.window.setTimeout = (fn, _ms) => {
    const id = nextId++;
    fakeTimers.push({ id, fn });
    timerCallback = fn;
    return id;
  };
  globalThis.window.clearTimeout = (id) => {
    const idx = fakeTimers.findIndex((t) => t.id === id);
    if (idx !== -1) {
      fakeTimers.splice(idx, 1);
      timerCallback = null;
    }
  };

  try {
    const manager = new ChannelSectionSyncManager("pk-test");
    const store = makeStore({
      sections: [{ id: "s1", name: "Work", order: 0 }],
    });

    // Queue a publish — this sets the debounce timer.
    manager.publishSections(store);
    assert.ok(timerCallback !== null, "debounce timer should be set");

    // Destroy before the debounce fires — simulates workspace switch.
    manager.destroy();

    // Timer must be cleared and no publish should fire now.
    assert.ok(
      timerCallback === null,
      "debounce timer should be cleared on destroy",
    );

    // Advance time by invoking the callback that was cleared — it shouldn't exist.
    // If clearTimeout didn't work, try firing whatever was captured before destroy.
    // (There's nothing to fire after a correct destroy.)
    assert.equal(
      publishCalls.length,
      0,
      "no publish event should have been sent after destroy",
    );
  } finally {
    // Restore timer functions.
    if (originalSetTimeout !== undefined) {
      globalThis.window.setTimeout = originalSetTimeout;
    }
    if (originalClearTimeout !== undefined) {
      globalThis.window.clearTimeout = originalClearTimeout;
    }
    mock.reset();
  }
});

test("destroy: is safe to call with no pending publish", () => {
  const manager = new ChannelSectionSyncManager("pk-no-pending");
  // Should not throw even with nothing queued.
  assert.doesNotThrow(() => manager.destroy());
});

test("destroy: cancelPendingPublish clears pendingStore", () => {
  let timerCallback = null;
  let nextId = 1;
  if (typeof globalThis.window === "undefined") {
    globalThis.window = {};
  }
  const orig = globalThis.window.setTimeout;
  const origClear = globalThis.window.clearTimeout;
  globalThis.window.setTimeout = (fn, _ms) => {
    timerCallback = fn;
    return nextId++;
  };
  globalThis.window.clearTimeout = (_id) => {
    timerCallback = null;
  };

  try {
    const manager = new ChannelSectionSyncManager("pk-pending-null");
    const store = makeStore({
      sections: [{ id: "s1", name: "Test", order: 0 }],
    });
    manager.publishSections(store);
    assert.deepEqual(manager.getPendingStore(), store);

    manager.destroy();
    assert.equal(
      manager.getPendingStore(),
      null,
      "pendingStore must be null after destroy",
    );
    assert.ok(timerCallback === null, "timer must be cleared after destroy");
  } finally {
    globalThis.window.setTimeout = orig;
    globalThis.window.clearTimeout = origClear;
  }
});
