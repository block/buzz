import assert from "node:assert/strict";
import test from "node:test";

import {
  MEETING_ROOMS_REFETCH_INTERVAL_MS,
  meetingsQueryKeys,
} from "./hooks.ts";

const RELAY = "wss://relay.example";

test("query keys are stable and scoped under the relay prefix", () => {
  assert.deepEqual(meetingsQueryKeys.all(RELAY), ["meetings", RELAY]);
  assert.deepEqual(meetingsQueryKeys.rooms(RELAY), [
    "meetings",
    RELAY,
    "rooms",
  ]);
  assert.deepEqual(meetingsQueryKeys.myRooms(RELAY, "abc"), [
    "meetings",
    RELAY,
    "my-rooms",
    "abc",
  ]);
});

test("room and my-room keys share the invalidation prefix", () => {
  const prefix = meetingsQueryKeys.all(RELAY);
  for (const key of [
    meetingsQueryKeys.rooms(RELAY),
    meetingsQueryKeys.myRooms(RELAY, "abc"),
  ]) {
    assert.deepEqual(key.slice(0, prefix.length), prefix);
  }
});

test("poll cadence matches the spec's 15s", () => {
  assert.equal(MEETING_ROOMS_REFETCH_INTERVAL_MS, 15_000);
});
