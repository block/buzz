import assert from "node:assert/strict";
import test from "node:test";

import { getOverrides, OVERRIDES_KEY } from "./store.ts";

function installStorage(value) {
  const values = new Map([[OVERRIDES_KEY, JSON.stringify(value)]]);
  globalThis.window = {
    localStorage: {
      getItem: (key) => values.get(key) ?? null,
      setItem: (key, next) => values.set(key, String(next)),
    },
  };
  return values;
}

test("getOverrides drops unknown feature IDs and compacts persisted storage", () => {
  const values = installStorage({ workflows: true, removedFeature: false });

  assert.deepEqual(getOverrides(), { workflows: true });
  assert.equal(values.get(OVERRIDES_KEY), JSON.stringify({ workflows: true }));
});

test("getOverrides drops non-boolean values", () => {
  installStorage({ workflows: "yes", projects: false });

  assert.deepEqual(getOverrides(), { projects: false });
});
