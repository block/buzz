import assert from "node:assert/strict";
import test from "node:test";

import {
  TIME_PRESETS,
  parseCustomDateTime,
  todayDateString,
} from "./timePresets.ts";

const nowSeconds = () => Math.floor(Date.now() / 1_000);

test("TIME_PRESETS_every_preset_returns_strictly_future_timestamp", () => {
  const now = nowSeconds();
  for (const preset of TIME_PRESETS) {
    assert.ok(
      preset.getTimestamp() > now,
      `${preset.label} must be strictly in the future`,
    );
  }
});

test("TIME_PRESETS_relative_offsets_match_their_labels", () => {
  const before = nowSeconds();
  const byLabel = Object.fromEntries(
    TIME_PRESETS.map((p) => [p.label, p.getTimestamp()]),
  );
  // Allow a 2s window for clock drift across the getTimestamp calls.
  assert.ok(Math.abs(byLabel["In 30 minutes"] - (before + 30 * 60)) <= 2);
  assert.ok(Math.abs(byLabel["In 1 hour"] - (before + 60 * 60)) <= 2);
  assert.ok(Math.abs(byLabel["In 3 hours"] - (before + 3 * 60 * 60)) <= 2);
});

test("TIME_PRESETS_9am_presets_land_on_a_9am_boundary", () => {
  for (const label of ["Tomorrow at 9am", "Next Monday at 9am"]) {
    const preset = TIME_PRESETS.find((p) => p.label === label);
    const d = new Date(preset.getTimestamp() * 1_000);
    assert.equal(d.getHours(), 9);
    assert.equal(d.getMinutes(), 0);
  }
});

test("TIME_PRESETS_next_monday_lands_on_a_monday", () => {
  const preset = TIME_PRESETS.find((p) => p.label === "Next Monday at 9am");
  const d = new Date(preset.getTimestamp() * 1_000);
  assert.equal(d.getDay(), 1); // Monday
});

test("parseCustomDateTime_future_datetime_returns_timestamp", () => {
  const future = new Date(Date.now() + 24 * 60 * 60 * 1_000);
  const date = `${future.getFullYear()}-${String(future.getMonth() + 1).padStart(2, "0")}-${String(future.getDate()).padStart(2, "0")}`;
  const result = parseCustomDateTime(date, "14:30");
  assert.ok(result !== null);
  assert.ok(result > nowSeconds());
});

test("parseCustomDateTime_past_datetime_returns_null", () => {
  // One year ago — unambiguously past regardless of run time.
  const past = new Date(Date.now() - 365 * 24 * 60 * 60 * 1_000);
  const date = `${past.getFullYear()}-${String(past.getMonth() + 1).padStart(2, "0")}-${String(past.getDate()).padStart(2, "0")}`;
  assert.equal(parseCustomDateTime(date, "09:00"), null);
});

test("parseCustomDateTime_empty_inputs_return_null", () => {
  assert.equal(parseCustomDateTime("", "09:00"), null);
  assert.equal(parseCustomDateTime("2099-01-01", ""), null);
  assert.equal(parseCustomDateTime("", ""), null);
});

test("parseCustomDateTime_malformed_inputs_return_null", () => {
  assert.equal(parseCustomDateTime("not-a-date", "09:00"), null);
  assert.equal(parseCustomDateTime("2099-01-01", "99:99"), null);
});

test("todayDateString_returns_today_in_YYYY_MM_DD_local", () => {
  const now = new Date();
  const expected = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")}`;
  assert.equal(todayDateString(), expected);
});
