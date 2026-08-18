import assert from "node:assert/strict";
import test from "node:test";
import { JSDOM } from "jsdom";

import {
  recordQuickReactionEmoji,
  resolveQuickReactionEmojis,
  useQuickReactionEmojis,
} from "./useQuickReactionEmojis.ts";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});
globalThis.window = dom.window;
globalThis.document = dom.window.document;
globalThis.CustomEvent = dom.window.CustomEvent;
globalThis.HTMLElement = dom.window.HTMLElement;

function entry(emoji) {
  return { emoji };
}

test("quick reactions backfill defaults after stale custom emoji", () => {
  assert.deepEqual(
    resolveQuickReactionEmojis(
      [
        entry(":gone_one:"),
        entry(":gone_two:"),
        entry(":gone_three:"),
        entry(":gone_four:"),
      ],
      4,
      [],
    ),
    ["👍", "❤️", "😂", "🎉"],
  );
});

test("quick reactions skip stale custom emoji before applying the limit", () => {
  assert.deepEqual(
    resolveQuickReactionEmojis(
      [
        entry(":gone_one:"),
        entry(":gone_two:"),
        entry(":gone_three:"),
        entry(":gone_four:"),
        entry(":shipit:"),
        entry("🔥"),
      ],
      4,
      [{ shortcode: "shipit", url: "https://relay/shipit.png" }],
    ),
    [":shipit:", "🔥", "👍", "❤️"],
  );
});

test("recording an emoji updates mounted quick reactions immediately", async () => {
  window.localStorage.clear();
  const { act, cleanup, renderHook } = await import("@testing-library/react");

  try {
    const { result } = renderHook(() => useQuickReactionEmojis());
    assert.deepEqual(result.current, ["👍", "❤️", "😂", "🎉"]);

    act(() => recordQuickReactionEmoji("🔥"));
    assert.deepEqual(result.current, ["🔥", "👍", "❤️", "😂"]);

    act(() => recordQuickReactionEmoji("🎉"));
    act(() => recordQuickReactionEmoji("🎉"));
    assert.deepEqual(result.current, ["🎉", "🔥", "👍", "❤️"]);
  } finally {
    cleanup();
    window.localStorage.clear();
  }
});
