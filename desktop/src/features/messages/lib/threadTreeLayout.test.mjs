import assert from "node:assert/strict";
import test from "node:test";

import {
  getThreadReplyAvatarCenterRem,
  getThreadReplyAvatarCenterYRem,
  getThreadReplyConnectorLayout,
  getThreadReplyDescendantRailStartYRem,
  getThreadReplyIndentRem,
  threadReplyLength,
} from "./threadTreeLayout.ts";

test("getThreadReplyIndentRem uses a visible Tailwind spacing step", () => {
  assert.equal(getThreadReplyIndentRem(0), 0);
  assert.equal(getThreadReplyIndentRem(1), 0);
  assert.equal(getThreadReplyIndentRem(2), 2.25);
  assert.equal(getThreadReplyIndentRem(3), 4.5);
});

test("avatar center helpers expose the rail anchor points", () => {
  assert.equal(getThreadReplyAvatarCenterRem(0), 1.875);
  assert.equal(getThreadReplyAvatarCenterRem(1), 1.875);
  assert.equal(getThreadReplyAvatarCenterRem(2), 4.125);
  assert.equal(getThreadReplyAvatarCenterYRem(), 1.375);
  assert.equal(getThreadReplyDescendantRailStartYRem(), 2.75);
});

test("getThreadReplyConnectorLayout stops before the child avatar edge", () => {
  assert.equal(getThreadReplyConnectorLayout(0), null);
  assert.equal(getThreadReplyConnectorLayout(1), null);
  assert.deepEqual(getThreadReplyConnectorLayout(2), {
    childOffsetRem: 4.125,
    heightRem: 1.375,
    parentOffsetRem: 1.875,
    widthRem: 0.875,
  });
  assert.deepEqual(getThreadReplyConnectorLayout(3), {
    childOffsetRem: 6.375,
    heightRem: 1.375,
    parentOffsetRem: 4.125,
    widthRem: 0.875,
  });
});

test("getThreadReplyConnectorLayout clamps very deep replies to the visible rail", () => {
  assert.deepEqual(getThreadReplyConnectorLayout(99), {
    childOffsetRem: 15.375,
    heightRem: 1.375,
    parentOffsetRem: 13.125,
    widthRem: 0.875,
  });
});

test("threadReplyLength formats rem values for inline styles", () => {
  assert.equal(threadReplyLength(0), "0");
  assert.equal(threadReplyLength(1.75), "1.75rem");
  assert.equal(threadReplyLength(-0.125), "-0.125rem");
});
