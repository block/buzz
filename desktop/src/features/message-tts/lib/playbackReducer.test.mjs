import assert from "node:assert/strict";
import test from "node:test";

import {
  applyPlaybackEvent,
  IDLE_STATE,
  isMessagePlaying,
} from "./playbackReducer.mjs";

function ev(messageId, status, extra = {}) {
  return {
    message_id: messageId,
    status,
    truncated: false,
    error: null,
    ...extra,
  };
}

test("preparing makes the message current", () => {
  const { state } = applyPlaybackEvent(IDLE_STATE, ev("a", "preparing"));
  assert.equal(state.messageId, "a");
  assert.equal(state.phase, "preparing");
  assert.equal(isMessagePlaying(state, "a"), true);
  assert.equal(isMessagePlaying(state, "b"), false);
});

test("playing keeps the message current and flips the phase", () => {
  let { state } = applyPlaybackEvent(IDLE_STATE, ev("a", "preparing"));
  ({ state } = applyPlaybackEvent(state, ev("a", "playing")));
  assert.equal(state.phase, "playing");
  assert.equal(isMessagePlaying(state, "a"), true);
});

test("completed for the current message returns to idle", () => {
  let { state } = applyPlaybackEvent(IDLE_STATE, ev("a", "playing"));
  ({ state } = applyPlaybackEvent(state, ev("a", "completed")));
  assert.deepEqual(state, IDLE_STATE);
});

test("stopped for the current message returns to idle", () => {
  let { state } = applyPlaybackEvent(IDLE_STATE, ev("a", "playing"));
  ({ state } = applyPlaybackEvent(state, ev("a", "stopped")));
  assert.deepEqual(state, IDLE_STATE);
});

test("supersede race: newer playback is never cleared by the old one's terminal event", () => {
  // play a → play b arrives (preparing b + superseded a can arrive in either
  // order) → a's late "superseded"/"completed" must not clear b.
  let { state } = applyPlaybackEvent(IDLE_STATE, ev("a", "playing"));
  ({ state } = applyPlaybackEvent(state, ev("b", "preparing")));
  assert.equal(state.messageId, "b");

  ({ state } = applyPlaybackEvent(state, ev("a", "superseded")));
  assert.equal(state.messageId, "b", "stale superseded must not clear b");

  ({ state } = applyPlaybackEvent(state, ev("a", "completed")));
  assert.equal(state.messageId, "b", "stale completed must not clear b");

  assert.equal(isMessagePlaying(state, "b"), true);
  assert.equal(isMessagePlaying(state, "a"), false);
});

test("superseded arriving before the new preparing also converges", () => {
  let { state } = applyPlaybackEvent(IDLE_STATE, ev("a", "playing"));
  ({ state } = applyPlaybackEvent(state, ev("a", "superseded")));
  assert.deepEqual(state, IDLE_STATE);
  ({ state } = applyPlaybackEvent(state, ev("b", "preparing")));
  assert.equal(state.messageId, "b");
});

test("failed for the current message carries the error and stops the playing affordance", () => {
  let { state } = applyPlaybackEvent(IDLE_STATE, ev("a", "preparing"));
  const result = applyPlaybackEvent(
    state,
    ev("a", "failed", { error: "voice model is not downloaded yet" }),
  );
  state = result.state;
  assert.equal(state.messageId, "a");
  assert.equal(state.phase, "idle");
  assert.equal(state.error, "voice model is not downloaded yet");
  assert.equal(result.detachedError, "voice model is not downloaded yet");
  assert.equal(isMessagePlaying(state, "a"), false);
});

test("failed for a non-current message leaves current playback untouched", () => {
  const { state } = applyPlaybackEvent(IDLE_STATE, ev("b", "playing"));
  const result = applyPlaybackEvent(
    state,
    ev("a", "failed", { error: "boom" }),
  );
  assert.equal(result.state.messageId, "b");
  assert.equal(result.state.phase, "playing");
  assert.equal(result.detachedError, "boom");
});

test("preflight failure from idle surfaces as a detached error", () => {
  const result = applyPlaybackEvent(
    IDLE_STATE,
    ev("a", "failed", { error: "message has no speakable text" }),
  );
  assert.equal(result.detachedError, "message has no speakable text");
  assert.equal(isMessagePlaying(result.state, "a"), false);
});

test("failed without error text falls back to a generic message", () => {
  const result = applyPlaybackEvent(IDLE_STATE, ev("a", "failed"));
  assert.equal(result.detachedError, "playback failed");
});

test("truncated flag is carried on preparing/playing", () => {
  const { state } = applyPlaybackEvent(
    IDLE_STATE,
    ev("a", "preparing", { truncated: true }),
  );
  assert.equal(state.truncated, true);
});

test("unknown status is ignored", () => {
  const before = applyPlaybackEvent(IDLE_STATE, ev("a", "playing")).state;
  const { state } = applyPlaybackEvent(before, ev("a", "warming-up"));
  assert.equal(state, before);
});

test("rapid play/stop/play across messages always tracks the latest", () => {
  let state = IDLE_STATE;
  for (let i = 0; i < 20; i++) {
    ({ state } = applyPlaybackEvent(state, ev(`m${i}`, "preparing")));
    // Stale events from the previous message trickle in afterwards.
    ({ state } = applyPlaybackEvent(state, ev(`m${i - 1}`, "superseded")));
    ({ state } = applyPlaybackEvent(state, ev(`m${i - 1}`, "completed")));
    assert.equal(state.messageId, `m${i}`);
  }
});
