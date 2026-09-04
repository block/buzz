import assert from "node:assert/strict";
import test from "node:test";

import {
  DURATION_SLIDER_STOPS,
  durationSliderIndex,
  formatDurationSeconds,
  formatDurationSecondsVerbose,
  parseDurationSeconds,
} from "./workflowDuration.ts";

test("parseDurationSeconds accepts compact and combined whole-second durations", () => {
  assert.equal(parseDurationSeconds("5s"), 5);
  assert.equal(parseDurationSeconds("90m"), 5_400);
  assert.equal(parseDurationSeconds("1h 2s"), 3_602);
  assert.equal(parseDurationSeconds("1H2M3S"), 3_723);
  assert.equal(parseDurationSeconds("2d"), 172_800);
  assert.equal(parseDurationSeconds("2w 3d 4h 5m 6s"), 1_483_506);
  assert.equal(parseDurationSeconds("42"), 42);
  assert.equal(parseDurationSeconds("0s"), 0);
});

test("parseDurationSeconds rejects empty, malformed, and fractional values", () => {
  assert.equal(parseDurationSeconds(""), null);
  assert.equal(parseDurationSeconds("1.5m"), null);
  assert.equal(parseDurationSeconds("1m later"), null);
});

test("formatDurationSeconds produces compact labels with significant units", () => {
  assert.equal(formatDurationSeconds(0), "0s");
  assert.equal(formatDurationSeconds(5), "5s");
  assert.equal(formatDurationSeconds(62), "1m 2s");
  assert.equal(formatDurationSeconds(3_602), "1h 2s");
  assert.equal(formatDurationSeconds(7_323), "2h 2m 3s");
  assert.equal(formatDurationSeconds(172_800), "2d");
  assert.equal(formatDurationSeconds(1_483_506), "2w 3d 4h 5m 6s");
});

test("formatDurationSecondsVerbose spells out units with correct plurals", () => {
  assert.equal(formatDurationSecondsVerbose(0), "0 seconds");
  assert.equal(formatDurationSecondsVerbose(300), "5 minutes");
  assert.equal(formatDurationSecondsVerbose(604_800), "1 week");
  assert.equal(
    formatDurationSecondsVerbose(1_483_506),
    "2 weeks 3 days 4 hours 5 minutes 6 seconds",
  );
});

test("duration slider starts at one second, keeps fine short-delay stops, and reaches three hours", () => {
  assert.deepEqual(DURATION_SLIDER_STOPS.slice(0, 3), [1, 2, 3]);
  assert.equal(DURATION_SLIDER_STOPS.at(-1), 10_800);
  assert.equal(DURATION_SLIDER_STOPS[durationSliderIndex(62)], 62);
  assert.equal(DURATION_SLIDER_STOPS[durationSliderIndex(300)], 300);
  assert.equal(DURATION_SLIDER_STOPS[durationSliderIndex(3_602)], 3_600);
});
