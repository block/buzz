import assert from "node:assert/strict";
import test from "node:test";

class MemoryStorage {
  values = new Map();
  getItem(key) {
    return this.values.get(key) ?? null;
  }
  setItem(key, value) {
    this.values.set(key, value);
  }
}

globalThis.window = { localStorage: new MemoryStorage() };
const { loadHuddleBackingChannelIds, rememberHuddleBackingChannelId } =
  await import("./huddleBackingChannelStorage.ts");

test("restores remembered Huddle backing channels", () => {
  rememberHuddleBackingChannelId("first");
  rememberHuddleBackingChannelId("second");
  rememberHuddleBackingChannelId("first");
  assert.deepEqual([...loadHuddleBackingChannelIds()], ["second", "first"]);
});

test("bounds persisted backing channels", () => {
  for (let index = 0; index < 105; index += 1) {
    rememberHuddleBackingChannelId(`channel-${index}`);
  }
  const ids = [...loadHuddleBackingChannelIds()];
  assert.equal(ids.length, 100);
  assert.equal(ids.at(0), "channel-5");
  assert.equal(ids.at(-1), "channel-104");
});
