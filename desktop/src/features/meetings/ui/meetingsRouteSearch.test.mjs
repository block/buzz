import assert from "node:assert/strict";
import test from "node:test";

import {
  sanitizeRoomToCreate,
  sanitizeRoomToJoin,
  validateMeetingsSearch,
} from "./meetingsRouteSearch.ts";

// Live `rooms-by-pubkey` entry: created on the HiveTalk dashboard, so its name
// breaks the URL-safe rule that only binds at registration.
const DASHBOARD_ROOM = "Celestial  Solace";

test("join passes a dashboard room name through untouched", () => {
  assert.equal(sanitizeRoomToJoin(DASHBOARD_ROOM), DASHBOARD_ROOM);
  assert.deepEqual(
    validateMeetingsSearch({ action: "join", room: DASHBOARD_ROOM }),
    { action: "join", room: DASHBOARD_ROOM },
  );
});

test("join trims surrounding whitespace but keeps internal spacing", () => {
  assert.equal(sanitizeRoomToJoin(`  ${DASHBOARD_ROOM}  `), DASHBOARD_ROOM);
});

test("join rejects empty and control-character names", () => {
  assert.equal(sanitizeRoomToJoin("   "), undefined);
  for (const code of [0x00, 0x1f, 0x7f]) {
    const withControl = `bad${String.fromCharCode(code)}name`;
    assert.equal(sanitizeRoomToJoin(withControl), undefined);
  }
});

test("create still normalizes to a registrable name", () => {
  assert.equal(sanitizeRoomToCreate(DASHBOARD_ROOM), "celestial-solace");
  assert.deepEqual(
    validateMeetingsSearch({ action: "start", room: "Team Standup!" }),
    { action: "start", room: "team-standup" },
  );
});

test("create drops names that cannot survive normalization", () => {
  assert.equal(sanitizeRoomToCreate("!!"), undefined);
  assert.equal(sanitizeRoomToCreate("ab"), undefined);
});

test("an unknown action falls back to the create sanitizer", () => {
  assert.deepEqual(
    validateMeetingsSearch({ action: "nope", room: "My Room" }),
    {
      action: undefined,
      room: "my-room",
    },
  );
});

test("a missing or non-string room yields undefined, not a crash", () => {
  assert.deepEqual(validateMeetingsSearch({}), {
    action: undefined,
    room: undefined,
  });
  assert.deepEqual(validateMeetingsSearch({ action: "join", room: 7 }), {
    action: "join",
    room: undefined,
  });
});
