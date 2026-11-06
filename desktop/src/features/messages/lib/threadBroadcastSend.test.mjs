import assert from "node:assert/strict";
import test from "node:test";

import { createOptimisticMessage } from "../hooks.ts";
import { isBroadcastReply, isThreadReply } from "./threading.ts";

const IDENTITY = {
  pubkey: "aaaa1111bbbb2222cccc3333dddd4444eeee5555ffff6666aaaa1111bbbb2222",
};
const ROOT_ID = "b".repeat(64);

function optimisticReply(broadcast) {
  return createOptimisticMessage(
    "channel-A",
    "hello",
    IDENTITY,
    [],
    [],
    ROOT_ID,
    [],
    null,
    null,
    broadcast,
  );
}

test("createOptimisticMessage_broadcastReply_showsInChannelTimeline", () => {
  const message = optimisticReply(true);

  assert.ok(isBroadcastReply(message.tags));
  assert.equal(isThreadReply(message.tags), false);
  assert.ok(
    message.tags.some(
      ([name, id, , marker]) =>
        name === "e" && id === ROOT_ID && marker === "reply",
    ),
    "a broadcast reply keeps its thread reference",
  );
});

test("createOptimisticMessage_plainReply_staysInThread", () => {
  const message = optimisticReply(false);

  assert.equal(isBroadcastReply(message.tags), false);
  assert.equal(isThreadReply(message.tags), true);
});

test("createOptimisticMessage_topLevelBroadcast_addsNoBroadcastTag", () => {
  const message = createOptimisticMessage(
    "channel-A",
    "hello",
    IDENTITY,
    [],
    [],
    null,
    [],
    null,
    null,
    true,
  );

  assert.equal(isBroadcastReply(message.tags), false);
});
