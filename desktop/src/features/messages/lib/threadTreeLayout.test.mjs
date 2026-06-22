import assert from "node:assert/strict";
import test from "node:test";

import {
  getThreadReplyAvatarCenterPx,
  getThreadReplyAvatarCenterYPx,
  getThreadReplyConnectorLayout,
  getThreadReplyDescendantRailStartYPx,
  getThreadReplyIndentPx,
} from "./threadTreeLayout.ts";

test("getThreadReplyIndentPx aligns child avatars to parent text columns", () => {
  assert.equal(getThreadReplyIndentPx(0), 0);
  assert.equal(getThreadReplyIndentPx(1), 50);
  assert.equal(getThreadReplyIndentPx(2), 100);
  assert.equal(getThreadReplyIndentPx(3), 150);
});

test("avatar center helpers expose the rail anchor points", () => {
  assert.equal(getThreadReplyAvatarCenterPx(0), 32);
  assert.equal(getThreadReplyAvatarCenterPx(1), 82);
  assert.equal(getThreadReplyAvatarCenterYPx(), 28);
  assert.equal(getThreadReplyDescendantRailStartYPx(), 52);
});

test("getThreadReplyConnectorLayout stops before the child avatar edge", () => {
  assert.equal(getThreadReplyConnectorLayout(0), null);
  assert.deepEqual(getThreadReplyConnectorLayout(1), {
    childOffsetPx: 82,
    heightPx: 28,
    parentOffsetPx: 32,
    widthPx: 26,
  });
  assert.deepEqual(getThreadReplyConnectorLayout(2), {
    childOffsetPx: 132,
    heightPx: 28,
    parentOffsetPx: 82,
    widthPx: 26,
  });
});

test("getThreadReplyConnectorLayout clamps very deep replies to the visible rail", () => {
  assert.deepEqual(getThreadReplyConnectorLayout(99), {
    childOffsetPx: 332,
    heightPx: 28,
    parentOffsetPx: 282,
    widthPx: 26,
  });
});
