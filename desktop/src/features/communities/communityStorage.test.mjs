import assert from "node:assert/strict";
import test from "node:test";

import {
  clearCommunityStorage,
  migrateLegacyCommunityStorage,
} from "./communityStorage.ts";

function createMemoryStorage(initial = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, String(value)),
    removeItem: (key) => values.delete(key),
    clear: () => values.clear(),
    key: (index) => Array.from(values.keys())[index] ?? null,
    get length() {
      return values.size;
    },
  };
}

test("migrateLegacyCommunityStorage promotes current Buzz workspace state", () => {
  const storage = createMemoryStorage({
    "buzz-workspaces": '[{"id":"current"}]',
    "buzz-active-workspace-id": "current",
  });

  migrateLegacyCommunityStorage(storage);

  assert.equal(storage.getItem("buzz-communities"), '[{"id":"current"}]');
  assert.equal(storage.getItem("buzz-active-community-id"), "current");
});

test("migrateLegacyCommunityStorage does not overwrite new community state", () => {
  const storage = createMemoryStorage({
    "buzz-communities": '[{"id":"new"}]',
    "buzz-active-community-id": "new",
    "buzz-workspaces": '[{"id":"old"}]',
    "buzz-active-workspace-id": "old",
  });

  migrateLegacyCommunityStorage(storage);

  assert.equal(storage.getItem("buzz-communities"), '[{"id":"new"}]');
  assert.equal(storage.getItem("buzz-active-community-id"), "new");
});

test("clearCommunityStorage removes new and legacy state", () => {
  const storage = createMemoryStorage({
    "buzz-communities": "new",
    "buzz-active-community-id": "new",
    "buzz-workspaces": "old",
    "buzz-active-workspace-id": "old",
  });

  clearCommunityStorage(storage);
  migrateLegacyCommunityStorage(storage);

  assert.equal(storage.length, 0);
});
