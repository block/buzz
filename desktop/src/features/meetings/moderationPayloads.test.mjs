/**
 * Contract: the bodies Buzz puts on the wire for HiveTalk's moderation
 * endpoints. The field names are not ours to choose — they are pinned to
 * HiveTalk's `openapi.yaml` (`ParticipantAction`, `RoomToggle`; archived as
 * `RESEARCH/HIVETALK_OPENAPI.yaml`), which requires camelCase here even though
 * `/api/register-room` requires snake_case `room_name`.
 *
 * Provenance: the moderation transport test used to hand-feed a literal
 * `{ room, identity }` — a shape HiveTalk rejects — and passed anyway, because
 * nothing tied the test fixture to what the host controls actually send. These
 * assertions are that tie; `CallControlBar` builds no payload of its own.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  participantActionPayload,
  roomTogglePayload,
} from "./moderationPayloads.ts";

test("participantActionPayload matches HiveTalk's ParticipantAction schema", () => {
  const body = participantActionPayload("buzz-meet-spike", "bob");

  // Exact key set: required [roomName, participantIdentity], nothing else.
  assert.deepEqual(Object.keys(body).sort(), [
    "participantIdentity",
    "roomName",
  ]);
  assert.equal(body.roomName, "buzz-meet-spike");
  assert.equal(body.participantIdentity, "bob");
  // Not the registry casing, and not the old test fixture's shape.
  assert.equal(JSON.stringify(body).includes("room_name"), false);
  assert.equal("identity" in body, false);
});

test("roomTogglePayload matches HiveTalk's RoomToggle schema", () => {
  const on = roomTogglePayload("buzz-meet-spike", true);

  assert.deepEqual(Object.keys(on).sort(), ["enabled", "roomName"]);
  assert.equal(on.roomName, "buzz-meet-spike");
  assert.equal(on.enabled, true);
  // `enabled` is required, so `false` must be sent, never omitted.
  const off = roomTogglePayload("buzz-meet-spike", false);
  assert.deepEqual(Object.keys(off).sort(), ["enabled", "roomName"]);
  assert.equal(off.enabled, false);
  // The response carries `mute_on_join`/`room_name`; the request does not.
  assert.equal(JSON.stringify(off).includes("_"), false);
});
