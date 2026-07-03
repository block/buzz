/**
 * Regression tests for wrong-channel send bug.
 *
 * The bug: when a channel switch happens mid-send (during the async agent-prep
 * await in useMentionSendFlow), the "latest-value" onSendRef and sendMutateRef
 * would already point at the new channel. The fix threads capturedChannelId as
 * data through the entire pipeline so the mutation always targets the
 * compose-time channel regardless of navigation.
 *
 * Test coverage:
 *   1. createOptimisticMessage uses the supplied channelId for the h-tag.
 *   2. resolveEffectiveChannel pins the send to the captured channel even when
 *      the closed-over channel is different (the core invariant).
 *   3. resolveEffectiveChannel returns null for a supplied-but-unresolvable id
 *      so the caller can throw rather than silently misdeliver.
 *   4. resolveEffectiveChannel falls back to the closed-over channel when no
 *      capturedChannelId was supplied (legacy-caller path).
 */

import assert from "node:assert/strict";
import test from "node:test";

import {
  createOptimisticMessage,
  resolveEffectiveChannel,
  resolveThreadReplyTarget,
} from "../hooks.ts";

// ---------------------------------------------------------------------------
// Minimal stubs
// ---------------------------------------------------------------------------
const IDENTITY = {
  pubkey: "aaaa1111bbbb2222cccc3333dddd4444eeee5555ffff6666aaaa1111bbbb2222",
};

function makeChannel(id) {
  return {
    id,
    name: id,
    channelType: "channel",
    // Only id and channelType are required by the resolution logic.
  };
}

// ---------------------------------------------------------------------------
// createOptimisticMessage — h-tag carries the compose-time channelId
// ---------------------------------------------------------------------------

test("createOptimisticMessage_composedChannelId_hTagMatchesComposedChannel", () => {
  const composeChannelId = "channel-A";
  const msg = createOptimisticMessage(
    composeChannelId,
    "hello",
    IDENTITY,
    [], // currentMessages
    [], // mentionPubkeys
    null, // parentEventId
    [], // mediaTags
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
  const msgA = createOptimisticMessage(
    "channel-A",
    "msg A",
    IDENTITY,
    [],
    [],
    null,
    [],
  );
  const msgB = createOptimisticMessage(
    "channel-B",
    "msg B",
    IDENTITY,
    [],
    [],
    null,
    [],
  );

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
  const parentEvent = createOptimisticMessage(
    "channel-A",
    "parent",
    IDENTITY,
    [],
    [],
    null,
    [],
  );
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

// ---------------------------------------------------------------------------
// resolveEffectiveChannel — the channel-binding invariant
// ---------------------------------------------------------------------------

test("resolveEffectiveChannel_capturedIdPresentInCache_returnsComposeTimeChannel", () => {
  // Core invariant: closure channel is B, variables carry channel A.
  // The mutation must target A regardless of what the closure says.
  const channelA = makeChannel("channel-A");
  const channelB = makeChannel("channel-B");
  const cache = [channelA, channelB];

  const result = resolveEffectiveChannel("channel-A", cache, channelB);

  assert.strictEqual(
    result?.id,
    "channel-A",
    "must return the compose-time channel even when the closed-over channel is B",
  );
});

test("resolveEffectiveChannel_capturedIdNotInCache_returnsNull", () => {
  // F3 invariant: a supplied-but-unresolvable id must not fall back to the
  // live channel — the caller is expected to throw "channel no longer available".
  const channelB = makeChannel("channel-B");
  const cache = [channelB]; // channel-A is absent (e.g. new channel, cache miss)

  const result = resolveEffectiveChannel("channel-A", cache, channelB);

  assert.strictEqual(
    result,
    null,
    "a supplied-but-unresolvable capturedChannelId must return null, not the live channel",
  );
});

test("resolveEffectiveChannel_capturedIdNull_returnsFallbackChannel", () => {
  // Legacy callers (thread reply, InboxDetailPane) don't supply a capturedId.
  // They rely on the closed-over channel being correct for other reasons.
  const channelB = makeChannel("channel-B");
  const cache = [channelB];

  const result = resolveEffectiveChannel(null, cache, channelB);

  assert.strictEqual(
    result?.id,
    "channel-B",
    "null capturedChannelId must fall through to the closed-over channel",
  );
});

test("resolveEffectiveChannel_capturedIdUndefined_returnsFallbackChannel", () => {
  // Same as null — undefined means the caller didn't capture an id.
  const channelB = makeChannel("channel-B");

  const result = resolveEffectiveChannel(undefined, [channelB], channelB);

  assert.strictEqual(result?.id, "channel-B");
});

test("resolveEffectiveChannel_emptyCache_capturedIdPresent_returnsNull", () => {
  // Cache was wiped (e.g. sign-out race). Must not fall back to live channel.
  const channelB = makeChannel("channel-B");

  const result = resolveEffectiveChannel("channel-A", [], channelB);

  assert.strictEqual(result, null);
});

// ---------------------------------------------------------------------------
// resolveThreadReplyTarget — flush-time resolution for handleSendThreadReply
//
// These tests exercise the production resolveThreadReplyTarget function.
// Key invariant (the race): when a captured context is provided, the live
// ref values (liveReplyTargetId, liveThreadHeadId) must be IGNORED even if
// they point at a different thread — they represent post-navigation state.
// ---------------------------------------------------------------------------

test("resolveThreadReplyTarget_capturedContext_ignoresLiveRefs", () => {
  // The race scenario: compose-time context captured A, live refs now point
  // at B (user switched threads mid-send).
  const result = resolveThreadReplyTarget(
    { parentEventId: "parent-A", threadHeadId: "head-A" },
    /* liveReplyTargetId = */ "parent-B",
    /* liveThreadHeadId = */ "head-B",
  );

  assert.deepStrictEqual(result, {
    parentEventId: "parent-A",
    threadHeadId: "head-A",
  });
});

test("resolveThreadReplyTarget_capturedContextNullParent_returnsNull", () => {
  // Captured context has no parentEventId — bail before any await fires.
  const result = resolveThreadReplyTarget(
    { parentEventId: null, threadHeadId: "head-A" },
    "live-parent",
    "live-head",
  );

  assert.strictEqual(result, null);
});

test("resolveThreadReplyTarget_capturedContextNullThreadHead_usesItNotLiveRef", () => {
  // F7 degenerate case: threadContext is non-null but threadHeadId is null.
  // Must not fall through to the live ref — use null from the context itself.
  const result = resolveThreadReplyTarget(
    { parentEventId: "parent-A", threadHeadId: null },
    "live-parent",
    "live-head",
  );

  assert.deepStrictEqual(result, {
    parentEventId: "parent-A",
    threadHeadId: null,
  });
});

test("resolveThreadReplyTarget_nullContext_fallsBackToLiveRefs", () => {
  // Legacy path: no captured context — fall back to live refs.
  const result = resolveThreadReplyTarget(
    null,
    /* liveReplyTargetId = */ "live-parent",
    /* liveThreadHeadId = */ "live-head",
  );

  assert.deepStrictEqual(result, {
    parentEventId: "live-parent",
    threadHeadId: "live-head",
  });
});

test("resolveThreadReplyTarget_nullContext_noLiveReplyTarget_fallsBackToThreadHead", () => {
  // When there is no specific reply target (just the thread head), parentEventId
  // equals the thread head.
  const result = resolveThreadReplyTarget(
    null,
    /* liveReplyTargetId = */ null,
    /* liveThreadHeadId = */ "head-only",
  );

  assert.deepStrictEqual(result, {
    parentEventId: "head-only",
    threadHeadId: "head-only",
  });
});

test("resolveThreadReplyTarget_nullContext_noLiveRefs_returnsNull", () => {
  // No context and no live refs — bail.
  const result = resolveThreadReplyTarget(null, null, null);

  assert.strictEqual(result, null);
});
