import assert from "node:assert/strict";
import test from "node:test";

import { clearLegacyPersonaCatalogVisibility } from "./legacyPersonaCatalogVisibility.ts";

test("clearLegacyPersonaCatalogVisibility removes the retired preference", () => {
  const removedKeys = [];

  clearLegacyPersonaCatalogVisibility({
    removeItem(key) {
      removedKeys.push(key);
    },
  });

  assert.deepEqual(removedKeys, ["buzz-persona-catalog-visibility-v1"]);
});

test("clearLegacyPersonaCatalogVisibility ignores unavailable storage", () => {
  assert.doesNotThrow(() => clearLegacyPersonaCatalogVisibility(null));
  assert.doesNotThrow(() =>
    clearLegacyPersonaCatalogVisibility({
      removeItem() {
        throw new Error("storage unavailable");
      },
    }),
  );
});
