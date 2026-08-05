import assert from "node:assert/strict";
import test from "node:test";

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
      `./scalePreference.ts?test=${importSequence++}`
    );
    await run(module);
  } finally {
    if (descriptor)
      Object.defineProperty(globalThis, "localStorage", descriptor);
    else delete globalThis.localStorage;
  }
}

test("createScalePreference stores, snaps, and resets", async () => {
  const writes = [];
  const removals = [];
  await withStorage(
    {
      getItem: () => null,
      setItem: (key, value) => writes.push([key, value]),
      removeItem: (key) => removals.push(key),
    },
    ({ createScalePreference }) => {
      const pref = createScalePreference({
        storageKey: "buzz:test-scale",
        cssVar: "--buzz-test-scale",
      });

      assert.equal(pref.get(), 1);
      pref.set(1.25);
      assert.equal(pref.get(), 1.25);
      assert.deepEqual(writes, [["buzz:test-scale", "1.25"]]);
      assert.equal(pref.formatPercent(1.25), "125%");
      assert.equal(pref.presetIndex(1.25), pref.PRESETS.indexOf(1.25));

      pref.set(1);
      assert.equal(pref.get(), 1);
      assert.deepEqual(removals, ["buzz:test-scale"]);
    },
  );
});

test("clearCssVarAtDefault false keeps the CSS variable at default", async () => {
  const sets = [];
  const removals = [];
  const previousDocument = globalThis.document;
  globalThis.document = {
    documentElement: {
      style: {
        setProperty(name, value) {
          sets.push([name, value]);
        },
        removeProperty(name) {
          removals.push(name);
        },
      },
    },
  };
  try {
    await withStorage(
      {
        getItem: () => null,
        setItem() {},
        removeItem() {},
      },
      ({ createScalePreference }) => {
        const pref = createScalePreference({
          storageKey: "buzz:test-scale-keep",
          cssVar: "--buzz-test-keep",
          clearCssVarAtDefault: false,
        });
        // Applied once on create at default.
        assert.deepEqual(sets.at(-1), ["--buzz-test-keep", "1"]);
        pref.set(1.25);
        assert.deepEqual(sets.at(-1), ["--buzz-test-keep", "1.25"]);
        pref.set(1);
        assert.deepEqual(sets.at(-1), ["--buzz-test-keep", "1"]);
        assert.equal(removals.length, 0);
      },
    );
  } finally {
    if (previousDocument === undefined) {
      delete globalThis.document;
    } else {
      globalThis.document = previousDocument;
    }
  }
});

test("loads clamped stored values", async () => {
  await withStorage(
    {
      getItem: (key) => (key === "buzz:test-scale" ? "9" : null),
      setItem() {},
      removeItem() {},
    },
    ({ createScalePreference }) => {
      const pref = createScalePreference({ storageKey: "buzz:test-scale" });
      assert.equal(pref.get(), pref.MAX);
    },
  );
});
