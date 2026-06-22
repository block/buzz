import assert from "node:assert/strict";
import test from "node:test";

import {
  hasNavigableTarget,
  resolveReminderDestination,
} from "./reminderNavigation.ts";

const FULL_TARGET = {
  eventId: "evt-1",
  channelId: "chan-1",
  preview: "hello",
  authorPubkey: "author-1",
};

/** Build a RelayEvent with the given e-tags. */
function eventWithTags(tags) {
  return { id: FULL_TARGET.eventId, tags };
}

// ── hasNavigableTarget ──────────────────────────────────────────────────────

test("hasNavigableTarget_full_target_is_navigable", () => {
  assert.equal(hasNavigableTarget(FULL_TARGET), true);
});

test("hasNavigableTarget_absent_target_is_not_navigable", () => {
  assert.equal(hasNavigableTarget(undefined), false);
});

test("hasNavigableTarget_empty_channelId_is_not_navigable", () => {
  assert.equal(hasNavigableTarget({ ...FULL_TARGET, channelId: "" }), false);
});

test("hasNavigableTarget_empty_eventId_is_not_navigable", () => {
  assert.equal(hasNavigableTarget({ ...FULL_TARGET, eventId: "" }), false);
});

test("hasNavigableTarget_empty_authorPubkey_is_not_navigable", () => {
  assert.equal(hasNavigableTarget({ ...FULL_TARGET, authorPubkey: "" }), false);
});

// ── resolveReminderDestination ──────────────────────────────────────────────

test("resolveReminderDestination_nested_reply_lands_in_thread", async () => {
  const fetchEvent = async () =>
    eventWithTags([
      ["e", "root-evt", "", "root"],
      ["e", "parent-evt", "", "reply"],
    ]);

  const destination = await resolveReminderDestination(FULL_TARGET, fetchEvent);

  assert.deepEqual(destination, {
    channelId: "chan-1",
    messageId: "evt-1",
    threadRootId: "root-evt",
  });
});

test("resolveReminderDestination_top_level_message_lands_channel_level", async () => {
  // A non-reply message has no reply tag -> getThreadReference yields null root.
  const fetchEvent = async () => eventWithTags([["h", "chan-1"]]);

  const destination = await resolveReminderDestination(FULL_TARGET, fetchEvent);

  assert.deepEqual(destination, {
    channelId: "chan-1",
    messageId: "evt-1",
    threadRootId: null,
  });
});

test("resolveReminderDestination_fetch_failure_degrades_to_channel_level", async () => {
  const fetchEvent = async () => {
    throw new Error("event not cached");
  };

  const destination = await resolveReminderDestination(FULL_TARGET, fetchEvent);

  assert.deepEqual(destination, {
    channelId: "chan-1",
    messageId: "evt-1",
    threadRootId: null,
  });
});

test("resolveReminderDestination_non_navigable_target_returns_null", async () => {
  let fetched = false;
  const fetchEvent = async () => {
    fetched = true;
    return eventWithTags([]);
  };

  const destination = await resolveReminderDestination(
    { ...FULL_TARGET, channelId: "" },
    fetchEvent,
  );

  assert.equal(destination, null);
  assert.equal(fetched, false);
});

test("resolveReminderDestination_absent_target_returns_null", async () => {
  const destination = await resolveReminderDestination(undefined, async () => {
    throw new Error("should not fetch");
  });

  assert.equal(destination, null);
});
