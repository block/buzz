import assert from "node:assert/strict";
import test from "node:test";

import {
  isMsgContextKey,
  isThreadContextKey,
  msgContextKey,
} from "./readStateFormat.ts";

const EVENT_ID = "a".repeat(64);

test("msgContextKey_prefixesId_returnsMsgKey", () => {
  assert.equal(msgContextKey(EVENT_ID), `msg:${EVENT_ID}`);
});

test("isMsgContextKey_wellFormedKey_returnsTrue", () => {
  assert.equal(isMsgContextKey(`msg:${EVENT_ID}`), true);
});

test("isThreadContextKey_wellFormedKey_returnsTrue", () => {
  assert.equal(isThreadContextKey(`thread:${EVENT_ID}`), true);
});

test("isMsgContextKey_threadKey_returnsFalse", () => {
  assert.equal(isMsgContextKey(`thread:${EVENT_ID}`), false);
});

test("isMsgContextKey_channelKey_returnsFalse", () => {
  assert.equal(isMsgContextKey("channel-1"), false);
});

test("isMsgContextKey_emptyId_returnsFalse", () => {
  assert.equal(isMsgContextKey("msg:"), false);
});

test("isMsgContextKey_shortId_returnsFalse", () => {
  assert.equal(isMsgContextKey("msg:abc123"), false);
});

test("isMsgContextKey_msgPrefixWrappingThreadKey_returnsFalse", () => {
  // A thread key accidentally re-prefixed must not pass as a message key.
  assert.equal(isMsgContextKey(`msg:thread:${EVENT_ID}`), false);
});

test("isThreadContextKey_shortId_returnsFalse", () => {
  assert.equal(isThreadContextKey("thread:abc123"), false);
});

test("msgContextKey_output_roundTripsThroughValidator", () => {
  assert.equal(isMsgContextKey(msgContextKey(EVENT_ID)), true);
});
