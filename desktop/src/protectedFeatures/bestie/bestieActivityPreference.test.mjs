import assert from "node:assert/strict";
import test from "node:test";

import {
  BESTIE_SHOW_ACTIVITY_STORAGE_KEY,
  setBestieShowActivity,
  useBestieShowActivity,
} from "./bestieActivityPreference.ts";

test("setBestieShowActivity persists and defaults off", () => {
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

  setBestieShowActivity(false);
  assert.equal(memory.get(BESTIE_SHOW_ACTIVITY_STORAGE_KEY), "0");
  setBestieShowActivity(true);
  assert.equal(memory.get(BESTIE_SHOW_ACTIVITY_STORAGE_KEY), "1");
  assert.equal(typeof useBestieShowActivity, "function");
});
