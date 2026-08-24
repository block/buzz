import assert from "node:assert/strict";
import test from "node:test";

const KEY = "buzz.channels.threadTimelineMode";
let importSequence = 0;

async function withStorage(storage, run) {
  const descriptor = Object.getOwnPropertyDescriptor(
    globalThis,
    "localStorage",
  );
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: storage,
  });
  try {
    const module = await import(
      `./threadTimelineModePreference.ts?test=${importSequence++}`
    );
    await run(module);
  } finally {
    if (descriptor)
      Object.defineProperty(globalThis, "localStorage", descriptor);
    else delete globalThis.localStorage;
  }
}

test("missing, malformed, and unreadable timeline modes default to panel", async () => {
  for (const stored of [null, "drawer", "{bad-json"]) {
    await withStorage(
      { getItem: (key) => (key === KEY ? stored : null), setItem() {} },
      ({ getThreadTimelineMode }) => {
        assert.equal(getThreadTimelineMode(), "panel");
      },
    );
  }

  await withStorage(
    {
      getItem() {
        throw new Error("storage unavailable");
      },
      setItem() {},
    },
    ({ getThreadTimelineMode }) => {
      assert.equal(getThreadTimelineMode(), "panel");
    },
  );
});

test("loads and writes the stored inline timeline mode", async () => {
  const writes = [];
  await withStorage(
    {
      getItem: (key) => (key === KEY ? "inline" : null),
      setItem: (key, value) => writes.push([key, value]),
    },
    ({ getThreadTimelineMode, setThreadTimelineMode }) => {
      assert.equal(getThreadTimelineMode(), "inline");
      setThreadTimelineMode("panel");
      assert.equal(getThreadTimelineMode(), "panel");
      assert.deepEqual(writes, [[KEY, "panel"]]);
    },
  );
});

test("keeps the in-memory timeline mode when persistence fails", async () => {
  await withStorage(
    {
      getItem: () => null,
      setItem() {
        throw new Error("quota exceeded");
      },
    },
    ({ getThreadTimelineMode, setThreadTimelineMode }) => {
      assert.doesNotThrow(() => setThreadTimelineMode("inline"));
      assert.equal(getThreadTimelineMode(), "inline");
    },
  );
});
