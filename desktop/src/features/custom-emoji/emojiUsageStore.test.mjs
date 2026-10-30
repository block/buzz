import assert from "node:assert/strict";
import test from "node:test";

class MemoryStorage {
  data = new Map();
  getItem(key) {
    return this.data.get(key) ?? null;
  }
  setItem(key, value) {
    this.data.set(key, String(value));
  }
  removeItem(key) {
    this.data.delete(key);
  }
  key(i) {
    return [...this.data.keys()][i] ?? null;
  }
  get length() {
    return this.data.size;
  }
}
globalThis.window = { localStorage: new MemoryStorage() };

const store = await import("./emojiUsageStore.ts");
const e = (shortcode) => ({ shortcode, url: `https://x/${shortcode}.png` });

test("usage is bounded, ordered, persisted per community, and reset clears it", () => {
  store.resetEmojiUsage();
  store.loadEmojiUsage("wss://a.example/");
  for (let i = 0; i < store.EMOJI_USAGE_CAP + 10; i += 1) {
    store.recordEmojiUse(e(`emoji${i}`), 1000 + i);
  }
  store.recordEmojiUse(e("emoji60"), 5000);
  store.recordEmojiUse(e("emoji60"), 5001);

  assert.equal(store.getEmojiUsageSnapshot().length, store.EMOJI_USAGE_CAP);
  assert.equal(store.recentEmoji(1)[0].shortcode, "emoji60");
  assert.equal(store.frequentEmoji(1)[0].shortcode, "emoji60");
  assert.equal(store.recentEmoji(5).length, 5);

  store.resetEmojiUsage();
  assert.equal(store.getEmojiUsageSnapshot().length, 0);

  store.loadEmojiUsage("wss://b.example/");
  assert.equal(store.getEmojiUsageSnapshot().length, 0);

  store.loadEmojiUsage("wss://a.example");
  assert.equal(store.getEmojiUsageSnapshot().length, store.EMOJI_USAGE_CAP);
  assert.equal(store.recentEmoji(1)[0].shortcode, "emoji60");
});

test("a corrupt or oversized persisted value loads as a bounded valid list", () => {
  store.resetEmojiUsage();
  const key = [...window.localStorage.data.keys()][0];
  window.localStorage.setItem(key, "{not json");
  store.loadEmojiUsage("wss://a.example/");
  assert.equal(store.getEmojiUsageSnapshot().length, 0);

  const many = Array.from({ length: 500 }, (_, i) => ({
    shortcode: `s${i}`,
    url: "https://x/y.png",
    count: 1,
    lastUsed: i,
  }));
  window.localStorage.setItem(key, JSON.stringify([...many, { bad: true }]));
  store.resetEmojiUsage();
  store.loadEmojiUsage("wss://a.example/");
  assert.equal(store.getEmojiUsageSnapshot().length, store.EMOJI_USAGE_CAP);
});
