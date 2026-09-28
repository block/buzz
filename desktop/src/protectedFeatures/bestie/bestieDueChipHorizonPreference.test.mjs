import assert from "node:assert/strict";
import test from "node:test";

import {
  BESTIE_DUE_CHIP_HORIZON_STORAGE_KEY,
  setBestieDueChipHorizonMinutes,
  useBestieDueChipHorizonMinutes,
} from "./bestieDueChipHorizonPreference.ts";

test("setBestieDueChipHorizonMinutes persists minutes", () => {
  const memory = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (key) => memory.get(key) ?? null,
      removeItem: (key) => {
        memory.delete(key);
      },
      setItem: (key, value) => {
        memory.set(key, String(value));
      },
    },
  };

  setBestieDueChipHorizonMinutes(30);
  assert.equal(memory.get(BESTIE_DUE_CHIP_HORIZON_STORAGE_KEY), "30");
  setBestieDueChipHorizonMinutes(0);
  assert.equal(memory.get(BESTIE_DUE_CHIP_HORIZON_STORAGE_KEY), "0");
  setBestieDueChipHorizonMinutes(120);
  assert.equal(memory.get(BESTIE_DUE_CHIP_HORIZON_STORAGE_KEY), "120");
  assert.equal(typeof useBestieDueChipHorizonMinutes, "function");
});
