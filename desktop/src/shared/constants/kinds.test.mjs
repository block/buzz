import assert from "node:assert/strict";
import test from "node:test";

import {
  CHANNEL_MESSAGE_CONVERSATIONAL_KINDS,
  CHANNEL_MESSAGE_EVENT_KINDS,
  isConversationalUnreadKind,
  KIND_STREAM_MESSAGE,
  KIND_STREAM_MESSAGE_V2,
  KIND_STREAM_MESSAGE_DIFF,
  KIND_SYSTEM_MESSAGE,
  KIND_JOB_REQUEST,
  KIND_JOB_ACCEPTED,
  KIND_JOB_PROGRESS,
  KIND_JOB_RESULT,
  KIND_JOB_CANCEL,
  KIND_JOB_ERROR,
  KIND_HUDDLE_STARTED,
  KIND_HUDDLE_PARTICIPANT_JOINED,
  KIND_HUDDLE_PARTICIPANT_LEFT,
  KIND_HUDDLE_ENDED,
} from "./kinds.ts";

test("isConversationalUnreadKind_streamMessage_counts", () => {
  assert.equal(isConversationalUnreadKind(KIND_STREAM_MESSAGE), true);
});

test("isConversationalUnreadKind_streamMessageV2_counts", () => {
  // 40002 is a real message edit/v2 — must stay counted.
  assert.equal(isConversationalUnreadKind(KIND_STREAM_MESSAGE_V2), true);
});

test("isConversationalUnreadKind_streamMessageDiff_counts", () => {
  // 40008 is a real message diff — must stay counted.
  assert.equal(isConversationalUnreadKind(KIND_STREAM_MESSAGE_DIFF), true);
});

test("isConversationalUnreadKind_systemMessage_excluded", () => {
  // 40099 channel_created / member_joined rows must not inflate the pill.
  assert.equal(isConversationalUnreadKind(KIND_SYSTEM_MESSAGE), false);
});

test("isConversationalUnreadKind_allJobKinds_excluded", () => {
  for (const kind of [
    KIND_JOB_REQUEST,
    KIND_JOB_ACCEPTED,
    KIND_JOB_PROGRESS,
    KIND_JOB_RESULT,
    KIND_JOB_CANCEL,
    KIND_JOB_ERROR,
  ]) {
    assert.equal(isConversationalUnreadKind(kind), false, `kind ${kind}`);
  }
});

test("isConversationalUnreadKind_huddleLifecycle_excluded", () => {
  for (const kind of [
    KIND_HUDDLE_STARTED,
    KIND_HUDDLE_PARTICIPANT_JOINED,
    KIND_HUDDLE_PARTICIPANT_LEFT,
    KIND_HUDDLE_ENDED,
  ]) {
    assert.equal(isConversationalUnreadKind(kind), false, `kind ${kind}`);
  }
});

test("isConversationalUnreadKind_undefinedKind_countsAsConversational", () => {
  // Optimistic/pending rows whose kind has not populated must not be dropped.
  assert.equal(isConversationalUnreadKind(undefined), true);
});

test("isConversationalUnreadKind_unknownKind_countsAsConversational", () => {
  // An exclude-list, not an include-list: anything not explicitly excluded
  // (e.g. a future conversational kind) is kept.
  assert.equal(isConversationalUnreadKind(12345), true);
});

test("isConversationalUnreadKind_surface_counts", () => {
  // Surfaces are conversational content: they must trigger unread dots,
  // home-feed rows, and mention counts exactly like a kind-9 message.
  assert.equal(isConversationalUnreadKind(40110), true);
});

test("conversationalKinds_matchRustCONVERSATIONAL_KINDS", () => {
  // Parity with `CONVERSATIONAL_KINDS` in crates/buzz-core/src/kind.rs.
  // Every read path that treats kind:9 as "a message someone wrote" must treat
  // surfaces the same way — on both sides of the Rust/TS boundary.
  const RUST_CONVERSATIONAL_KINDS = [9, 40002, 40110];
  for (const kind of RUST_CONVERSATIONAL_KINDS) {
    assert.ok(
      CHANNEL_MESSAGE_EVENT_KINDS.includes(kind),
      `kind ${kind} is conversational in Rust but missing from CHANNEL_MESSAGE_EVENT_KINDS`,
    );
    // The exported Set is what readers actually branch on (Projects inline
    // chat, and anything else asking "is this a message?"), so assert it too —
    // otherwise a kind could silently drop out of it with tests still green.
    assert.ok(
      CHANNEL_MESSAGE_CONVERSATIONAL_KINDS.has(kind),
      `kind ${kind} missing from CHANNEL_MESSAGE_CONVERSATIONAL_KINDS`,
    );
    assert.equal(isConversationalUnreadKind(kind), true);
  }
  assert.equal(
    CHANNEL_MESSAGE_CONVERSATIONAL_KINDS.size,
    RUST_CONVERSATIONAL_KINDS.length,
    "the conversational set must mirror Rust's CONVERSATIONAL_KINDS exactly",
  );
});
