import assert from "node:assert/strict";
import test from "node:test";

import { eventNotifyMode, isBroadcastReply } from "./threading.ts";

// `notify` (channel-wide mention, NIP-CN/#3146) and `broadcast` (NIP-CW reply
// surfaced to the timeline) are separate concepts gated by separate
// preferences, so the two readers must never see each other's tag.

test("eventNotifyMode reads the channel and here markers", () => {
  assert.equal(eventNotifyMode([["notify", "channel"]]), "channel");
  assert.equal(eventNotifyMode([["notify", "here"]]), "here");
  assert.equal(
    eventNotifyMode([
      ["h", "chan-1"],
      ["notify", "here"],
    ]),
    "here",
  );
});

test("eventNotifyMode ignores absent, empty and unknown markers", () => {
  assert.equal(eventNotifyMode([]), null);
  assert.equal(eventNotifyMode([["notify"]]), null);
  assert.equal(eventNotifyMode([["notify", "everyone"]]), null);
  assert.equal(eventNotifyMode([["h", "chan-1"]]), null);
});

test("notify and broadcast markers do not alias each other", () => {
  assert.equal(eventNotifyMode([["broadcast", "1"]]), null);
  assert.equal(isBroadcastReply([["notify", "channel"]]), false);
});
