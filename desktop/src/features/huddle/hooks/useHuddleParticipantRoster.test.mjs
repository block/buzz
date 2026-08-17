import assert from "node:assert/strict";
import test from "node:test";

import { reconstructHuddleParticipantRoster } from "./useHuddleParticipantRoster.ts";

const room = "huddle-room";

function event({
  id,
  kind,
  participant,
  pubkey = "relay",
  createdAt = 1,
  channel = room,
  rosterRevision,
}) {
  return {
    id,
    kind,
    pubkey,
    created_at: createdAt,
    content: JSON.stringify({
      ephemeral_channel_id: channel,
      ...(rosterRevision === undefined
        ? {}
        : { roster_revision: rosterRevision }),
    }),
    tags: participant ? [["p", participant]] : [],
    sig: "",
  };
}

test("applies relay-signed joins and leaves over membership fallback", () => {
  const events = [
    event({ id: "left", kind: 48102, participant: "mobile", createdAt: 4 }),
    event({ id: "joined", kind: 48101, participant: "mobile", createdAt: 3 }),
    event({ id: "started", kind: 48100, pubkey: "desktop", createdAt: 1 }),
  ];

  assert.deepEqual(
    reconstructHuddleParticipantRoster({
      ephemeralChannelId: room,
      events: events.slice(1),
      fallbackParticipants: ["desktop"],
    }),
    ["desktop", "mobile"],
  );
  assert.deepEqual(
    reconstructHuddleParticipantRoster({
      ephemeralChannelId: room,
      events,
      fallbackParticipants: ["desktop", "mobile"],
    }),
    ["desktop"],
  );
});

test("ignores other rooms and preserves local and agent participants", () => {
  const events = [
    event({
      id: "other-join",
      kind: 48101,
      participant: "someone-else",
      channel: "another-room",
    }),
  ];

  assert.deepEqual(
    reconstructHuddleParticipantRoster({
      ephemeralChannelId: room,
      events,
      fallbackParticipants: ["DESKTOP"],
      preservedParticipants: ["desktop", "AGENT"],
    }),
    ["desktop", "agent"],
  );
});

test("orders same-second start before participant joins", () => {
  const events = [
    event({
      id: "join-sorts-before-start-by-id",
      kind: 48101,
      participant: "mobile",
      createdAt: 2,
      rosterRevision: 2,
    }),
    event({
      id: "start-sorts-after-join-by-id",
      kind: 48100,
      pubkey: "desktop",
      createdAt: 2,
    }),
  ];

  assert.deepEqual(
    reconstructHuddleParticipantRoster({
      ephemeralChannelId: room,
      events,
      fallbackParticipants: [],
    }),
    ["desktop", "mobile"],
  );
});

test("orders same-second leave then rejoin by roster revision", () => {
  const events = [
    event({
      id: "joined",
      kind: 48101,
      participant: "mobile",
      createdAt: 2,
      rosterRevision: 3,
    }),
    event({
      id: "left",
      kind: 48102,
      participant: "mobile",
      createdAt: 2,
      rosterRevision: 2,
    }),
  ];

  assert.deepEqual(
    reconstructHuddleParticipantRoster({
      ephemeralChannelId: room,
      events,
      fallbackParticipants: ["mobile"],
    }),
    ["mobile"],
  );
});

test("an ended lifecycle does not retain stale membership", () => {
  const events = [
    event({ id: "started", kind: 48100, pubkey: "desktop", createdAt: 1 }),
    event({ id: "joined", kind: 48101, participant: "mobile", createdAt: 2 }),
    event({ id: "ended", kind: 48103, createdAt: 3 }),
  ];

  assert.deepEqual(
    reconstructHuddleParticipantRoster({
      ephemeralChannelId: room,
      events,
      fallbackParticipants: ["desktop", "mobile"],
      preservedParticipants: ["desktop"],
    }),
    [],
  );
});
