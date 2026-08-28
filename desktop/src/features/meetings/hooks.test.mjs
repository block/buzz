import assert from "node:assert/strict";
import test from "node:test";

import {
  MEETING_ROOMS_REFETCH_INTERVAL_MS,
  PAYMENT_STATUS_POLL_INTERVAL_MS,
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
  assert.deepEqual(meetingsQueryKeys.token(RELAY, "standup"), [
    "meetings",
    RELAY,
    "token",
    "standup",
  ]);
  assert.deepEqual(meetingsQueryKeys.plans(RELAY), [
    "meetings",
    RELAY,
    "plans",
  ]);
  assert.deepEqual(meetingsQueryKeys.subscription(RELAY, "abc"), [
    "meetings",
    RELAY,
    "subscription",
    "abc",
  ]);
  assert.deepEqual(meetingsQueryKeys.payment(RELAY, "int_1"), [
    "meetings",
    RELAY,
    "payment",
    "int_1",
  ]);
});

test("room and my-room keys share the invalidation prefix", () => {
  const prefix = meetingsQueryKeys.all(RELAY);
  for (const key of [
    meetingsQueryKeys.rooms(RELAY),
    meetingsQueryKeys.myRooms(RELAY, "abc"),
    meetingsQueryKeys.token(RELAY, "standup"),
    meetingsQueryKeys.plans(RELAY),
    meetingsQueryKeys.subscription(RELAY, "abc"),
    meetingsQueryKeys.payment(RELAY, "int_1"),
  ]) {
    assert.deepEqual(key.slice(0, prefix.length), prefix);
  }
});

test("poll cadence matches the spec's 15s", () => {
  assert.equal(MEETING_ROOMS_REFETCH_INTERVAL_MS, 15_000);
});

test("payment status poll cadence matches the spec's ~3s", () => {
  assert.equal(PAYMENT_STATUS_POLL_INTERVAL_MS, 3_000);
});
