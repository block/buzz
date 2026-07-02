/**
 * Regression tests for wrong-channel send bug.
 *
 * The bug: when a channel switch happens mid-send (during the async agent-prep
 * await in useMentionSendFlow), the "latest-value" onSendRef and sendMutateRef
 * would already point at the new channel. The fix threads capturedChannelId as
 * data through the entire pipeline so the mutation always targets the
 * compose-time channel regardless of navigation.
 *
 * These tests cover the pure / pure-ish slice of the invariant:
 *   - createOptimisticMessage uses the supplied channelId, not any global state
 *   - The message event tags carry the correct channel h-tag
 */

import assert from "node:assert/strict";
import test from "node:test";

import { createOptimisticMessage } from "../hooks.ts";

// ---------------------------------------------------------------------------
// Minimal identity stub
// ---------------------------------------------------------------------------
const IDENTITY = {
  pubkey: "aaaa1111bbbb2222cccc3333dddd4444eeee5555ffff6666aaaa1111bbbb2222",
};

// ---------------------------------------------------------------------------
// createOptimisticMessage uses the supplied channelId for the h-tag
// ---------------------------------------------------------------------------

test("createOptimisticMessage_composedChannelId_hTagMatchesComposedChannel", () => {
  const composeChannelId = "channel-A";
  const msg = createOptimisticMessage(
    composeChannelId,
    "hello",
    IDENTITY,
    [],   // currentMessages
    [],   // mentionPubkeys
    null, // parentEventId
    [],   // mediaTags
  );

  const hTag = msg.tags.find(([name]) => name === "h");
  assert.ok(hTag, "message must have an h-tag");
  assert.equal(
    hTag[1],
    composeChannelId,
    "h-tag must match the compose-time channelId, not any other channel",
  );
  assert.equal(msg.content, "hello");
  assert.equal(msg.pending, true);
});

test("createOptimisticMessage_differentChannelIds_hTagsAreIndependent", () => {
  // Simulate two messages composed in two different channels.
  // If a channel switch had corrupted channelId, both would carry the same tag.
  const msgA = createOptimisticMessage("channel-A", "msg A", IDENTITY, [], [], null, []);
  const msgB = createOptimisticMessage("channel-B", "msg B", IDENTITY, [], [], null, []);

  const hTagA = msgA.tags.find(([n]) => n === "h");
  const hTagB = msgB.tags.find(([n]) => n === "h");

  assert.equal(hTagA[1], "channel-A", "message A must target channel-A");
  assert.equal(hTagB[1], "channel-B", "message B must target channel-B");
  assert.notEqual(
    hTagA[1],
    hTagB[1],
    "compose-time channel isolation: the two h-tags must differ",
  );
});

test("createOptimisticMessage_withReply_hTagStillCarriesSuppliedChannelId", () => {
  // Thread replies also carry the h-tag via buildReplyTags.
  // Verify the channel id flows through when a parentEventId is set.
  const composeChannelId = "channel-A";
  const parentEvent = createOptimisticMessage("channel-A", "parent", IDENTITY, [], [], null, []);
  const replyMsg = createOptimisticMessage(
    composeChannelId,
    "reply",
    IDENTITY,
    [parentEvent],
    [],
    parentEvent.id,
    [],
  );

  const hTag = replyMsg.tags.find(([name]) => name === "h");
  assert.ok(hTag, "reply must have an h-tag");
  assert.equal(
    hTag[1],
    composeChannelId,
    "reply h-tag must match the compose-time channelId",
  );
});
