import assert from "node:assert/strict";
import test from "node:test";

import { APPEARANCE_SCALE_PRESETS } from "./appearanceScalePresets.ts";
import {
  CHAT_SCALE_PRESETS,
  DEFAULT_CHAT_SCALE,
  MAX_CHAT_SCALE,
  MIN_CHAT_SCALE,
  formatChatScalePercent,
  normalizeChatScale,
} from "./chatScale.ts";

test("chat scale shares the 75%-500% Appearance ladder", () => {
  assert.deepEqual([...CHAT_SCALE_PRESETS], [...APPEARANCE_SCALE_PRESETS]);
  assert.equal(MIN_CHAT_SCALE, 0.75);
  assert.equal(MAX_CHAT_SCALE, 5);
  assert.equal(DEFAULT_CHAT_SCALE, 1);
  assert.equal(formatChatScalePercent(5), "500%");
});

test("chat scale snaps to nearest preset", () => {
  assert.equal(normalizeChatScale(1.2), 1.25);
  assert.equal(normalizeChatScale(9), 5);
  assert.equal(normalizeChatScale(0), 0.75);
});
