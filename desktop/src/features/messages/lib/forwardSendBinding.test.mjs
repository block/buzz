/**
 * The optimistic half of a forward.
 *
 * `useSendMessageMutation` inserts `createOptimisticMessage`'s row before the
 * signed event comes back, so whatever attribution the row is missing is
 * attribution the user watches pop in a moment later. These tests pin that the
 * provenance tag is on the optimistic row, and that building it goes through
 * the same validation the signed send uses.
 */

import assert from "node:assert/strict";
import test from "node:test";

import { createOptimisticMessage } from "../hooks.ts";
import { FORWARDED_FROM_TAG } from "./forwardedMessage.ts";

const IDENTITY = { pubkey: "a".repeat(64) };
const AUTHOR = "b".repeat(64);
const SOURCE_EVENT = "c".repeat(64);
const SOURCE_CHANNEL = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";
const DESTINATION_CHANNEL = "7c9e6679-7425-40de-944b-e07fc1f90ae7";

const FORWARDED_FROM = {
  sourceEventId: SOURCE_EVENT,
  sourceChannelId: SOURCE_CHANNEL,
  authorPubkey: AUTHOR,
  threadRootId: null,
};

function optimisticForward(forwardedFrom = FORWARDED_FROM) {
  return createOptimisticMessage(
    DESTINATION_CHANNEL,
    "ship it",
    IDENTITY,
    [],
    [],
    null,
    [],
    null,
    null,
    forwardedFrom,
  );
}

test("the optimistic forward row carries its provenance immediately", () => {
  const row = optimisticForward();

  assert.deepEqual(
    row.tags.find(([name]) => name === FORWARDED_FROM_TAG),
    [FORWARDED_FROM_TAG, SOURCE_EVENT, SOURCE_CHANNEL, AUTHOR],
  );
  // The forward lands in the destination as a fresh top-level message: it is
  // addressed to the destination channel, and it is not a reply to anything.
  assert.deepEqual(
    row.tags.find(([name]) => name === "h"),
    ["h", DESTINATION_CHANNEL],
  );
  assert.equal(
    row.tags.some(([name]) => name === "e"),
    false,
  );
  // Only the sender is p-tagged; the source message's audience is not paged.
  assert.deepEqual(
    row.tags.filter(([name]) => name === "p"),
    [["p", IDENTITY.pubkey]],
  );
  assert.equal(row.pending, true);
});

test("an ordinary send is unchanged by the forward plumbing", () => {
  const row = optimisticForward(null);
  assert.equal(
    row.tags.some(([name]) => name === FORWARDED_FROM_TAG),
    false,
  );
});

test("the optimistic row refuses provenance the backend would reject", () => {
  assert.throws(
    () =>
      optimisticForward({ ...FORWARDED_FROM, sourceChannelId: "not-a-uuid" }),
    /complete source reference/,
  );
});
