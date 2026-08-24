import assert from "node:assert/strict";
import test from "node:test";

import {
  clearTypingStateForCompletion,
  recordTypingCompletion,
} from "./useChannelTyping.ts";

const TYPING_KEY = "agent:thread";
const NOW = 2_000_000_000_000;
const NOW_SECONDS = NOW / 1_000;

function makeState() {
  return {
    latestMessageCreatedAtByPubkey: {},
    suppressUntilByPubkey: {},
  };
}

function record(state, createdAt, { now = NOW } = {}) {
  return recordTypingCompletion({
    createdAt,
    latestMessageCreatedAtByPubkey: state.latestMessageCreatedAtByPubkey,
    now,
    suppressUntilByPubkey: state.suppressUntilByPubkey,
    typingKey: TYPING_KEY,
  });
}

test("duplicate full-array replay does not reprocess completion events", () => {
  const state = makeState();

  assert.equal(record(state, NOW_SECONDS), true);
  assert.equal(record(state, NOW_SECONDS, { now: NOW + 1_000 }), false);
  assert.deepEqual(state.latestMessageCreatedAtByPubkey, {
    [TYPING_KEY]: { createdAt: NOW_SECONDS, observedAt: NOW },
  });
  assert.deepEqual(state.suppressUntilByPubkey, {
    [TYPING_KEY]: NOW + 2_000,
  });
});

test("newer-then-older completion arrival keeps the newer watermark", () => {
  const state = makeState();

  assert.equal(record(state, NOW_SECONDS), true);
  assert.equal(record(state, NOW_SECONDS - 1), false);
  assert.equal(
    state.latestMessageCreatedAtByPubkey[TYPING_KEY].createdAt,
    NOW_SECONDS,
  );
  assert.equal(state.suppressUntilByPubkey[TYPING_KEY], NOW + 2_000);
});

test("older-then-newer completion arrival advances the watermark", () => {
  const state = makeState();

  assert.equal(record(state, NOW_SECONDS - 1), true);
  assert.equal(record(state, NOW_SECONDS), true);
  assert.equal(
    state.latestMessageCreatedAtByPubkey[TYPING_KEY].createdAt,
    NOW_SECONDS,
  );
  assert.equal(state.suppressUntilByPubkey[TYPING_KEY], NOW + 2_000);
});

test("historical first observation arms one bounded suppression for trailing ticks", () => {
  const state = makeState();
  const historicalCreatedAt = NOW_SECONDS - 60;

  assert.equal(record(state, historicalCreatedAt), true);
  assert.equal(record(state, historicalCreatedAt, { now: NOW + 1_000 }), false);
  assert.deepEqual(state.latestMessageCreatedAtByPubkey, {
    [TYPING_KEY]: { createdAt: historicalCreatedAt, observedAt: NOW },
  });
  assert.deepEqual(state.suppressUntilByPubkey, {
    [TYPING_KEY]: NOW + 2_000,
  });
});

test("lagging agent completion clears typing for a matching agent-clock event", () => {
  const state = makeState();
  const laggingCreatedAt = NOW_SECONDS - 60;

  assert.equal(record(state, laggingCreatedAt), true);
  assert.deepEqual(state.suppressUntilByPubkey, {
    [TYPING_KEY]: NOW + 2_000,
  });
});

test("ahead agent completion keeps suppression desktop-local", () => {
  const state = makeState();
  const aheadCreatedAt = NOW_SECONDS + 60;

  assert.equal(record(state, aheadCreatedAt), true);
  assert.deepEqual(state.suppressUntilByPubkey, {
    [TYPING_KEY]: NOW + 2_000,
  });
});

test("queued newer typing survives an older completion clear", () => {
  const typingStateAfterQueuedTick = {
    [TYPING_KEY]: {
      createdAt: NOW_SECONDS + 1,
      expiresAt: NOW + 8_000,
      firstSeenAt: NOW,
      pubkey: "agent",
      threadHeadId: "thread",
    },
  };

  assert.equal(
    clearTypingStateForCompletion(
      typingStateAfterQueuedTick,
      TYPING_KEY,
      NOW_SECONDS,
      NOW,
    ),
    typingStateAfterQueuedTick,
  );
});

test("completion clears matching or older typing state", () => {
  const typingState = {
    [TYPING_KEY]: {
      createdAt: NOW_SECONDS,
      expiresAt: NOW + 8_000,
      firstSeenAt: NOW,
      pubkey: "agent",
      threadHeadId: "thread",
    },
  };

  assert.deepEqual(
    clearTypingStateForCompletion(typingState, TYPING_KEY, NOW_SECONDS, NOW),
    {},
  );
});

test("watermarks expire by desktop observation time, not agent clock", () => {
  const state = makeState();
  record(state, NOW_SECONDS + 60);

  assert.equal(record(state, NOW_SECONDS + 61, { now: NOW + 8_000 }), true);
  assert.deepEqual(state.latestMessageCreatedAtByPubkey, {
    [TYPING_KEY]: { createdAt: NOW_SECONDS + 61, observedAt: NOW + 8_000 },
  });
});
