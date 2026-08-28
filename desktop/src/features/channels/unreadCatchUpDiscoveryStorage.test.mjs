import assert from "node:assert/strict";
import test from "node:test";
import {
  advanceCatchUpDiscoveryAt,
  readCatchUpDiscoveryAt,
} from "./unreadCatchUpDiscoveryStorage.ts";

function installStorage() {
  const values = new Map();
  globalThis.window = {
    localStorage: {
      getItem: (key) => values.get(key) ?? null,
      setItem: (key, value) => values.set(key, value),
    },
  };
}

test("fresh scope has no discovery watermark despite synced read state", () => {
  installStorage();
  assert.equal(readCatchUpDiscoveryAt("pk:relay", "channel"), null);
});

test("a completed scan advances its upper bound monotonically per device scope", () => {
  installStorage();
  advanceCatchUpDiscoveryAt("pk:relay", "channel", 900);
  advanceCatchUpDiscoveryAt("pk:relay", "channel", 100);
  assert.equal(readCatchUpDiscoveryAt("pk:relay", "channel"), 900);
  assert.equal(readCatchUpDiscoveryAt("other:relay", "channel"), null);
});
