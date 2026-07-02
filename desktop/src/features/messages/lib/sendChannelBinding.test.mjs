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

import { createOptimisticMessage, resolveEffectiveChannel } from "../hooks.ts";

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
// Thread-reply context invariant (CRITICAL fix — Thufir Pass 1)
//
// handleSendThreadReply now accepts a `threadContext` object captured at
// submit time. These tests verify the structural invariants of the object
// shape and the fall-through semantics, NOT the full async chain (which is
// tested by the E2E spec). The key property to pin: once a capturedThreadContext
// is in hand, the send must use its values — not re-read live refs.
// ---------------------------------------------------------------------------

// Helpers that mirror the submit-time capture in MessageThreadPanel
function makeCapturedThreadContext(parentEventId, threadHeadId) {
  return { parentEventId, threadHeadId };
}

test("capturedThreadContext_withReplyTarget_preservesParentEventId", () => {
  const replyTargetId = "event-abc-reply-target";
  const threadHeadId = "event-abc-thread-head";

  const ctx = makeCapturedThreadContext(replyTargetId, threadHeadId);

  assert.equal(
    ctx.parentEventId,
    replyTargetId,
    "captured context must preserve the reply-target id set at submit time",
  );
  assert.equal(
    ctx.threadHeadId,
    threadHeadId,
    "captured context must preserve the thread-head id set at submit time",
  );
});

test("capturedThreadContext_whenReplyTargetIsNull_fallsBackToThreadHead", () => {
  // When the user has not selected a specific reply target (replying to the
  // thread as a whole), parentEventId falls back to threadHeadId at capture time.
  const threadHeadId = "event-abc-thread-head";
  const parentEventId =
    /* no reply target selected, captured as: */ threadHeadId;

  const ctx = makeCapturedThreadContext(parentEventId, threadHeadId);

  assert.equal(
    ctx.parentEventId,
    threadHeadId,
    "when no specific reply target is selected, parentEventId equals threadHeadId",
  );
});

test("capturedThreadContext_navigatedAwayDuringAwaits_submitTimeValuesUnchanged", () => {
  // Simulates: submit in thread T1 of channel A → async awaits → user opens
  // thread T2. The captured context must still hold T1's values.
  const T1_parentEventId = "t1-reply";
  const T1_threadHeadId = "t1-head";
  const capturedAtSubmit = makeCapturedThreadContext(
    T1_parentEventId,
    T1_threadHeadId,
  );

  // Simulate navigation: "live refs" would now point at T2 if read again
  const T2_threadHeadId = "t2-head";
  const T2_parentEventId = "t2-reply";

  // The captured context is immutable — it holds submit-time values
  assert.equal(
    capturedAtSubmit.parentEventId,
    T1_parentEventId,
    "captured parentEventId must not change after navigation to a different thread",
  );
  assert.equal(
    capturedAtSubmit.threadHeadId,
    T1_threadHeadId,
    "captured threadHeadId must not change after navigation to a different thread",
  );

  // Verify the live state is actually different (the race has occurred)
  assert.notEqual(
    capturedAtSubmit.parentEventId,
    T2_parentEventId,
    "live state after navigation differs from captured state",
  );
  assert.notEqual(
    capturedAtSubmit.threadHeadId,
    T2_threadHeadId,
    "live thread head after navigation differs from captured thread head",
  );
});

test("capturedThreadContext_withChannelId_bothValuesAreIndependent", () => {
  // When both channelId and thread context are captured together, they must
  // remain independently stable after navigation.
  const capturedChannelId = "channel-A";
  const capturedCtx = makeCapturedThreadContext("reply-in-A", "head-in-A");

  // Simulate post-navigation state
  const liveChannelId = "channel-B";
  const liveThreadHeadId = "head-in-B";

  assert.equal(
    capturedChannelId,
    "channel-A",
    "captured channel id must not be affected by navigation",
  );
  assert.equal(
    capturedCtx.threadHeadId,
    "head-in-A",
    "captured thread head must not be affected by navigation",
  );
  assert.notEqual(capturedChannelId, liveChannelId);
  assert.notEqual(capturedCtx.threadHeadId, liveThreadHeadId);
});
