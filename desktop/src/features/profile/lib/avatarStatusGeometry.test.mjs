import assert from "node:assert/strict";
import test from "node:test";

import {
  MAX_STATUS_CUTOUT_PX,
  MAX_STATUS_DOT_PX,
  MIN_STATUS_DOT_PX,
  PROFILE_HERO_STATUS_RATIOS,
  resolveAvatarStatusGeometry,
} from "./avatarStatusGeometry.ts";

test("small avatars keep a readable minimum status pip", () => {
  const result = resolveAvatarStatusGeometry(24, {
    centerX: 0.85,
    centerY: 0.85,
    cutoutDiameter: 0.4,
    dotDiameter: 0.25,
  });
  assert.equal(result.badgeBox.height, MIN_STATUS_DOT_PX);
  assert.equal(result.badgeBox.width, MIN_STATUS_DOT_PX);
  assert.ok(result.cutout.r * 2 >= result.badgeBox.height);
});

test("large heroes clamp status so the pip does not dominate the avatar", () => {
  const result = resolveAvatarStatusGeometry(96, PROFILE_HERO_STATUS_RATIOS);
  assert.equal(result.badgeBox.height, MAX_STATUS_DOT_PX);
  assert.equal(result.badgeBox.width, MAX_STATUS_DOT_PX);
  assert.ok(result.cutout.r * 2 <= MAX_STATUS_CUTOUT_PX);
  // Badge stays on the lower-right rim (non-negative insets, not centered).
  assert.ok(result.badgeBox.bottom >= -0.5);
  assert.ok(result.badgeBox.right >= -0.5);
  assert.ok(result.badgeBox.bottom < 96 * 0.25);
});

test("500% profile status geometry stays clamped and inside the avatar", () => {
  const result = resolveAvatarStatusGeometry(480, {
    centerX: 0.85,
    centerY: 0.85,
    cutoutDiameter: 0.375,
    dotDiameter: 0.3,
  });
  assert.equal(result.badgeBox.height, MAX_STATUS_DOT_PX);
  assert.equal(result.badgeBox.width, MAX_STATUS_DOT_PX);
  assert.ok(result.cutout.cx <= 480);
  assert.ok(result.cutout.cy <= 480);
  assert.ok(result.cutout.r * 2 <= MAX_STATUS_CUTOUT_PX);
});

test("hero ratio preset at 500% stays a small pip", () => {
  const result = resolveAvatarStatusGeometry(480, PROFILE_HERO_STATUS_RATIOS);
  assert.equal(result.badgeBox.height, result.badgeBox.width);
  assert.equal(result.badgeBox.height, MAX_STATUS_DOT_PX);
});
