import assert from "node:assert/strict";
import test from "node:test";

import { APPEARANCE_SCALE_PRESETS } from "./appearanceScalePresets.ts";

const KEY = "buzz:text-scale";
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
    const module = await import(`./textScale.ts?test=${importSequence++}`);
    await run(module);
  } finally {
    if (descriptor)
      Object.defineProperty(globalThis, "localStorage", descriptor);
    else delete globalThis.localStorage;
  }
}

test("missing, malformed, and unreadable preferences default to 1", async () => {
  for (const stored of [null, "nope", "NaN"]) {
    await withStorage(
      { getItem: (key) => (key === KEY ? stored : null), setItem() {} },
      ({ getTextScale, DEFAULT_TEXT_SCALE }) => {
        assert.equal(getTextScale(), DEFAULT_TEXT_SCALE);
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
    ({ getTextScale }) => {
      assert.equal(getTextScale(), 1);
    },
  );
});

test("loads a stored scale and clamps out-of-range values", async () => {
  await withStorage(
    {
      getItem: (key) => (key === KEY ? "1.25" : null),
      setItem() {},
      removeItem() {},
    },
    ({ getTextScale }) => {
      assert.equal(getTextScale(), 1.25);
    },
  );

  await withStorage(
    {
      getItem: (key) => (key === KEY ? "9" : null),
      setItem() {},
      removeItem() {},
    },
    ({ getTextScale, MAX_TEXT_SCALE }) => {
      assert.equal(getTextScale(), MAX_TEXT_SCALE);
    },
  );

  await withStorage(
    {
      getItem: (key) => (key === KEY ? "0.1" : null),
      setItem() {},
      removeItem() {},
    },
    ({ getTextScale, MIN_TEXT_SCALE }) => {
      assert.equal(getTextScale(), MIN_TEXT_SCALE);
    },
  );
});

test("writes non-default scale and removes storage at default", async () => {
  const writes = [];
  const removals = [];
  await withStorage(
    {
      getItem: () => null,
      setItem: (key, value) => writes.push([key, value]),
      removeItem: (key) => removals.push(key),
    },
    ({ setTextScale, getTextScale, DEFAULT_TEXT_SCALE }) => {
      setTextScale(1.25);
      assert.equal(getTextScale(), 1.25);
      assert.deepEqual(writes, [[KEY, "1.25"]]);

      setTextScale(DEFAULT_TEXT_SCALE);
      assert.equal(getTextScale(), DEFAULT_TEXT_SCALE);
      assert.deepEqual(removals, [KEY]);
    },
  );
});

test("adjustTextScale steps and resets within bounds", async () => {
  await withStorage(
    {
      getItem: () => null,
      setItem() {},
      removeItem() {},
    },
    ({
      adjustTextScale,
      setTextScale,
      MIN_TEXT_SCALE,
      MAX_TEXT_SCALE,
      DEFAULT_TEXT_SCALE,
    }) => {
      setTextScale(1);
      assert.equal(adjustTextScale("increase"), 1.1);
      assert.equal(adjustTextScale("decrease"), 1);
      assert.equal(adjustTextScale("reset"), DEFAULT_TEXT_SCALE);

      setTextScale(MAX_TEXT_SCALE);
      assert.equal(adjustTextScale("increase"), MAX_TEXT_SCALE);

      setTextScale(1.5);
      assert.equal(adjustTextScale("increase"), 1.75);

      setTextScale(4);
      assert.equal(adjustTextScale("increase"), 5);
      assert.equal(adjustTextScale("increase"), 5);

      setTextScale(MIN_TEXT_SCALE);
      assert.equal(adjustTextScale("decrease"), MIN_TEXT_SCALE);
    },
  );
});

test("keyboard increase advances beyond 150% and stops at 500%", async () => {
  await withStorage(
    {
      getItem: () => null,
      setItem() {},
      removeItem() {},
    },
    ({ adjustTextScale, setTextScale }) => {
      setTextScale(1.5);
      assert.equal(adjustTextScale("increase"), 1.75);
      setTextScale(4);
      assert.equal(adjustTextScale("increase"), 5);
      assert.equal(adjustTextScale("increase"), 5);
    },
  );
});

test("formatTextScalePercent and normalize helpers", async () => {
  await withStorage(
    { getItem: () => null, setItem() {}, removeItem() {} },
    ({
      formatTextScalePercent,
      normalizeTextScale,
      clampTextScale,
      textScalePresetIndex,
      TEXT_SCALE_PRESETS,
      MAX_TEXT_SCALE,
    }) => {
      assert.deepEqual([...TEXT_SCALE_PRESETS], [...APPEARANCE_SCALE_PRESETS]);
      assert.equal(formatTextScalePercent(1), "100%");
      assert.equal(formatTextScalePercent(1.25), "125%");
      assert.equal(formatTextScalePercent(5), "500%");
      assert.equal(normalizeTextScale(1.26), 1.25);
      assert.equal(normalizeTextScale(1.4), 1.5);
      assert.equal(normalizeTextScale(0.75), 0.75);
      assert.equal(clampTextScale(9), MAX_TEXT_SCALE);
      assert.equal(normalizeTextScale(Number.NaN), 1);
      assert.equal(textScalePresetIndex(1), TEXT_SCALE_PRESETS.indexOf(1));
      assert.equal(textScalePresetIndex(0.75), 0);
      assert.equal(textScalePresetIndex(5), TEXT_SCALE_PRESETS.length - 1);
    },
  );
});

test("keeps the in-memory choice when persistence fails", async () => {
  await withStorage(
    {
      getItem: () => null,
      setItem() {
        throw new Error("quota exceeded");
      },
      removeItem() {
        throw new Error("quota exceeded");
      },
    },
    ({ getTextScale, setTextScale }) => {
      assert.doesNotThrow(() => setTextScale(1.5));
      assert.equal(getTextScale(), 1.5);
    },
  );
});
