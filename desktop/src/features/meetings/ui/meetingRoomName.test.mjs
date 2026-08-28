import assert from "node:assert/strict";
import test from "node:test";

import {
  MEETING_ROOM_NAME_MAX,
  normalizeMeetingRoomName,
  validateMeetingRoomName,
} from "./meetingRoomName.ts";

test("normalize folds whitespace, lowercases, and trims separators", () => {
  assert.equal(normalizeMeetingRoomName("  Weekly  Sync  "), "weekly-sync");
  assert.equal(normalizeMeetingRoomName("--Room__"), "room");
  assert.equal(normalizeMeetingRoomName("a  b  c"), "a-b-c");
});

test("normalize drops disallowed punctuation but keeps - and _", () => {
  assert.equal(normalizeMeetingRoomName("stand-up! (team)"), "stand-up-team");
  assert.equal(normalizeMeetingRoomName("road_map/2026"), "road_map2026");
});

test("normalize keeps non-ASCII letters and digits", () => {
  assert.equal(normalizeMeetingRoomName("café-회의-2"), "café-회의-2");
});

test("validate rejects empty and too-short names with a reason", () => {
  assert.equal(validateMeetingRoomName("   ").ok, false);
  assert.equal(validateMeetingRoomName("!!!").ok, false);
  assert.equal(validateMeetingRoomName("ab").ok, false);
});

test("validate rejects names over the max length", () => {
  const long = "x".repeat(MEETING_ROOM_NAME_MAX + 1);
  const result = validateMeetingRoomName(long);
  assert.equal(result.ok, false);
});

test("validate returns the normalized value on success", () => {
  assert.deepEqual(validateMeetingRoomName("  Design Review  "), {
    ok: true,
    value: "design-review",
  });
});
