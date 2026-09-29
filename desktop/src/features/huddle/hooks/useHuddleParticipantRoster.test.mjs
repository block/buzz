import assert from "node:assert/strict";
import test from "node:test";

import { reconstructHuddleParticipantRoster as reconstructRoster } from "./useHuddleParticipantRoster.ts";

function reconstructHuddleParticipantRoster(options) {
  const events = [...options.events];
  return reconstructRoster({
    ...options,
    events,
    relaySelfPubkey: "relay",
    huddleThreadEventId:
      options.huddleThreadEventId ??
      events.find((candidate) => candidate.kind === 48100)?.id ??
      null,
  });
}

const room = "huddle-room";

function event({
  id,
  kind,
  participant,
  pubkey = "relay",
  createdAt = 1,
  channel = room,
  rosterRevision,
  admissionId,
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
      ...(admissionId === undefined ? {} : { admission_id: admissionId }),
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

test("orders same-second join then leave by admission identity", () => {
  const events = [
    event({
      id: "joined",
      kind: 48101,
      participant: "mobile",
      createdAt: 2,
      rosterRevision: 3,
      admissionId: "admission-a",
    }),
    event({
      id: "left",
      kind: 48102,
      participant: "mobile",
      createdAt: 2,
      admissionId: "admission-a",
    }),
  ];

  assert.deepEqual(
    reconstructHuddleParticipantRoster({
      ephemeralChannelId: room,
      events,
      fallbackParticipants: ["mobile"],
    }),
    [],
  );
});

test("keeps a participant until their final admission leaves", () => {
  const events = [
    event({
      id: "joined-desktop",
      kind: 48101,
      participant: "mobile",
      createdAt: 2,
      rosterRevision: 2,
      admissionId: "desktop-admission",
    }),
    event({
      id: "joined-phone",
      kind: 48101,
      participant: "mobile",
      createdAt: 3,
      rosterRevision: 3,
      admissionId: "phone-admission",
    }),
    event({
      id: "left-desktop",
      kind: 48102,
      participant: "mobile",
      createdAt: 4,
      rosterRevision: 4,
      admissionId: "desktop-admission",
    }),
  ];

  assert.deepEqual(
    reconstructHuddleParticipantRoster({
      ephemeralChannelId: room,
      events,
      fallbackParticipants: [],
    }),
    ["mobile"],
  );

  events.push(
    event({
      id: "left-phone",
      kind: 48102,
      participant: "mobile",
      createdAt: 5,
      rosterRevision: 5,
      admissionId: "phone-admission",
    }),
  );
  assert.deepEqual(
    reconstructHuddleParticipantRoster({
      ephemeralChannelId: room,
      events,
      fallbackParticipants: [],
    }),
    [],
  );
});

test("orders same-second unrevisioned remote leave before revised rejoin", () => {
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

test("rejects lifecycle events without relay or creator provenance", () => {
  const started = event({
    id: "started",
    kind: 48100,
    pubkey: "creator",
    createdAt: 1,
  });
  const events = [
    started,
    event({
      id: "forged-left",
      kind: 48102,
      participant: "mobile",
      pubkey: "attacker",
      createdAt: 2,
    }),
    event({
      id: "forged-start",
      kind: 48100,
      pubkey: "attacker",
      createdAt: 3,
    }),
    event({
      id: "forged-end",
      kind: 48103,
      pubkey: "attacker",
      createdAt: 4,
    }),
  ];

  assert.deepEqual(
    reconstructHuddleParticipantRoster({
      ephemeralChannelId: room,
      events,
      fallbackParticipants: ["mobile"],
      huddleThreadEventId: started.id,
    }),
    ["creator"],
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

test("preserves live fallback peers when the canonical start is outside history", () => {
  const canonicalStart = event({
    id: "canonical-start",
    kind: 48100,
    pubkey: "desktop",
    createdAt: 1,
  });
  const events = [
    event({
      id: "recent-join",
      kind: 48101,
      participant: "newcomer",
      createdAt: 2,
    }),
  ];

  assert.deepEqual(
    reconstructHuddleParticipantRoster({
      ephemeralChannelId: room,
      events,
      fallbackParticipants: ["desktop", "long-lived"],
      huddleThreadEventId: canonicalStart.id,
      canonicalStartEvent: canonicalStart,
    }),
    ["desktop", "long-lived", "newcomer"],
  );
});

test("authenticates an end with a canonical start outside the lifecycle window", () => {
  const canonicalStart = event({
    id: "canonical-start",
    kind: 48100,
    pubkey: "desktop",
    createdAt: 1,
  });
  const events = [
    event({ id: "joined", kind: 48101, participant: "mobile", createdAt: 2 }),
    event({
      id: "ended",
      kind: 48103,
      pubkey: "desktop",
      createdAt: 3,
    }),
  ];

  assert.deepEqual(
    reconstructHuddleParticipantRoster({
      ephemeralChannelId: room,
      events,
      fallbackParticipants: ["desktop", "mobile"],
      huddleThreadEventId: canonicalStart.id,
      canonicalStartEvent: canonicalStart,
    }),
    [],
  );
});
