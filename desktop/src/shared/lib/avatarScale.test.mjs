import assert from "node:assert/strict";
import test from "node:test";

import { APPEARANCE_SCALE_PRESETS } from "./appearanceScalePresets.ts";
import {
  AVATAR_BASE_SIZE_REM,
  AVATAR_SCALE_PRESETS,
  BASE_MESSAGE_AVATAR_SIZE_REM,
  DEFAULT_AVATAR_SCALE,
  MAX_AVATAR_SCALE,
  MIN_AVATAR_SCALE,
  avatarScalePresetIndex,
  formatAvatarScalePercent,
  getAvatarSizeRem,
  getMessageAvatarSizeRem,
  normalizeAvatarScale,
} from "./avatarScale.ts";
import { MAX_CHAT_SCALE, CHAT_SCALE_PRESETS } from "./chatScale.ts";
import { MAX_TEXT_SCALE, TEXT_SCALE_PRESETS } from "./textScale.ts";

test("all Appearance scales share the 75%-500% preset ladder", () => {
  assert.deepEqual([...TEXT_SCALE_PRESETS], [...APPEARANCE_SCALE_PRESETS]);
  assert.deepEqual([...CHAT_SCALE_PRESETS], [...APPEARANCE_SCALE_PRESETS]);
  assert.deepEqual([...AVATAR_SCALE_PRESETS], [...APPEARANCE_SCALE_PRESETS]);
  assert.equal(MAX_TEXT_SCALE, 5);
  assert.equal(MAX_CHAT_SCALE, 5);
  assert.equal(MAX_AVATAR_SCALE, 5);
});

test("message avatar base size is 48px (3rem) at default scale", () => {
  assert.equal(BASE_MESSAGE_AVATAR_SIZE_REM, 3);
  assert.equal(AVATAR_BASE_SIZE_REM.md, 3);
  assert.equal(DEFAULT_AVATAR_SCALE, 1);
  assert.equal(getMessageAvatarSizeRem(1), 3);
  assert.equal(getMessageAvatarSizeRem(DEFAULT_AVATAR_SCALE), 3);
});

test("avatar scale presets extend to 5x", () => {
  assert.equal(MIN_AVATAR_SCALE, 0.75);
  assert.equal(MAX_AVATAR_SCALE, 5);
  assert.ok(AVATAR_SCALE_PRESETS.includes(5));
  assert.equal(getMessageAvatarSizeRem(5), 15);
  assert.equal(formatAvatarScalePercent(5), "500%");
});

test("avatar scale clamps out-of-range requests to presets", () => {
  assert.equal(normalizeAvatarScale(9), MAX_AVATAR_SCALE);
  assert.equal(normalizeAvatarScale(0), MIN_AVATAR_SCALE);
  assert.equal(getMessageAvatarSizeRem(9), getMessageAvatarSizeRem(5));
});

test("avatar scale preset index maps stored values onto the ramp", () => {
  assert.equal(avatarScalePresetIndex(1), AVATAR_SCALE_PRESETS.indexOf(1));
  assert.equal(
    avatarScalePresetIndex(1.75),
    AVATAR_SCALE_PRESETS.indexOf(1.75),
  );
  assert.equal(avatarScalePresetIndex(1.4), AVATAR_SCALE_PRESETS.indexOf(1.5));
});

for (const [scale, expected] of [
  [0.75, { xs: 0.9375, sm: 1.125, md: 2.25 }],
  [1, { xs: 1.25, sm: 1.5, md: 3 }],
  [2, { xs: 2.5, sm: 3, md: 6 }],
  [5, { xs: 6.25, sm: 7.5, md: 15 }],
]) {
  test(`semantic avatar metrics resolve at ${scale * 100}%`, () => {
    assert.equal(getAvatarSizeRem("xs", scale), expected.xs);
    assert.equal(getAvatarSizeRem("sm", scale), expected.sm);
    assert.equal(getAvatarSizeRem("md", scale), expected.md);
  });
}
