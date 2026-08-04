import assert from "node:assert/strict";
import test from "node:test";

import {
  channelManualOrderStorageKey,
  readChannelManualOrderStore,
  writeChannelManualOrderStore,
} from "./channelManualOrderStorage.ts";

const storage = new Map();
globalThis.window = {
  localStorage: {
    getItem: (key) => storage.get(key) ?? null,
    setItem: (key, value) => storage.set(key, value),
  },
};

test("storage is scoped by identity and normalized relay", () => {
  const a = channelManualOrderStorageKey("pk", "WSS://Relay.Example.com/");
  const b = channelManualOrderStorageKey("pk", "wss://relay.example.com");
  const c = channelManualOrderStorageKey("other", "wss://relay.example.com");
  assert.equal(a, b);
  assert.notEqual(a, c);
});

test("store round-trips and corrupt payload fails safe", () => {
  const store = {
    version: 1,
    groups: { channels: ["b", "a"] },
    manualGroups: ["channels"],
  };
  assert.equal(
    writeChannelManualOrderStore("pk-roundtrip", store, "wss://relay.test"),
    true,
  );
  assert.deepEqual(
    readChannelManualOrderStore("pk-roundtrip", "wss://relay.test"),
    store,
  );

  const key = channelManualOrderStorageKey("pk-corrupt", "wss://relay.test");
  storage.set(key, "{broken");
  assert.deepEqual(
    readChannelManualOrderStore("pk-corrupt", "wss://relay.test"),
    { version: 1, groups: {}, manualGroups: [] },
  );
});
