import assert from "node:assert/strict";
import test from "node:test";

const KEY = "buzz.window.nativeDecorations";
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
      `./windowDecorationsPreference.ts?test=${importSequence++}`
    );
    await run(module);
  } finally {
    if (descriptor)
      Object.defineProperty(globalThis, "localStorage", descriptor);
    else delete globalThis.localStorage;
  }
}

test("missing, malformed, and unreadable preferences default to visible", async () => {
  for (const stored of [null, "hidden", "{bad-json"]) {
    await withStorage(
      { getItem: (key) => (key === KEY ? stored : null), setItem() {} },
      ({ getWindowDecorationsVisible }) => {
        assert.equal(getWindowDecorationsVisible(), true);
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
    ({ getWindowDecorationsVisible }) => {
      assert.equal(getWindowDecorationsVisible(), true);
    },
  );
});

test("loads and writes the native decoration preference", async () => {
  const writes = [];
  await withStorage(
    {
      getItem: (key) => (key === KEY ? "false" : null),
      setItem: (key, value) => writes.push([key, value]),
    },
    ({ getWindowDecorationsVisible, setWindowDecorationsVisible }) => {
      assert.equal(getWindowDecorationsVisible(), false);
      setWindowDecorationsVisible(true);
      assert.equal(getWindowDecorationsVisible(), true);
      assert.deepEqual(writes, [[KEY, "true"]]);
    },
  );
});

test("keeps the in-memory choice when persistence fails", async () => {
  await withStorage(
    {
      getItem: () => "true",
      setItem() {
        throw new Error("quota exceeded");
      },
    },
    ({ getWindowDecorationsVisible, setWindowDecorationsVisible }) => {
      assert.doesNotThrow(() => setWindowDecorationsVisible(false));
      assert.equal(getWindowDecorationsVisible(), false);
    },
  );
});
