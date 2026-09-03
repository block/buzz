import assert from "node:assert/strict";
import test from "node:test";

import {
  HuddlePresenceTracker,
  HUDDLE_LIFECYCLE_PAGE_LIMIT,
  fetchHuddleLifecycleHistory,
  reconstructHuddlePresence,
} from "./huddlePresence.ts";

const ALICE = "a".repeat(64);
const BOB = "b".repeat(64);
const CHARLIE = "d".repeat(64);
const RELAY = "c".repeat(64);
const ATTACKER = "d".repeat(64);

function event({
  id,
  kind,
  pubkey = ALICE,
  session = "room",
  tags = [],
  admissionId,
  rosterRevision,
  generation,
  createdAt = Number(id),
}) {
  return {
    id,
    kind,
    pubkey,
    content: JSON.stringify({
      ephemeral_channel_id: session,
      admission_id: admissionId,
      roster_revision: rosterRevision,
      generation,
    }),
    tags,
    created_at: createdAt,
    sig: "",
  };
}

function participantEvent(options) {
  return event({ pubkey: RELAY, tags: [["p", BOB]], ...options });
}

test("reconstructs authenticated huddle participants and removes leavers", () => {
  const result = reconstructHuddlePresence(
    [
      event({ id: "1", kind: 48100 }),
      participantEvent({ id: "2", kind: 48101 }),
      participantEvent({ id: "3", kind: 48102, tags: [["p", ALICE]] }),
    ],
    RELAY,
  );

  assert.deepEqual([...result], [BOB]);
});

test("fails closed without a verified relay identity", () => {
  const result = reconstructHuddlePresence(
    [
      event({ id: "1", kind: 48100 }),
      participantEvent({ id: "2", kind: 48101 }),
    ],
    null,
  );

  assert.deepEqual([...result], []);
});

test("ignores forged participant lifecycle events", () => {
  const result = reconstructHuddlePresence(
    [
      event({ id: "1", kind: 48100 }),
      event({
        id: "2",
        kind: 48101,
        pubkey: ATTACKER,
        tags: [["p", BOB]],
      }),
      event({
        id: "3",
        kind: 48102,
        pubkey: ATTACKER,
        tags: [["p", ALICE]],
      }),
    ],
    RELAY,
  );

  assert.deepEqual([...result], []);
});

test("keeps a participant until their final admission leaves", () => {
  const result = reconstructHuddlePresence(
    [
      event({ id: "1", kind: 48100 }),
      participantEvent({
        id: "2",
        kind: 48101,
        admissionId: "desktop",
        rosterRevision: 1,
      }),
      participantEvent({
        id: "3",
        kind: 48101,
        admissionId: "mobile",
        rosterRevision: 2,
      }),
      participantEvent({
        id: "4",
        kind: 48102,
        admissionId: "desktop",
        rosterRevision: 3,
      }),
    ],
    RELAY,
  );

  assert.equal(result.has(BOB), true);
});

test("orders same-second reconnect events by roster revision", () => {
  const result = reconstructHuddlePresence(
    [
      event({ id: "1", kind: 48100, createdAt: 1 }),
      participantEvent({
        id: "z",
        kind: 48101,
        admissionId: "new",
        rosterRevision: 3,
        createdAt: 2,
      }),
      participantEvent({
        id: "a",
        kind: 48102,
        admissionId: "old",
        rosterRevision: 2,
        createdAt: 2,
      }),
    ],
    RELAY,
  );

  assert.equal(result.has(BOB), true);
});

test("ignores an older replay for the same admission", () => {
  const tracker = new HuddlePresenceTracker(RELAY);
  tracker.apply(event({ id: "1", kind: 48100 }));
  tracker.apply(
    participantEvent({
      id: "3",
      kind: 48102,
      admissionId: "desktop",
      rosterRevision: 3,
    }),
  );
  assert.equal(tracker.snapshot().has(BOB), false);
  tracker.apply(
    participantEvent({
      id: "2",
      kind: 48101,
      admissionId: "desktop",
      rosterRevision: 2,
    }),
  );

  assert.equal(tracker.snapshot().has(BOB), false);
});

test("retains a legacy leave tombstone across snapshots", () => {
  const tracker = new HuddlePresenceTracker(RELAY);
  tracker.apply(event({ id: "1", kind: 48100, createdAt: 1 }));
  tracker.apply(participantEvent({ id: "2", kind: 48101, createdAt: 2 }));
  assert.equal(tracker.snapshot().has(BOB), true);

  tracker.apply(participantEvent({ id: "3", kind: 48102, createdAt: 3 }));
  assert.equal(tracker.snapshot().has(BOB), false);

  assert.equal(
    tracker.apply(participantEvent({ id: "2", kind: 48101, createdAt: 2 })),
    false,
  );
  assert.equal(tracker.snapshot().has(BOB), false);
});

test("accepts an older revision from another participant and admission", () => {
  const tracker = new HuddlePresenceTracker(RELAY);
  tracker.apply(event({ id: "1", kind: 48100 }));
  tracker.apply(
    participantEvent({
      id: "3",
      kind: 48102,
      admissionId: "new",
      rosterRevision: 3,
    }),
  );

  assert.equal(
    tracker.apply(
      participantEvent({
        id: "2",
        kind: 48101,
        admissionId: "old",
        rosterRevision: 2,
        tags: [["p", CHARLIE]],
      }),
    ),
    true,
  );
  assert.equal(tracker.snapshot().has(CHARLIE), true);
});

test("accepts lower revisions from a new relay room generation", () => {
  const tracker = new HuddlePresenceTracker(RELAY);
  tracker.apply(event({ id: "1", kind: 48100 }));
  tracker.apply(
    participantEvent({
      id: "2",
      kind: 48101,
      admissionId: "before-restart",
      rosterRevision: 20,
    }),
  );

  assert.equal(
    tracker.apply(
      participantEvent({
        id: "3",
        kind: 48101,
        admissionId: "after-restart",
        rosterRevision: 1,
      }),
    ),
    true,
  );
  assert.equal(tracker.snapshot().has(BOB), true);

  tracker.apply(
    participantEvent({
      id: "4",
      kind: 48101,
      admissionId: "another-participant",
      rosterRevision: 2,
      tags: [["p", CHARLIE]],
    }),
  );
  tracker.apply(
    participantEvent({
      id: "5",
      kind: 48102,
      admissionId: "after-restart",
      rosterRevision: 3,
    }),
  );
  assert.equal(tracker.snapshot().has(BOB), false);
  assert.equal(tracker.snapshot().has(CHARLIE), true);

  const hydrated = reconstructHuddlePresence(
    [
      event({ id: "1", kind: 48100 }),
      participantEvent({
        id: "2",
        kind: 48101,
        admissionId: "before-restart",
        rosterRevision: 20,
      }),
      participantEvent({
        id: "3",
        kind: 48101,
        admissionId: "after-restart",
        rosterRevision: 1,
      }),
      participantEvent({
        id: "4",
        kind: 48101,
        admissionId: "another-participant",
        rosterRevision: 2,
        tags: [["p", CHARLIE]],
      }),
      participantEvent({
        id: "5",
        kind: 48102,
        admissionId: "after-restart",
        rosterRevision: 3,
      }),
    ],
    RELAY,
  );
  assert.equal(hydrated.has(BOB), false);
  assert.equal(hydrated.has(CHARLIE), true);
});

test("preserves reordered admissions in one explicit generation", () => {
  const tracker = new HuddlePresenceTracker(RELAY);
  tracker.apply(event({ id: "1", kind: 48100 }));
  tracker.apply(
    participantEvent({
      id: "2",
      kind: 48101,
      admissionId: "charlie",
      rosterRevision: 2,
      generation: "7",
      tags: [["p", CHARLIE]],
      createdAt: 2,
    }),
  );

  assert.equal(
    tracker.apply(
      participantEvent({
        id: "3",
        kind: 48101,
        admissionId: "bob",
        rosterRevision: 1,
        generation: "7",
        createdAt: 3,
      }),
    ),
    true,
  );
  assert.deepEqual(tracker.snapshot(), new Set([BOB, CHARLIE]));
});

test("clears legacy admissions on the first explicit generation", () => {
  const tracker = new HuddlePresenceTracker(RELAY);
  tracker.apply(event({ id: "1", kind: 48100 }));
  tracker.apply(
    participantEvent({
      id: "2",
      kind: 48101,
      admissionId: "legacy-bob",
      rosterRevision: 20,
      createdAt: 2,
    }),
  );

  assert.equal(
    tracker.apply(
      participantEvent({
        id: "3",
        kind: 48101,
        admissionId: "generated-charlie",
        rosterRevision: 1,
        generation: "7",
        tags: [["p", CHARLIE]],
        createdAt: 3,
      }),
    ),
    true,
  );
  assert.deepEqual(tracker.snapshot(), new Set([CHARLIE]));
});

test("rejects lifecycle events after liveness advances the generation", () => {
  const tracker = new HuddlePresenceTracker(RELAY);
  tracker.apply(event({ id: "1", kind: 48100 }));
  tracker.apply(
    participantEvent({
      id: "2",
      kind: 48101,
      admissionId: "generation-1",
      rosterRevision: 1,
      generation: "1",
    }),
  );
  tracker.reconcileLiveness(new Map([["room", "1"]]), new Map([["room", "1"]]));
  tracker.apply(
    participantEvent({
      id: "3",
      kind: 48101,
      admissionId: "generation-2",
      rosterRevision: 1,
      generation: "2",
      tags: [["p", CHARLIE]],
    }),
  );

  assert.equal(
    tracker.apply(
      participantEvent({
        id: "4",
        kind: 48102,
        admissionId: "generation-1",
        rosterRevision: 2,
        generation: "1",
      }),
    ),
    false,
  );
  assert.equal(
    tracker.apply(
      participantEvent({
        id: "5",
        kind: 48101,
        admissionId: "delayed-generation-1",
        rosterRevision: 3,
        generation: "1",
      }),
    ),
    false,
  );
  assert.equal(tracker.snapshot().has(BOB), false);
  assert.equal(tracker.snapshot().has(CHARLIE), true);
});

test("accepts a new generation join before liveness establishes authority", () => {
  const tracker = new HuddlePresenceTracker(RELAY);
  tracker.apply(event({ id: "start", kind: 48100 }));
  tracker.apply(
    participantEvent({
      id: "old",
      kind: 48101,
      admissionId: "old-admission",
      rosterRevision: 1,
      generation: "1",
    }),
  );

  assert.equal(
    tracker.apply(
      participantEvent({
        id: "new",
        kind: 48101,
        admissionId: "new-admission",
        rosterRevision: 1,
        generation: "2",
        tags: [["p", CHARLIE]],
      }),
    ),
    true,
  );
  assert.equal(tracker.snapshot().has(BOB), false);
  assert.equal(tracker.snapshot().has(CHARLIE), true);
});

test("waits for liveness before accepting a new opaque generation", () => {
  const generation1 = "11111111-1111-4111-8111-111111111111";
  const generation2 = "22222222-2222-4222-8222-222222222222";
  const tracker = new HuddlePresenceTracker(RELAY);
  tracker.apply(event({ id: "start", kind: 48100 }));
  tracker.apply(
    participantEvent({
      id: "old",
      kind: 48101,
      admissionId: "old-admission",
      rosterRevision: 1,
      generation: generation1,
    }),
  );
  tracker.reconcileLiveness(
    new Map([["room", generation1]]),
    new Map([["room", generation1]]),
  );
  const newJoin = participantEvent({
    id: "new",
    kind: 48101,
    admissionId: "new-admission",
    rosterRevision: 1,
    generation: generation2,
    tags: [["p", CHARLIE]],
  });

  assert.equal(tracker.apply(newJoin), false);
  assert.equal(tracker.snapshot().has(BOB), true);
  assert.equal(tracker.snapshot().has(CHARLIE), false);

  tracker.reconcileLiveness(
    new Map([["room", generation2]]),
    new Map([["room", generation1]]),
  );
  assert.equal(tracker.apply(newJoin), true);
  assert.equal(tracker.snapshot().has(BOB), false);
  assert.equal(tracker.snapshot().has(CHARLIE), true);
});

test("preserves distinct admissions that share one roster revision", () => {
  const events = [
    event({ id: "1", kind: 48100 }),
    participantEvent({
      id: "2",
      kind: 48101,
      admissionId: "charlie-local",
      rosterRevision: 3,
      tags: [["p", CHARLIE]],
    }),
    participantEvent({
      id: "3",
      kind: 48101,
      admissionId: "bob-remote",
      rosterRevision: 3,
    }),
  ];

  const tracker = new HuddlePresenceTracker(RELAY);
  for (const item of events) tracker.apply(item);
  assert.equal(tracker.snapshot().has(ALICE), false);
  assert.equal(tracker.snapshot().has(BOB), true);
  assert.equal(tracker.snapshot().has(CHARLIE), true);

  const hydrated = reconstructHuddlePresence(events, RELAY);
  assert.equal(hydrated.has(ALICE), false);
  assert.equal(hydrated.has(BOB), true);
  assert.equal(hydrated.has(CHARLIE), true);
});

test("does not infer a room restart from a repeated roster revision", () => {
  const events = [
    event({ id: "1", kind: 48100 }),
    participantEvent({
      id: "2",
      kind: 48101,
      admissionId: "before-restart",
      rosterRevision: 1,
    }),
    participantEvent({
      id: "3",
      kind: 48101,
      admissionId: "after-restart",
      rosterRevision: 1,
    }),
    participantEvent({
      id: "4",
      kind: 48101,
      admissionId: "charlie-after-restart",
      rosterRevision: 2,
      tags: [["p", CHARLIE]],
    }),
    participantEvent({
      id: "5",
      kind: 48102,
      admissionId: "after-restart",
      rosterRevision: 3,
    }),
  ];

  const tracker = new HuddlePresenceTracker(RELAY);
  for (const item of events) tracker.apply(item);
  assert.equal(tracker.snapshot().has(BOB), true);
  assert.equal(tracker.snapshot().has(CHARLIE), true);

  const hydrated = reconstructHuddlePresence(events, RELAY);
  assert.equal(hydrated.has(BOB), true);
  assert.equal(hydrated.has(CHARLIE), true);
});

test("a lower-id same-second start is canonical and clears old participants", () => {
  const tracker = new HuddlePresenceTracker(RELAY);
  tracker.apply(event({ id: "z", kind: 48100, createdAt: 10 }));
  tracker.apply(
    participantEvent({
      id: "join",
      kind: 48101,
      admissionId: "desktop",
      rosterRevision: 1,
      createdAt: 11,
    }),
  );
  assert.equal(tracker.snapshot().has(BOB), true);

  assert.equal(
    tracker.apply(event({ id: "a", kind: 48100, createdAt: 10 })),
    true,
  );
  assert.equal(tracker.snapshot().has(BOB), false);
  assert.equal(tracker.snapshot().has(ALICE), false);
});

test("rejects an unauthorized end signer", () => {
  const result = reconstructHuddlePresence(
    [
      event({ id: "1", kind: 48100 }),
      event({ id: "2", kind: 48103, pubkey: ATTACKER }),
    ],
    RELAY,
  );

  assert.deepEqual([...result], []);
});

test("accepts either creator-signed or relay-signed end events", () => {
  for (const pubkey of [ALICE, RELAY]) {
    const result = reconstructHuddlePresence(
      [
        event({ id: "1", kind: 48100 }),
        event({ id: "2", kind: 48103, pubkey }),
      ],
      RELAY,
    );
    assert.deepEqual([...result], []);
  }
});

test("requires a canonical start before participant events", () => {
  const result = reconstructHuddlePresence(
    [participantEvent({ id: "2", kind: 48101 })],
    RELAY,
  );

  assert.deepEqual([...result], []);
});

test("tracks simultaneous sessions and clears only the ended huddle", () => {
  const result = reconstructHuddlePresence(
    [
      event({ id: "1", kind: 48100, session: "first" }),
      event({ id: "2", kind: 48100, pubkey: BOB, session: "second" }),
      event({ id: "3", kind: 48103, session: "first" }),
    ],
    RELAY,
  );

  assert.deepEqual([...result], []);
});

test("pages complete lifecycle history without a fixed lifetime horizon", async () => {
  const firstPage = Array.from(
    { length: HUDDLE_LIFECYCLE_PAGE_LIMIT },
    (_, index) =>
      event({
        id: `first-${index}`,
        kind: 48101,
        createdAt: 9_500 - index,
      }),
  );
  const boundary = firstPage.at(-1).created_at;
  const secondPage = [
    firstPage.at(-1),
    event({ id: "older-start", kind: 48100, createdAt: boundary - 1 }),
  ];
  const filters = [];

  const result = await fetchHuddleLifecycleHistory(async (filter) => {
    filters.push(filter);
    return filters.length === 1 ? firstPage : secondPage;
  });

  assert.equal(result.length, HUDDLE_LIFECYCLE_PAGE_LIMIT + 1);
  assert.equal(filters[0].since, undefined);
  assert.equal(filters[0].until, undefined);
  assert.equal(filters[1].until, boundary);
  assert.equal(
    result.some((item) => item.id === "older-start"),
    true,
  );
});

test("advances dense lifecycle timestamps with the composite cursor", async () => {
  const page = Array.from({ length: HUDDLE_LIFECYCLE_PAGE_LIMIT }, (_, index) =>
    event({
      id: index.toString(16).padStart(64, "0"),
      kind: 48101,
      createdAt: 9_000,
    }),
  );
  const filters = [];
  const result = await fetchHuddleLifecycleHistory(async (filter) => {
    filters.push(filter);
    return filters.length === 1
      ? page
      : [event({ id: "older", kind: 48100, createdAt: 8_999 })];
  });

  assert.equal(filters[1].until, 9_000);
  assert.equal(filters[1].before_id, page.at(-1).id);
  assert.equal(result.length, HUDDLE_LIFECYCLE_PAGE_LIMIT + 1);
});

test("bounds departed admissions while rejecting compacted late joins", () => {
  const tracker = new HuddlePresenceTracker(RELAY);
  tracker.apply(event({ id: "start", kind: 48100, createdAt: 1 }));
  for (let revision = 1; revision <= 1_002; revision += 1) {
    tracker.apply(
      participantEvent({
        id: `leave-${revision}`,
        kind: 48102,
        admissionId: `admission-${revision}`,
        rosterRevision: revision,
        createdAt: revision + 1,
      }),
    );
  }

  assert.equal(
    tracker.apply(
      participantEvent({
        id: "late-join",
        kind: 48101,
        admissionId: "admission-1",
        rosterRevision: 1,
        createdAt: 2_000,
      }),
    ),
    false,
  );
  assert.equal(tracker.snapshot().has(BOB), false);
});

test("reconcileLiveness preserves admissions for the same generation", () => {
  const tracker = new HuddlePresenceTracker(RELAY);
  tracker.apply(event({ id: "start", kind: 48100, createdAt: 1 }));
  tracker.apply(
    participantEvent({
      id: "old",
      kind: 48101,
      admissionId: "old-admission",
      rosterRevision: 1,
      createdAt: 2,
    }),
  );

  tracker.reconcileLiveness(new Map([["room", "generation-1"]]));
  tracker.reconcileLiveness(
    new Map([["room", "generation-1"]]),
    new Map([["room", "generation-1"]]),
  );

  assert.equal(tracker.snapshot().has(BOB), true);
});

test("lifecycle generation change clears equal-revision stale admissions", () => {
  const tracker = new HuddlePresenceTracker(RELAY);
  tracker.apply(event({ id: "start", kind: 48100, createdAt: 1 }));
  tracker.apply(
    participantEvent({
      id: "old",
      kind: 48101,
      admissionId: "old-admission",
      rosterRevision: 1,
      createdAt: 2,
      generation: "1",
    }),
  );
  tracker.apply(
    participantEvent({
      id: "new",
      kind: 48101,
      admissionId: "new-admission",
      rosterRevision: 1,
      createdAt: 3,
      generation: "2",
      tags: [["p", CHARLIE]],
    }),
  );

  assert.equal(tracker.snapshot().has(BOB), false);
  assert.equal(tracker.snapshot().has(CHARLIE), true);
});

test("incremental state retains an end tombstone and ignores late events", () => {
  const tracker = new HuddlePresenceTracker(RELAY);
  tracker.apply(event({ id: "1", kind: 48100 }));
  tracker.apply(event({ id: "2", kind: 48103, pubkey: RELAY }));

  assert.equal(
    tracker.apply(participantEvent({ id: "3", kind: 48101 })),
    false,
  );
  assert.deepEqual([...tracker.snapshot()], []);
});

test("incremental state ignores an older start replayed after an active session", () => {
  const tracker = new HuddlePresenceTracker(RELAY);
  tracker.apply(event({ id: "new", kind: 48100, createdAt: 10 }));
  tracker.apply(
    participantEvent({
      id: "join",
      kind: 48101,
      admissionId: "desktop",
      rosterRevision: 1,
      createdAt: 11,
    }),
  );

  assert.equal(
    tracker.apply(
      event({ id: "old", kind: 48100, pubkey: ATTACKER, createdAt: 9 }),
    ),
    false,
  );
  assert.equal(tracker.snapshot().has(BOB), true);
});

test("shows only participants with authenticated audio admissions", () => {
  const tracker = new HuddlePresenceTracker(RELAY);
  tracker.apply(event({ id: "1", kind: 48100 }));
  tracker.apply(
    participantEvent({
      id: "2",
      kind: 48101,
      admissionId: "participant-device",
      rosterRevision: 1,
    }),
  );

  assert.equal(tracker.snapshot().has(ALICE), false);
  assert.equal(tracker.snapshot().has(BOB), true);

  tracker.apply(
    participantEvent({
      id: "3",
      kind: 48101,
      tags: [["p", ALICE]],
      admissionId: "creator-device",
      rosterRevision: 2,
    }),
  );
  assert.equal(tracker.snapshot().has(ALICE), true);

  tracker.apply(
    participantEvent({
      id: "4",
      kind: 48102,
      tags: [["p", ALICE]],
      admissionId: "creator-device",
      rosterRevision: 3,
    }),
  );
  assert.equal(tracker.snapshot().has(ALICE), false);
});
