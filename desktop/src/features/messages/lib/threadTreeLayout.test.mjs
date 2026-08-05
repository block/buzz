import assert from "node:assert/strict";
import test from "node:test";

import {
  getThreadDepthIndentRem,
  getThreadRailCenterRem,
  getThreadReplyAvatarCenterRem,
  getThreadReplyAvatarCenterYRem,
  getThreadReplyAvatarSizeRem,
  getThreadReplyConnectorLayout,
  getThreadReplyDescendantRailStartYRem,
  getThreadReplyIndentRem,
  THREAD_REPLY_ROW_PADDING_TOP_REM,
  threadReplyLength,
} from "./threadTreeLayout.ts";

test("getThreadReplyIndentRem uses a visible Tailwind spacing step", () => {
  assert.equal(getThreadReplyIndentRem(0, 1), 0);
  assert.equal(getThreadReplyIndentRem(1, 1), 0);
  assert.equal(getThreadReplyIndentRem(2, 1), 3);
  assert.equal(getThreadReplyIndentRem(3, 1), 6);
});

test("avatar center helpers expose the rail anchor points", () => {
  assert.equal(getThreadReplyAvatarCenterRem(0, 1), 2.25);
  assert.equal(getThreadReplyAvatarCenterRem(1, 1), 2.25);
  assert.equal(getThreadReplyAvatarCenterRem(2, 1), 5.25);
  assert.equal(getThreadReplyAvatarCenterYRem(1), 1.75);
  assert.equal(getThreadReplyDescendantRailStartYRem(1), 3.5);
});

test("getThreadReplyConnectorLayout stops before the child avatar edge", () => {
  assert.equal(getThreadReplyConnectorLayout(0, 1), null);
  assert.equal(getThreadReplyConnectorLayout(1, 1), null);
  assert.deepEqual(getThreadReplyConnectorLayout(2, 1), {
    childOffsetRem: 5.25,
    heightRem: 1.75,
    parentOffsetRem: 2.25,
    widthRem: 1.25,
  });
  assert.deepEqual(getThreadReplyConnectorLayout(3, 1), {
    childOffsetRem: 8.25,
    heightRem: 1.75,
    parentOffsetRem: 5.25,
    widthRem: 1.25,
  });
});

test("getThreadReplyConnectorLayout clamps very deep replies to the visible rail", () => {
  assert.deepEqual(getThreadReplyConnectorLayout(99, 1), {
    childOffsetRem: 20.25,
    heightRem: 1.75,
    parentOffsetRem: 17.25,
    widthRem: 1.25,
  });
});

test("threadReplyLength formats rem values for inline styles", () => {
  assert.equal(threadReplyLength(0), "0");
  assert.equal(threadReplyLength(1.75), "1.75rem");
  assert.equal(threadReplyLength(-0.125), "-0.125rem");
});

test("thread rail centers remain aligned at 500% avatar scale", () => {
  const avatarSize = 15;
  assert.equal(getThreadReplyAvatarSizeRem(5), avatarSize);
  assert.equal(
    getThreadRailCenterRem(5),
    THREAD_REPLY_ROW_PADDING_TOP_REM + avatarSize / 2,
  );
  assert.equal(getThreadDepthIndentRem(5), avatarSize);
});

for (const scale of [0.75, 1, 2, 5]) {
  test(`rail center equals avatar center Y at ${scale * 100}%`, () => {
    const avatarSize = getThreadReplyAvatarSizeRem(scale);
    assert.equal(
      getThreadReplyAvatarCenterYRem(scale),
      THREAD_REPLY_ROW_PADDING_TOP_REM + avatarSize / 2,
    );
    assert.equal(getThreadReplyIndentRem(2, scale), avatarSize);
  });
}
