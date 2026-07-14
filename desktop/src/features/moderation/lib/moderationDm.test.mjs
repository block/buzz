import assert from "node:assert/strict";
import test from "node:test";

import { isModerationDm } from "./moderationDm.ts";

const RELAY = "a".repeat(64);
const ME = "b".repeat(64);
const OTHER = "c".repeat(64);

const dm = (participantPubkeys) => ({ channelType: "dm", participantPubkeys });

test("moderation DM: 1:1 with the relay self is detected", () => {
  assert.equal(isModerationDm(dm([ME, RELAY]), ME, RELAY), true);
});

test("moderation DM: case-insensitive on both self and participants", () => {
  assert.equal(isModerationDm(dm([ME, RELAY.toUpperCase()]), ME, RELAY), true);
});

test("ordinary DM with another member is not a moderation DM", () => {
  assert.equal(isModerationDm(dm([ME, OTHER]), ME, RELAY), false);
});

test("group DM including the relay is not a moderation DM", () => {
  assert.equal(isModerationDm(dm([ME, RELAY, OTHER]), ME, RELAY), false);
});

test("non-DM channels are never moderation DMs", () => {
  assert.equal(
    isModerationDm(
      { channelType: "stream", participantPubkeys: [ME, RELAY] },
      ME,
      RELAY,
    ),
    false,
  );
});

test("fails open when relay self is null/undefined", () => {
  assert.equal(isModerationDm(dm([ME, RELAY]), ME, null), false);
  assert.equal(isModerationDm(dm([ME, RELAY]), ME, undefined), false);
});

test("null channel is not a moderation DM", () => {
  assert.equal(isModerationDm(null, ME, RELAY), false);
});
