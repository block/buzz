import assert from "node:assert/strict";
import test from "node:test";

import {
  recallChannelThread,
  rememberChannelThread,
  resetChannelPanelMemory,
} from "./channelPanelMemory.ts";

const SESSION_KEY = "buzz.channels.thread-panel-memory";

/** Minimal sessionStorage stub installed on globalThis.window. */
function installSessionStorage(initial = {}) {
  const store = new Map(Object.entries(initial));
  const stub = {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => {
      store.set(key, String(value));
    },
    removeItem: (key) => {
      store.delete(key);
    },
  };

  const previousWindow = globalThis.window;
  if (previousWindow === undefined) {
    globalThis.window = {};
  }
  const previousSessionStorage = globalThis.window.sessionStorage;
  globalThis.window.sessionStorage = stub;

  return {
    store,
    restore: () => {
      if (previousWindow === undefined) {
        delete globalThis.window;
      } else {
        globalThis.window.sessionStorage = previousSessionStorage;
      }
    },
  };
}

function withSessionStorage(initial, fn) {
  const { store, restore } = installSessionStorage(initial);
  try {
    fn(store);
  } finally {
    restore();
  }
}

// The module hydrates from sessionStorage on first access, so the hydration
// test must run before anything else touches the memory in this process.
test("hydrates existing session state on first access", () => {
  withSessionStorage(
    {
      [SESSION_KEY]: JSON.stringify({
        "channel-open": "thread-1",
        "channel-closed": null,
      }),
    },
    () => {
      assert.equal(recallChannelThread("channel-open"), "thread-1");
      assert.equal(recallChannelThread("channel-closed"), null);
      assert.equal(recallChannelThread("channel-unknown"), undefined);
    },
  );
});

test("remember/recall round-trip is tri-state", () => {
  withSessionStorage({}, () => {
    resetChannelPanelMemory();

    assert.equal(recallChannelThread("channel-a"), undefined);

    rememberChannelThread("channel-a", "thread-a");
    assert.equal(recallChannelThread("channel-a"), "thread-a");

    // Explicitly closed is remembered as null, distinct from "no memory".
    rememberChannelThread("channel-a", null);
    assert.equal(recallChannelThread("channel-a"), null);
    assert.equal(recallChannelThread("channel-b"), undefined);
  });
});

test("channels remember independently", () => {
  withSessionStorage({}, () => {
    resetChannelPanelMemory();

    rememberChannelThread("channel-a", "thread-a");
    rememberChannelThread("channel-b", "thread-b");
    rememberChannelThread("channel-c", null);

    assert.equal(recallChannelThread("channel-a"), "thread-a");
    assert.equal(recallChannelThread("channel-b"), "thread-b");
    assert.equal(recallChannelThread("channel-c"), null);
  });
});

test("writes through to sessionStorage", () => {
  withSessionStorage({}, (store) => {
    resetChannelPanelMemory();

    rememberChannelThread("channel-a", "thread-a");
    assert.deepEqual(JSON.parse(store.get(SESSION_KEY)), {
      "channel-a": "thread-a",
    });

    rememberChannelThread("channel-a", null);
    assert.deepEqual(JSON.parse(store.get(SESSION_KEY)), {
      "channel-a": null,
    });
  });
});

test("redundant writes are skipped", () => {
  withSessionStorage({}, (store) => {
    resetChannelPanelMemory();

    rememberChannelThread("channel-a", "thread-a");
    store.delete(SESSION_KEY);

    // Same value again: no new storage write.
    rememberChannelThread("channel-a", "thread-a");
    assert.equal(store.has(SESSION_KEY), false);

    // Changed value: writes.
    rememberChannelThread("channel-a", "thread-b");
    assert.equal(store.has(SESSION_KEY), true);
  });
});

test("reset forgets memory and clears storage", () => {
  withSessionStorage({}, (store) => {
    resetChannelPanelMemory();

    rememberChannelThread("channel-a", "thread-a");
    resetChannelPanelMemory();

    assert.equal(recallChannelThread("channel-a"), undefined);
    assert.equal(store.has(SESSION_KEY), false);
  });
});

test("storage failures leave the in-memory map working", () => {
  const throwingStub = {
    getItem: () => {
      throw new Error("storage unavailable");
    },
    setItem: () => {
      throw new Error("storage unavailable");
    },
    removeItem: () => {
      throw new Error("storage unavailable");
    },
  };

  const previousWindow = globalThis.window;
  if (previousWindow === undefined) {
    globalThis.window = {};
  }
  const previousSessionStorage = globalThis.window?.sessionStorage;
  globalThis.window.sessionStorage = throwingStub;

  try {
    resetChannelPanelMemory();
    rememberChannelThread("channel-a", "thread-a");
    assert.equal(recallChannelThread("channel-a"), "thread-a");
    resetChannelPanelMemory();
    assert.equal(recallChannelThread("channel-a"), undefined);
  } finally {
    if (previousWindow === undefined) {
      delete globalThis.window;
    } else {
      globalThis.window.sessionStorage = previousSessionStorage;
    }
  }
});
