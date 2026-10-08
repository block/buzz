import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { deriveUnreadOverflow } from "./useUnreadOverflow.ts";

const root = {
  getBoundingClientRect: () => ({ height: 100, top: 50 }),
};

function entry(channelId, top, isIntersecting = false) {
  return {
    element: {
      getAttribute: (name) => (name === "data-channel-id" ? channelId : null),
      getBoundingClientRect: () => ({ top }),
    },
    isIntersecting,
  };
}

describe("deriveUnreadOverflow", () => {
  it("orders offscreen channels by position", () => {
    const result = deriveUnreadOverflow(
      [
        entry("below-2", 180),
        entry("above-2", 0),
        entry("below-1", 160),
        entry("above-1", 20),
      ],
      root,
    );

    assert.deepEqual(result.unreadAboveChannelIds, ["above-1", "above-2"]);
    assert.deepEqual(result.unreadBelowChannelIds, ["below-1", "below-2"]);
    assert.equal(result.unreadAboveCount, 2);
    assert.equal(result.unreadBelowCount, 2);
  });

  it("deduplicates repeated sidebar rows for the same channel", () => {
    const result = deriveUnreadOverflow(
      [entry("starred", 170), entry("starred", 240)],
      root,
    );

    assert.deepEqual(result.unreadBelowChannelIds, ["starred"]);
    assert.equal(result.unreadBelowCount, 1);
  });

  it("does not count a duplicate row when the same channel is visible", () => {
    const result = deriveUnreadOverflow(
      [entry("starred", 80, true), entry("starred", 240)],
      root,
    );

    assert.deepEqual(result.unreadBelowChannelIds, []);
    assert.equal(result.unreadBelowCount, 0);
  });

  it("ignores visible rows", () => {
    const result = deriveUnreadOverflow(
      [entry("visible", 80, true), entry("below", 180)],
      root,
    );

    assert.deepEqual(result.unreadBelowChannelIds, ["below"]);
  });
});
