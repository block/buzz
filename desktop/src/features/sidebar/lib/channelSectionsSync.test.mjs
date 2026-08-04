import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import {
  ChannelSectionSyncManager,
  channelSectionStoresEqual,
  serializeChannelSectionsPayload,
} from "./channelSectionsSync.ts";

function makeStore(overrides = {}) {
  return {
    version: 1,
    sections: overrides.sections ?? [],
    assignments: overrides.assignments ?? {},
    ...overrides,
  };
}

// ─── destroy() must cancel pending publish, not flush ─────────────────────────

// Regression guard for the community-switch cross-relay publish vector:
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

    // Destroy before the debounce fires — simulates community switch.
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

// Regression guard for the timer-fired race: debounce fires → doPublish starts
// awaiting refreshRemoteTimestampBeforePublish → destroy() is called (relayUrl dep
// change) → publishEvent must never be called even though the timer already
// fired and cleared itself before destroy() ran.
test("destroy: aborts in-flight publish after remote timestamp refresh", async () => {
  // fetchEvents is held until we release it — simulates the latency window.
  let releaseFetch = null;
  const publishCalls = [];

  mock.method(relayClient, "fetchEvents", () => {
    return new Promise((resolve) => {
      // resolve with empty so the timestamp refresh completes
      releaseFetch = () => resolve([]);
    });
  });
  mock.method(relayClient, "publishEvent", (...args) => {
    publishCalls.push(args);
    return Promise.resolve();
  });

  if (typeof globalThis.window === "undefined") {
    globalThis.window = {};
  }
  let capturedCallback = null;
  let nextId = 1;
  const origSetTimeout = globalThis.window.setTimeout;
  const origClearTimeout = globalThis.window.clearTimeout;
  globalThis.window.setTimeout = (fn, _ms) => {
    capturedCallback = fn;
    return nextId++;
  };
  globalThis.window.clearTimeout = (_id) => {
    capturedCallback = null;
  };

  try {
    const manager = new ChannelSectionSyncManager("pk-race");
    const store = makeStore({
      sections: [{ id: "s1", name: "Work", order: 0 }],
    });

    // Queue the publish — captures the debounce callback.
    manager.publishSections(store);
    assert.ok(capturedCallback !== null, "debounce timer should be set");

    // Fire the debounce manually — this starts doPublish() and nulls
    // debounceTimer inside publishSections' callback, leaving the async
    // doPublish running and awaiting refreshRemoteTimestampBeforePublish.
    const timerFn = capturedCallback;
    capturedCallback = null; // timer cleared itself inside the callback
    timerFn();

    // Now destroy() — debounceTimer is already null (timer fired), so only
    // the destroyed flag can stop doPublish.
    manager.destroy();

    // Release the held fetchEvents, then doPublish should check destroyed and
    // abort before signing or publishing.
    releaseFetch();

    // Drain microtasks so doPublish fully runs through to its abort point.
    await new Promise((resolve) => setTimeout(resolve, 0));

    assert.equal(
      publishCalls.length,
      0,
      "publishEvent must not be called after destroy() even when timer already fired",
    );
  } finally {
    globalThis.window.setTimeout = origSetTimeout;
    globalThis.window.clearTimeout = origClearTimeout;
    mock.reset();
  }
});

test("destroy: is safe to call with no pending publish", () => {
  const manager = new ChannelSectionSyncManager("pk-no-pending");
  // Should not throw even with nothing queued.
  assert.doesNotThrow(() => manager.destroy());
});

test("remote section state applies when no explicit local edit is pending", () => {
  const manager = new ChannelSectionSyncManager("pk-remote-first");
  assert.equal(
    manager.shouldApplyRemote({
      store: makeStore({
        sections: [{ id: "remote", name: "Remote category", order: 0 }],
      }),
      createdAt: 10,
      eventId: "remote-first",
    }),
    true,
  );
});

test("pending local section edit wins over a remote snapshot during debounce", () => {
  let timerCallback = null;
  let nextId = 1;
  if (typeof globalThis.window === "undefined") {
    globalThis.window = {};
  }
  const origSetTimeout = globalThis.window.setTimeout;
  const origClearTimeout = globalThis.window.clearTimeout;
  globalThis.window.setTimeout = (fn, _ms) => {
    timerCallback = fn;
    return nextId++;
  };
  globalThis.window.clearTimeout = (_id) => {
    timerCallback = null;
  };

  try {
    const manager = new ChannelSectionSyncManager("pk-local-wins");
    const local = makeStore({
      sections: [{ id: "local", name: "New category", order: 0 }],
    });
    const staleRemote = {
      store: makeStore({ sections: [] }),
      createdAt: 100,
      eventId: "remote-before-local-publish",
    };

    manager.publishSections(local);

    assert.equal(
      manager.shouldApplyRemote(staleRemote),
      false,
      "remote snapshot must not replace an explicit local edit awaiting publish",
    );
    assert.deepEqual(
      manager.getPendingStore(),
      local,
      "the pending local category must remain queued",
    );
    assert.ok(
      timerCallback !== null,
      "the local publish timer must remain armed",
    );
  } finally {
    globalThis.window.setTimeout = origSetTimeout;
    globalThis.window.clearTimeout = origClearTimeout;
  }
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

// ─── channelsBlockIndex serialize + equality (pure helpers) ─────────────────

test("serializeChannelSectionsPayload: includes optional channelsBlockIndex", () => {
  const payload = serializeChannelSectionsPayload(
    makeStore({
      sections: [
        { id: "a", name: "A", order: 0 },
        { id: "b", name: "B", order: 1 },
      ],
      channelsBlockIndex: 1,
    }),
  );
  assert.equal(payload.version, 1);
  assert.equal(payload.channelsBlockIndex, 1);
  assert.equal(payload.sections.length, 2);
});

test("serializeChannelSectionsPayload: omits channelsBlockIndex when unset", () => {
  const payload = serializeChannelSectionsPayload(
    makeStore({ sections: [{ id: "a", name: "A", order: 0 }] }),
  );
  assert.equal(Object.hasOwn(payload, "channelsBlockIndex"), false);
});

test("channelSectionStoresEqual: index-only change is not equal", () => {
  const base = makeStore({
    sections: [
      { id: "a", name: "A", order: 0 },
      { id: "b", name: "B", order: 1 },
    ],
    channelsBlockIndex: 2,
  });
  const moved = { ...base, channelsBlockIndex: 0 };
  assert.equal(channelSectionStoresEqual(base, base), true);
  assert.equal(
    channelSectionStoresEqual(base, moved),
    false,
    "index-only block move must not compare equal",
  );
  assert.equal(channelSectionStoresEqual(moved, moved), true);
});

test("channelSectionStoresEqual: undefined index equals undefined, not 0", () => {
  const legacy = makeStore({
    sections: [{ id: "a", name: "A", order: 0 }],
  });
  const zero = makeStore({
    sections: [{ id: "a", name: "A", order: 0 }],
    channelsBlockIndex: 0,
  });
  assert.equal(channelSectionStoresEqual(legacy, legacy), true);
  assert.equal(channelSectionStoresEqual(legacy, zero), false);
});

test("shouldApplyRemote: cold-start remote index wins when no pending local edit", () => {
  const manager = new ChannelSectionSyncManager("pk-remote-index");
  const remote = {
    store: makeStore({
      sections: [
        { id: "a", name: "A", order: 0 },
        { id: "b", name: "B", order: 1 },
      ],
      channelsBlockIndex: 0,
    }),
    createdAt: 50,
    eventId: "remote-index",
  };
  assert.equal(manager.shouldApplyRemote(remote), true);
  assert.equal(remote.store.channelsBlockIndex, 0);
});
