import assert from "node:assert/strict";
import test from "node:test";

import {
  readThreadStarsStore,
  writeThreadStarsStore,
  starredThreadIdsFromStore,
} from "./threadStarsStorage.ts";
import { __resetThreadStarsCacheForTests } from "./useThreadStars.ts";

const memory = new Map();

function installLocalStorage() {
  globalThis.window = {
    localStorage: {
      getItem(key) {
        return memory.has(key) ? memory.get(key) : null;
      },
      setItem(key, value) {
        memory.set(key, String(value));
      },
      removeItem(key) {
        memory.delete(key);
      },
    },
    addEventListener() {},
    removeEventListener() {},
  };
}

test("thread star store round-trip via localStorage", () => {
  memory.clear();
  __resetThreadStarsCacheForTests();
  installLocalStorage();

  const pubkey = "npub-test";
  const ok = writeThreadStarsStore(pubkey, {
    version: 1,
    threads: {
      rootA: {
        rootId: "rootA",
        channelId: "chanA",
        title: "Hello",
        channelName: "general",
        starredAt: 42,
      },
    },
  });
  assert.equal(ok, true);
  __resetThreadStarsCacheForTests();
  const store = readThreadStarsStore(pubkey);
  assert.deepEqual([...starredThreadIdsFromStore(store)], ["rootA"]);
  assert.equal(store.threads.rootA.title, "Hello");
});
