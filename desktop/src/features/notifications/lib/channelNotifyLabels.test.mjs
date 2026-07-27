import assert from "node:assert/strict";
import test from "node:test";

import {
  CHANNEL_MUTE_PRESETS,
  CHANNEL_NOTIFY_LEVEL_OPTIONS,
  channelNotifyHeaderSuffix,
  formatMuteUntil,
} from "./channelNotifyLabels.ts";
import { DEFAULT_CHANNEL_NOTIFY_STATE } from "./resolveChannelNotifyState.ts";

test("the level options cover every level exactly once", () => {
  assert.deepEqual(
    CHANNEL_NOTIFY_LEVEL_OPTIONS.map((option) => option.value),
    ["all", "mentions", "mute"],
  );
});

test("header suffix is null at the default level", () => {
  assert.equal(channelNotifyHeaderSuffix(DEFAULT_CHANNEL_NOTIFY_STATE), null);
});

test("header suffix names the non-default levels", () => {
  assert.equal(
    channelNotifyHeaderSuffix({
      ...DEFAULT_CHANNEL_NOTIFY_STATE,
      level: "mentions",
    }),
    "Notifications: Just mentions",
  );
  assert.equal(
    channelNotifyHeaderSuffix({
      ...DEFAULT_CHANNEL_NOTIFY_STATE,
      level: "mute",
    }),
    "Notifications: Muted",
  );
});

test("mute presets return future timestamps in seconds", () => {
  const now = Math.floor(Date.now() / 1_000);
  for (const preset of CHANNEL_MUTE_PRESETS) {
    assert.ok(preset.getTimestamp() > now, preset.label);
  }
  assert.ok(CHANNEL_MUTE_PRESETS[0].getTimestamp() - now <= 3_600);
});

test("formatMuteUntil omits the weekday for a same-day expiry", () => {
  const now = new Date(2026, 0, 5, 8, 0, 0);
  const until = new Date(2026, 0, 5, 9, 4, 0);
  const formatted = formatMuteUntil(Math.floor(until.getTime() / 1_000), now);
  assert.equal(
    formatted,
    until.toLocaleTimeString(undefined, {
      hour: "numeric",
      minute: "2-digit",
    }),
  );
});

test("formatMuteUntil includes the weekday once the expiry rolls over", () => {
  const now = new Date(2026, 0, 5, 20, 0, 0);
  const until = new Date(2026, 0, 6, 9, 0, 0);
  const formatted = formatMuteUntil(Math.floor(until.getTime() / 1_000), now);
  assert.ok(
    formatted.startsWith(
      until.toLocaleDateString(undefined, { weekday: "short" }),
    ),
    formatted,
  );
});
