import assert from "node:assert/strict";
import test from "node:test";

import { isMsgContextKey, msgContextKey } from "./readStateFormat.ts";

test("msgContextKey_prefixesId_returnsMsgKey", () => {
  assert.equal(msgContextKey("abc123"), "msg:abc123");
});

test("isMsgContextKey_wellFormedKey_returnsTrue", () => {
  assert.equal(isMsgContextKey("msg:abc123"), true);
});

test("isMsgContextKey_threadKey_returnsFalse", () => {
  assert.equal(isMsgContextKey(`thread:${"a".repeat(64)}`), false);
});

test("isMsgContextKey_channelKey_returnsFalse", () => {
  assert.equal(isMsgContextKey("channel-1"), false);
});

test("isMsgContextKey_emptyId_returnsFalse", () => {
  assert.equal(isMsgContextKey("msg:"), false);
});

test("isMsgContextKey_msgPrefixWrappingThreadKey_returnsFalse", () => {
  // A thread key accidentally re-prefixed must not pass as a message key.
  assert.equal(isMsgContextKey(`msg:thread:${"a".repeat(64)}`), false);
});

test("msgContextKey_output_roundTripsThroughValidator", () => {
  assert.equal(isMsgContextKey(msgContextKey("event-id")), true);
});
