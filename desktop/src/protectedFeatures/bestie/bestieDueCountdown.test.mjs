import assert from "node:assert/strict";
import test from "node:test";

import {
  bestieDueCountdownTickMs,
  formatBestieDueCountdown,
} from "./bestieDueCountdown.ts";

test("formatBestieDueCountdown hides beyond horizon", () => {
  assert.equal(formatBestieDueCountdown(10_000, 0, 3600), null);
  assert.equal(formatBestieDueCountdown(3600, 0, 3600), "60m");
  assert.equal(formatBestieDueCountdown(3599, 0, 3600), "60m");
});

test("formatBestieDueCountdown uses minutes then seconds", () => {
  assert.equal(formatBestieDueCountdown(7 * 60, 0, 3600), "7m");
  assert.equal(formatBestieDueCountdown(90, 0, 3600), "2m");
  assert.equal(formatBestieDueCountdown(45, 0, 3600), "45s");
  assert.equal(formatBestieDueCountdown(1, 0, 3600), "1s");
  assert.equal(formatBestieDueCountdown(0, 0, 3600), "due");
  assert.equal(formatBestieDueCountdown(-5, 0, 3600), "due");
});

test("formatBestieDueCountdown off when horizon is 0", () => {
  assert.equal(formatBestieDueCountdown(30, 0, 0), null);
});

test("bestieDueCountdownTickMs cadence", () => {
  assert.equal(bestieDueCountdownTickMs(30, 0, 3600), 1000);
  assert.equal(bestieDueCountdownTickMs(120, 0, 3600), 15_000);
  assert.equal(bestieDueCountdownTickMs(0, 0, 3600), null);
  const beyond = bestieDueCountdownTickMs(10_000, 0, 3600);
  assert.ok(beyond != null && beyond <= 60_000);
});
