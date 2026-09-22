import assert from "node:assert/strict";
import test from "node:test";

const KEY = "buzz.channels.threadViewMode";
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
      `./threadViewModePreference.ts?test=${importSequence++}`
    );
    await run(module);
  } finally {
    if (descriptor)
      Object.defineProperty(globalThis, "localStorage", descriptor);
    else delete globalThis.localStorage;
  }
}

test("missing, malformed, and unreadable preferences default to split", async () => {
  for (const stored of [null, "side-by-side", "{bad-json", "focus"]) {
    await withStorage(
      {
        getItem: (key) => (key === KEY ? stored : null),
        setItem() {},
      },
      ({ getThreadViewMode }) => {
        assert.equal(getThreadViewMode(), "split");
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
    ({ getThreadViewMode }) => {
      assert.equal(getThreadViewMode(), "split");
    },
  );
});

test("HulaBuzz ignores focus writes and stays on split", async () => {
  const writes = [];
  await withStorage(
    {
      getItem: (key) => (key === KEY ? "focus" : null),
      setItem: (key, value) => writes.push([key, value]),
    },
    ({ getThreadViewMode, setThreadViewMode }) => {
      assert.equal(getThreadViewMode(), "split");
      setThreadViewMode("focus");
      assert.equal(getThreadViewMode(), "split");
      // Stale focus is rewritten to split on module load and/or set.
      assert.ok(
        writes.some(([key, value]) => key === KEY && value === "split"),
        `expected a split rewrite, got ${JSON.stringify(writes)}`,
      );
    },
  );
});

test("keeps split when persistence fails", async () => {
  await withStorage(
    {
      getItem: () => null,
      setItem() {
        throw new Error("quota exceeded");
      },
    },
    ({ getThreadViewMode, setThreadViewMode }) => {
      assert.doesNotThrow(() => setThreadViewMode("split"));
      assert.equal(getThreadViewMode(), "split");
    },
  );
});
