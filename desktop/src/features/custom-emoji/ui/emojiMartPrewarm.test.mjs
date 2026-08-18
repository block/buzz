import assert from "node:assert/strict";
import test from "node:test";

const entries = new Map([
  ["emoji-mart.frequently", "{}"],
  ["emoji-mart.last", "thumbsup"],
]);
const idleCalls = [];
const initCalls = [];

globalThis.window = {
  localStorage: {
    getItem: (key) => entries.get(key) ?? null,
    removeItem: (key) => entries.delete(key),
  },
  requestIdleCallback: (callback, options) => {
    idleCalls.push({ callback, options });
    return 1;
  },
};
globalThis.__BUZZ_TEST_EMOJI_MART_INIT__ = (...args) => initCalls.push(args);

const { emojiMartData } = await import("./emojiMartPrewarm.ts");

test("normalizes poisoned recents before scheduling the emoji index prewarm", () => {
  assert.equal(entries.has("emoji-mart.frequently"), false);
  assert.equal(entries.has("emoji-mart.last"), false);
  assert.equal(initCalls.length, 0);
  assert.equal(idleCalls.length, 1);
  assert.deepEqual(idleCalls[0].options, { timeout: 1_500 });

  idleCalls[0].callback();
  assert.equal(initCalls.length, 1);
  assert.equal(initCalls[0][0].data, emojiMartData);
});
