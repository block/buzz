import assert from "node:assert/strict";
import test from "node:test";

import { startHuddlePresenceRuntime } from "./huddlePresenceRuntime.ts";

const ALICE = "a".repeat(64);
const BOB = "b".repeat(64);
const CAROL = "d".repeat(64);
const RELAY = "c".repeat(64);

function event({
  id,
  kind,
  pubkey = ALICE,
  tags = [],
  admissionId,
  rosterRevision,
  generation,
  createdAt = Number(id),
  session = "room",
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
    tags: tags.some((tag) => tag[0] === "h")
      ? tags
      : [["h", "general"], ...tags],
    created_at: createdAt,
    sig: "",
  };
}

function participantEvent(options) {
  return event({ pubkey: RELAY, tags: [["p", BOB]], ...options });
}

function livenessEvent(session = "room", generation = "1") {
  return event({
    id: `live-${session}`,
    kind: 48104,
    createdAt: 1_000,
    session,
    generation,
  });
}

async function settle() {
  await new Promise((resolve) => setImmediate(resolve));
  await new Promise((resolve) => setImmediate(resolve));
}

function runtimeHarness(initialHistory) {
  let history = initialHistory;
  let liveSessions = ["room"];
  let liveGeneration = "1";
  let reconnect;
  let liveHandler;
  let livenessTimer;
  let livenessDelay;
  let liveDisposed = false;
  let reconnectDisposed = false;
  const snapshots = [];
  const filters = [];
  const runtime = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["general", "design"],
    subscribeLive: async (filter, handler) => {
      filters.push(filter);
      liveHandler = handler;
      return () => {
        liveDisposed = true;
      };
    },
    fetchEvents: async (filter) =>
      filter.kinds?.includes(48104)
        ? liveSessions.map((session) => livenessEvent(session, liveGeneration))
        : history,
    subscribeToReconnects: (listener) => {
      reconnect = listener;
      return () => {
        reconnectDisposed = true;
      };
    },
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setLivenessTimer: (callback, delayMs) => {
      livenessTimer = callback;
      livenessDelay = delayMs;
      return callback;
    },
    clearLivenessTimer: () => {
      livenessTimer = undefined;
    },
    nowSeconds: () => 1_000,
  });

  return {
    dispose: runtime,
    emit: (next) => liveHandler(next),
    filters,
    reconnect: () => reconnect(),
    refreshLiveness: () => livenessTimer(),
    setHistory: (next) => {
      history = next;
    },
    setLiveSessions: (next) => {
      liveSessions = next;
    },
    setLiveGeneration: (next) => {
      liveGeneration = next;
    },
    snapshots,
    livenessDelay: () => livenessDelay,
    wasDisposed: () => liveDisposed && reconnectDisposed,
  };
}

test("hydrates lifecycle history in global phase and revision order", async () => {
  const harness = runtimeHarness([
    participantEvent({
      id: "join",
      kind: 48101,
      admissionId: "desktop",
      rosterRevision: 1,
      createdAt: 10,
    }),
    event({ id: "start", kind: 48100, createdAt: 10 }),
  ]);

  await settle();

  assert.equal(harness.snapshots.at(-1).has(ALICE), false);
  assert.equal(harness.snapshots.at(-1).has(BOB), true);
  assert.equal(harness.filters[0].limit > 0, true);
  assert.equal(harness.filters[0].since, 1_000);
  assert.deepEqual(harness.filters[0]["#h"], ["design", "general"]);
  harness.dispose();
});

test("reconciles joins, leaves, and ends missed during disconnect", async () => {
  const start = event({ id: "1", kind: 48100 });
  const join = participantEvent({
    id: "2",
    kind: 48101,
    admissionId: "desktop",
    rosterRevision: 1,
  });
  const left = participantEvent({
    id: "3",
    kind: 48102,
    admissionId: "desktop",
    rosterRevision: 2,
  });
  const ended = event({ id: "4", kind: 48103, pubkey: RELAY });
  const harness = runtimeHarness([start]);
  await settle();

  harness.setHistory([join, start]);
  harness.reconnect();
  await settle();
  assert.equal(harness.snapshots.at(-1).has(BOB), true);

  harness.setHistory([left, join, start]);
  harness.reconnect();
  await settle();
  assert.equal(harness.snapshots.at(-1).has(BOB), false);
  assert.equal(harness.snapshots.at(-1).has(ALICE), false);

  harness.setHistory([ended, left, join, start]);
  harness.reconnect();
  await settle();
  assert.deepEqual([...harness.snapshots.at(-1)], []);
  harness.dispose();
});

test("applies channel-scoped live joins, leaves, and ends without reconnecting", async () => {
  const start = event({
    id: "1",
    kind: 48100,
    tags: [["h", "general"]],
  });
  const harness = runtimeHarness([start]);
  await settle();

  harness.emit(
    participantEvent({
      id: "2",
      kind: 48101,
      admissionId: "desktop",
      rosterRevision: 1,
      tags: [
        ["h", "general"],
        ["p", BOB],
      ],
    }),
  );
  assert.equal(harness.snapshots.at(-1).has(BOB), true);

  harness.emit(
    participantEvent({
      id: "3",
      kind: 48102,
      admissionId: "desktop",
      rosterRevision: 2,
      tags: [
        ["h", "general"],
        ["p", BOB],
      ],
    }),
  );
  assert.equal(harness.snapshots.at(-1).has(BOB), false);
  assert.equal(harness.snapshots.at(-1).has(ALICE), false);

  harness.emit(
    event({
      id: "4",
      kind: 48103,
      pubkey: RELAY,
      tags: [["h", "general"]],
    }),
  );
  assert.deepEqual([...harness.snapshots.at(-1)], []);
  harness.dispose();
});

test("clears stale presence on the lease-cadence liveness refresh", async () => {
  const harness = runtimeHarness([
    event({ id: "1", kind: 48100 }),
    participantEvent({
      id: "2",
      kind: 48101,
      admissionId: "desktop",
      rosterRevision: 1,
    }),
  ]);
  await settle();

  assert.equal(harness.snapshots.at(-1).has(ALICE), false);
  assert.equal(harness.snapshots.at(-1).has(BOB), true);
  assert.equal(harness.livenessDelay(), 10_000);

  harness.setLiveSessions([]);
  harness.refreshLiveness();
  await settle();

  assert.deepEqual([...harness.snapshots.at(-1)], []);
  harness.dispose();
});

test("globally orders pending starts with persisted same-second joins", async () => {
  let liveHandler;
  let resolveHistory;
  const snapshots = [];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["general"],
    subscribeLive: async (_filter, handler) => {
      liveHandler = handler;
      return () => {};
    },
    fetchEvents: async (filter) => {
      if (filter.kinds?.includes(48104)) return [livenessEvent()];
      return new Promise((resolve) => {
        resolveHistory = resolve;
      });
    },
    subscribeToReconnects: () => () => {},
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setLivenessTimer: (callback) => callback,
    clearLivenessTimer: () => {},
  });
  await settle();

  liveHandler(event({ id: "a", kind: 48100, createdAt: 10 }));
  resolveHistory([
    event({ id: "z", kind: 48100, createdAt: 10 }),
    participantEvent({
      id: "join",
      kind: 48101,
      admissionId: "desktop",
      rosterRevision: 1,
      createdAt: 10,
    }),
  ]);
  await settle();

  assert.equal(snapshots.at(-1).has(BOB), true);
  dispose();
});

test("repeated same-generation liveness preserves admissions", async () => {
  const harness = runtimeHarness([
    event({ id: "1", kind: 48100 }),
    participantEvent({
      id: "2",
      kind: 48101,
      admissionId: "desktop",
      rosterRevision: 1,
    }),
  ]);
  await settle();

  harness.refreshLiveness();
  await settle();

  assert.equal(harness.snapshots.at(-1).has(BOB), true);
  harness.dispose();
});

test("changed liveness generation retires equal-revision admissions", async () => {
  const harness = runtimeHarness([
    event({ id: "1", kind: 48100 }),
    participantEvent({
      id: "2",
      kind: 48101,
      admissionId: "desktop",
      rosterRevision: 1,
    }),
  ]);
  await settle();

  harness.setLiveGeneration("2");
  harness.refreshLiveness();
  await settle();

  assert.equal(harness.snapshots.at(-1).has(BOB), false);
  harness.dispose();
});

test("live generation change retires stale admissions immediately", async () => {
  const harness = runtimeHarness([
    event({ id: "start", kind: 48100, generation: "1" }),
    participantEvent({
      id: "old",
      kind: 48101,
      admissionId: "old-admission",
      rosterRevision: 1,
      generation: "1",
    }),
  ]);
  await settle();
  assert.equal(harness.snapshots.at(-1).has(BOB), true);

  harness.emit(
    participantEvent({
      id: "new",
      kind: 48101,
      admissionId: "new-admission",
      rosterRevision: 1,
      generation: "2",
      tags: [["p", CAROL]],
    }),
  );

  assert.equal(harness.snapshots.at(-1).has(BOB), false);
  assert.equal(harness.snapshots.at(-1).has(CAROL), true);

  harness.setLiveSessions([]);
  harness.refreshLiveness();
  await settle();
  assert.deepEqual([...harness.snapshots.at(-1)], []);
  harness.dispose();
});

test("keeps a live session added while an older liveness refresh is in flight", async () => {
  let liveHandler;
  let livenessTimer;
  let resolveRefresh;
  let livenessRequests = 0;
  const queriedSessions = [];
  const snapshots = [];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["general"],
    subscribeLive: async (_filter, handler) => {
      liveHandler = handler;
      return () => {};
    },
    fetchEvents: async (filter) => {
      if (!filter.kinds?.includes(48104)) {
        return [event({ id: "1", kind: 48100 })];
      }
      livenessRequests += 1;
      queriedSessions.push([...filter["#d"]]);
      if (livenessRequests === 1) return [livenessEvent()];
      if (livenessRequests === 2) {
        return new Promise((resolve) => {
          resolveRefresh = resolve;
        });
      }
      return filter["#d"].map((session) =>
        livenessEvent(session, session === "new-room" ? "pending" : "1"),
      );
    },
    subscribeToReconnects: () => () => {},
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setLivenessTimer: (callback) => {
      livenessTimer = callback;
      return callback;
    },
    clearLivenessTimer: () => {
      livenessTimer = undefined;
    },
  });
  await settle();

  livenessTimer();
  await settle();
  liveHandler(
    event({
      id: "new-start",
      kind: 48100,
      pubkey: CAROL,
      createdAt: 2,
      session: "new-room",
    }),
  );
  liveHandler(
    participantEvent({
      id: "new-join",
      kind: 48101,
      tags: [["p", CAROL]],
      admissionId: "new-room-admission",
      rosterRevision: 1,
      createdAt: 3,
      session: "new-room",
    }),
  );
  assert.equal(snapshots.at(-1).has(CAROL), true);

  resolveRefresh([livenessEvent()]);
  await settle();
  assert.equal(snapshots.at(-1).has(CAROL), true);
  assert.equal(typeof livenessTimer, "function");

  livenessTimer();
  await settle();
  assert.deepEqual(queriedSessions.at(-1), ["room", "new-room"]);
  assert.equal(snapshots.at(-1).has(CAROL), true);
  dispose();
});

test("preserves a newer generation learned during an older liveness refresh", async () => {
  let liveHandler;
  let livenessTimer;
  let resolveRefresh;
  let livenessRequests = 0;
  const snapshots = [];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["general"],
    subscribeLive: async (_filter, handler) => {
      liveHandler = handler;
      return () => {};
    },
    fetchEvents: async (filter) => {
      if (!filter.kinds?.includes(48104)) {
        return [
          event({ id: "start", kind: 48100, createdAt: 1 }),
          participantEvent({
            id: "old-join",
            kind: 48101,
            admissionId: "old-admission",
            rosterRevision: 1,
            generation: "1",
            createdAt: 2,
          }),
        ];
      }
      livenessRequests += 1;
      if (livenessRequests === 1) {
        return [livenessEvent("room", "1")];
      }
      return new Promise((resolve) => {
        resolveRefresh = resolve;
      });
    },
    subscribeToReconnects: () => () => {},
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setLivenessTimer: (callback) => {
      livenessTimer = callback;
      return callback;
    },
    clearLivenessTimer: () => {
      livenessTimer = undefined;
    },
  });
  await settle();
  assert.equal(snapshots.at(-1).has(BOB), true);

  livenessTimer();
  await settle();
  liveHandler(
    participantEvent({
      id: "new-join",
      kind: 48101,
      tags: [["p", CAROL]],
      admissionId: "new-admission",
      rosterRevision: 1,
      generation: "2",
      createdAt: 3,
    }),
  );
  assert.equal(snapshots.at(-1).has(CAROL), true);

  resolveRefresh([livenessEvent("room", "1")]);
  await settle();
  assert.equal(snapshots.at(-1).has(CAROL), true);
  dispose();
});

test("preserves a changed generation when stale liveness omits the session", async () => {
  let liveHandler;
  let livenessTimer;
  let resolveRefresh;
  let livenessRequests = 0;
  const snapshots = [];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["general"],
    subscribeLive: async (_filter, handler) => {
      liveHandler = handler;
      return () => {};
    },
    fetchEvents: async (filter) => {
      if (!filter.kinds?.includes(48104)) {
        return [
          event({ id: "start", kind: 48100, createdAt: 1 }),
          participantEvent({
            id: "old-join",
            kind: 48101,
            admissionId: "old-admission",
            rosterRevision: 1,
            generation: "1",
            createdAt: 2,
          }),
        ];
      }
      livenessRequests += 1;
      if (livenessRequests === 1) {
        return [livenessEvent("room", "1")];
      }
      return new Promise((resolve) => {
        resolveRefresh = resolve;
      });
    },
    subscribeToReconnects: () => () => {},
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setLivenessTimer: (callback) => {
      livenessTimer = callback;
      return callback;
    },
    clearLivenessTimer: () => {
      livenessTimer = undefined;
    },
  });
  await settle();
  assert.equal(snapshots.at(-1).has(BOB), true);

  livenessTimer();
  await settle();
  liveHandler(
    participantEvent({
      id: "new-join",
      kind: 48101,
      tags: [["p", CAROL]],
      admissionId: "new-admission",
      rosterRevision: 1,
      generation: "2",
      createdAt: 3,
    }),
  );
  assert.equal(snapshots.at(-1).has(CAROL), true);

  resolveRefresh([]);
  await settle();
  assert.equal(snapshots.at(-1).has(CAROL), true);
  dispose();
});

test("replays an opaque-generation join once liveness establishes it", async () => {
  const generation1 = "11111111-1111-4111-8111-111111111111";
  const generation2 = "22222222-2222-4222-8222-222222222222";
  let liveHandler;
  let livenessTimer;
  let resolveRefresh;
  let livenessRequests = 0;
  const snapshots = [];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["general"],
    subscribeLive: async (_filter, handler) => {
      liveHandler = handler;
      return () => {};
    },
    fetchEvents: async (filter) => {
      if (!filter.kinds?.includes(48104)) {
        return [
          event({ id: "start", kind: 48100, createdAt: 1 }),
          participantEvent({
            id: "old-join",
            kind: 48101,
            admissionId: "old-admission",
            rosterRevision: 1,
            generation: generation1,
            createdAt: 2,
          }),
        ];
      }
      livenessRequests += 1;
      if (livenessRequests === 1) {
        return [livenessEvent("room", generation1)];
      }
      return new Promise((resolve) => {
        resolveRefresh = resolve;
      });
    },
    subscribeToReconnects: () => () => {},
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setLivenessTimer: (callback) => {
      livenessTimer = callback;
      return callback;
    },
    clearLivenessTimer: () => {
      livenessTimer = undefined;
    },
  });
  await settle();
  assert.equal(snapshots.at(-1).has(BOB), true);

  livenessTimer();
  await settle();
  liveHandler(
    participantEvent({
      id: "new-join",
      kind: 48101,
      tags: [["p", CAROL]],
      admissionId: "new-admission",
      rosterRevision: 1,
      generation: generation2,
      createdAt: 3,
    }),
  );
  assert.equal(snapshots.at(-1).has(BOB), true);
  assert.equal(snapshots.at(-1).has(CAROL), false);

  resolveRefresh([livenessEvent("room", generation2)]);
  await settle();
  assert.equal(snapshots.at(-1).has(BOB), false);
  assert.equal(snapshots.at(-1).has(CAROL), true);
  dispose();
});

test("replays an opaque-generation join and leave in causal order", async () => {
  const generation1 = "11111111-1111-4111-8111-111111111111";
  const generation2 = "22222222-2222-4222-8222-222222222222";
  let liveHandler;
  let livenessTimer;
  let resolveRefresh;
  let livenessRequests = 0;
  const snapshots = [];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["general"],
    subscribeLive: async (_filter, handler) => {
      liveHandler = handler;
      return () => {};
    },
    fetchEvents: async (filter) => {
      if (!filter.kinds?.includes(48104)) {
        return [
          event({ id: "start", kind: 48100, createdAt: 1 }),
          participantEvent({
            id: "old-join",
            kind: 48101,
            admissionId: "old-admission",
            rosterRevision: 1,
            generation: generation1,
            createdAt: 2,
          }),
        ];
      }
      livenessRequests += 1;
      if (livenessRequests === 1) {
        return [livenessEvent("room", generation1)];
      }
      return new Promise((resolve) => {
        resolveRefresh = resolve;
      });
    },
    subscribeToReconnects: () => () => {},
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setLivenessTimer: (callback) => {
      livenessTimer = callback;
      return callback;
    },
    clearLivenessTimer: () => {
      livenessTimer = undefined;
    },
  });
  await settle();

  livenessTimer();
  await settle();
  liveHandler(
    participantEvent({
      id: "new-join",
      kind: 48101,
      tags: [["p", CAROL]],
      admissionId: "new-admission",
      rosterRevision: 1,
      generation: generation2,
      createdAt: 3,
    }),
  );
  liveHandler(
    participantEvent({
      id: "new-left",
      kind: 48102,
      tags: [["p", CAROL]],
      admissionId: "new-admission",
      rosterRevision: 2,
      generation: generation2,
      createdAt: 4,
    }),
  );
  assert.equal(snapshots.at(-1).has(CAROL), false);

  resolveRefresh([livenessEvent("room", generation2)]);
  await settle();
  assert.equal(snapshots.at(-1).has(BOB), false);
  assert.equal(snapshots.at(-1).has(CAROL), false);
  dispose();
});

test("replays an opaque-generation end after matching liveness", async () => {
  const generation1 = "11111111-1111-4111-8111-111111111111";
  const generation2 = "22222222-2222-4222-8222-222222222222";
  let liveHandler;
  let livenessTimer;
  let resolveRefresh;
  let livenessRequests = 0;
  const snapshots = [];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["general"],
    subscribeLive: async (_filter, handler) => {
      liveHandler = handler;
      return () => {};
    },
    fetchEvents: async (filter) => {
      if (!filter.kinds?.includes(48104)) {
        return [event({ id: "start", kind: 48100, createdAt: 1 })];
      }
      livenessRequests += 1;
      if (livenessRequests === 1) {
        return [livenessEvent("room", generation1)];
      }
      return new Promise((resolve) => {
        resolveRefresh = resolve;
      });
    },
    subscribeToReconnects: () => () => {},
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setLivenessTimer: (callback) => {
      livenessTimer = callback;
      return callback;
    },
    clearLivenessTimer: () => {
      livenessTimer = undefined;
    },
  });
  await settle();

  livenessTimer();
  await settle();
  liveHandler(
    participantEvent({
      id: "new-join",
      kind: 48101,
      admissionId: "new-admission",
      rosterRevision: 1,
      generation: generation2,
      createdAt: 2,
    }),
  );
  liveHandler(
    event({
      id: "new-end",
      kind: 48103,
      pubkey: RELAY,
      generation: generation2,
      createdAt: 3,
    }),
  );

  resolveRefresh([livenessEvent("room", generation2)]);
  await settle();
  assert.deepEqual([...snapshots.at(-1)], []);
  dispose();
});

test("opaque lifecycle overflow fences an older liveness settlement", async () => {
  const generation1 = "11111111-1111-4111-8111-111111111111";
  const generation2 = "22222222-2222-4222-8222-222222222222";
  let liveHandler;
  let livenessTimer;
  let resolveOldRefresh;
  let retry;
  let livenessRequests = 0;
  const snapshots = [];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["general"],
    subscribeLive: async (_filter, handler) => {
      liveHandler = handler;
      return () => {};
    },
    fetchEvents: async (filter) => {
      if (!filter.kinds?.includes(48104)) {
        return [
          event({ id: "start", kind: 48100, createdAt: 1 }),
          participantEvent({
            id: "old-join",
            kind: 48101,
            admissionId: "old-admission",
            rosterRevision: 1,
            generation: generation1,
            createdAt: 2,
          }),
        ];
      }
      livenessRequests += 1;
      if (livenessRequests === 1 || livenessRequests >= 3) {
        return [livenessEvent("room", generation1)];
      }
      if (livenessRequests === 2) {
        return new Promise((resolve) => {
          resolveOldRefresh = resolve;
        });
      }
      throw new Error(`Unexpected liveness request ${livenessRequests}`);
    },
    subscribeToReconnects: () => () => {},
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setRetryTimer: (callback) => {
      retry = callback;
      return callback;
    },
    clearRetryTimer: () => {
      retry = undefined;
    },
    setLivenessTimer: (callback) => {
      livenessTimer = callback;
      return callback;
    },
    clearLivenessTimer: () => {
      livenessTimer = undefined;
    },
  });
  await settle();
  assert.equal(snapshots.at(-1).has(BOB), true);

  livenessTimer();
  await settle();
  for (let index = 0; index <= 1_000; index += 1) {
    liveHandler(
      participantEvent({
        id: `opaque-${index}`,
        kind: 48101,
        tags: [["p", CAROL]],
        admissionId: `opaque-admission-${index}`,
        rosterRevision: index + 1,
        generation: generation2,
        createdAt: index + 3,
      }),
    );
  }
  assert.deepEqual([...snapshots.at(-1)], []);
  assert.equal(typeof retry, "function");

  retry();
  await settle();
  assert.equal(livenessRequests >= 3, true);
  assert.equal(snapshots.at(-1).has(BOB), true);

  resolveOldRefresh([]);
  await settle();
  assert.equal(snapshots.at(-1).has(BOB), true);
  dispose();
});

test("ignores a rejected liveness request from before completed recovery", async () => {
  let reconnect;
  let livenessTimer;
  let rejectOldRefresh;
  let livenessRequests = 0;
  let errors = 0;
  let retryScheduled = false;
  const snapshots = [];
  const history = [
    event({ id: "start", kind: 48100, createdAt: 1 }),
    participantEvent({
      id: "join",
      kind: 48101,
      admissionId: "admission",
      rosterRevision: 1,
      generation: "1",
      createdAt: 2,
    }),
  ];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["general"],
    subscribeLive: async () => () => {},
    fetchEvents: async (filter) => {
      if (!filter.kinds?.includes(48104)) return history;
      livenessRequests += 1;
      if (livenessRequests === 2) {
        return new Promise((_resolve, reject) => {
          rejectOldRefresh = reject;
        });
      }
      return [livenessEvent()];
    },
    subscribeToReconnects: (listener) => {
      reconnect = listener;
      return () => {};
    },
    onPresence: (participants) => snapshots.push(new Set(participants)),
    onError: () => {
      errors += 1;
    },
    setRetryTimer: (callback) => {
      retryScheduled = true;
      return callback;
    },
    clearRetryTimer: () => {
      retryScheduled = false;
    },
    setLivenessTimer: (callback) => {
      livenessTimer = callback;
      return callback;
    },
    clearLivenessTimer: () => {
      livenessTimer = undefined;
    },
  });
  await settle();
  assert.equal(snapshots.at(-1).has(BOB), true);

  livenessTimer();
  await settle();
  reconnect();
  await settle();
  assert.equal(livenessRequests, 3);
  assert.equal(snapshots.at(-1).has(BOB), true);

  rejectOldRefresh(new Error("obsolete timeout"));
  await settle();
  assert.equal(snapshots.at(-1).has(BOB), true);
  assert.equal(errors, 0);
  assert.equal(retryScheduled, false);
  dispose();
});

test("stale liveness removes unchanged requested sessions and preserves new ones", async () => {
  let liveHandler;
  let livenessTimer;
  let resolveRefresh;
  let livenessRequests = 0;
  const snapshots = [];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["general"],
    subscribeLive: async (_filter, handler) => {
      liveHandler = handler;
      return () => {};
    },
    fetchEvents: async (filter) => {
      if (!filter.kinds?.includes(48104)) {
        return [
          event({ id: "old-start", kind: 48100, session: "old-room" }),
          participantEvent({
            id: "old-join",
            kind: 48101,
            session: "old-room",
            admissionId: "old-admission",
            rosterRevision: 1,
            generation: "1",
          }),
        ];
      }
      livenessRequests += 1;
      if (livenessRequests === 1) {
        return [livenessEvent("old-room", "1")];
      }
      return new Promise((resolve) => {
        resolveRefresh = resolve;
      });
    },
    subscribeToReconnects: () => () => {},
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setLivenessTimer: (callback) => {
      livenessTimer = callback;
      return callback;
    },
    clearLivenessTimer: () => {
      livenessTimer = undefined;
    },
  });
  await settle();
  assert.equal(snapshots.at(-1).has(BOB), true);

  livenessTimer();
  await settle();
  liveHandler(
    event({
      id: "new-start",
      kind: 48100,
      pubkey: CAROL,
      session: "new-room",
    }),
  );
  liveHandler(
    participantEvent({
      id: "new-join",
      kind: 48101,
      tags: [["p", CAROL]],
      session: "new-room",
      admissionId: "new-admission",
      rosterRevision: 1,
      generation: "1",
    }),
  );

  resolveRefresh([]);
  await settle();
  assert.equal(snapshots.at(-1).has(BOB), false);
  assert.equal(snapshots.at(-1).has(CAROL), true);
  dispose();
});

test("retains parent routing for sessions accepted from hydration overlap", async () => {
  let liveHandler;
  let livenessTimer;
  let resolveHydrationLiveness;
  let livenessRequests = 0;
  const livenessFilters = [];
  const snapshots = [];
  const history = [
    event({
      id: "start-a",
      kind: 48100,
      session: "room-a",
      tags: [["h", "general"]],
      createdAt: 1,
    }),
    participantEvent({
      id: "join-a",
      kind: 48101,
      session: "room-a",
      admissionId: "admission-a",
      rosterRevision: 1,
      generation: "1",
      tags: [
        ["h", "general"],
        ["p", BOB],
      ],
      createdAt: 2,
    }),
  ];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["design", "general"],
    subscribeLive: async (_filter, handler) => {
      liveHandler = handler;
      return () => {};
    },
    fetchEvents: async (filter) => {
      if (!filter.kinds?.includes(48104)) return history;
      livenessRequests += 1;
      livenessFilters.push(filter);
      if (livenessRequests === 1) {
        return new Promise((resolve) => {
          resolveHydrationLiveness = resolve;
        });
      }
      return filter["#d"].map((session) => livenessEvent(session));
    },
    subscribeToReconnects: () => () => {},
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setLivenessTimer: (callback) => {
      livenessTimer = callback;
      return callback;
    },
    clearLivenessTimer: () => {
      livenessTimer = undefined;
    },
  });
  await settle();

  liveHandler(
    event({
      id: "start-b",
      kind: 48100,
      pubkey: CAROL,
      session: "room-b",
      tags: [["h", "design"]],
      createdAt: 3,
    }),
  );
  liveHandler(
    participantEvent({
      id: "join-b",
      kind: 48101,
      session: "room-b",
      admissionId: "admission-b",
      rosterRevision: 1,
      generation: "1",
      tags: [
        ["h", "design"],
        ["p", CAROL],
      ],
      createdAt: 4,
    }),
  );
  resolveHydrationLiveness([livenessEvent("room-a")]);
  await settle();
  assert.equal(snapshots.at(-1).has(CAROL), true);

  livenessTimer();
  await settle();
  assert.equal(livenessRequests, 2);
  assert.deepEqual(livenessFilters[1]["#h"], ["general", "design"]);
  assert.deepEqual(livenessFilters[1]["#d"], ["room-a", "room-b"]);
  assert.equal(snapshots.at(-1).has(CAROL), true);
  dispose();
});

test("bounds parent-routed liveness requests while preserving every session", async () => {
  const channels = Array.from(
    { length: 257 },
    (_, index) => `channel-${index}`,
  );
  const history = Array.from({ length: 257 }, (_, index) =>
    event({
      id: `start-${index}`,
      kind: 48100,
      session: `room-${index}`,
      tags: [["h", channels[index]]],
      createdAt: index + 1,
    }),
  );
  let activeRequests = 0;
  let peakRequests = 0;
  let totalRequests = 0;
  const filters = [];
  let hydrationComplete;
  const hydrated = new Promise((resolve) => {
    hydrationComplete = resolve;
  });
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: channels,
    subscribeLive: async () => () => {},
    fetchEvents: async (filter) => {
      if (!filter.kinds?.includes(48104)) return history;
      filters.push(filter);
      totalRequests += 1;
      activeRequests += 1;
      peakRequests = Math.max(peakRequests, activeRequests);
      await new Promise((resolve) => setImmediate(resolve));
      activeRequests -= 1;
      return filter["#d"].map((session) => livenessEvent(session));
    },
    subscribeToReconnects: () => () => {},
    onPresence: () => hydrationComplete(),
    setLivenessTimer: (callback) => callback,
    clearLivenessTimer: () => {},
  });

  await hydrated;
  assert.equal(totalRequests, 3);
  assert.equal(peakRequests, 3);
  assert.equal(
    filters.every((filter) => filter["#h"].length <= 128),
    true,
  );
  assert.equal(
    filters.every((filter) => filter["#d"].length <= 128),
    true,
  );
  assert.deepEqual(
    new Set(filters.flatMap((filter) => filter["#d"])),
    new Set(
      history.map((item) => JSON.parse(item.content).ephemeral_channel_id),
    ),
  );
  dispose();
});

test("packs parent-routed liveness linearly without widening full session chunks", async () => {
  const channels = ["channel-a", "channel-b"];
  const history = [
    ...Array.from({ length: 128 }, (_, index) =>
      event({
        id: `start-a-${index}`,
        kind: 48100,
        session: `room-a-${index}`,
        tags: [["h", channels[0]]],
        createdAt: index + 1,
      }),
    ),
    event({
      id: "start-b",
      kind: 48100,
      session: "room-b",
      tags: [["h", channels[1]]],
      createdAt: 129,
    }),
  ];
  const filters = [];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: channels,
    subscribeLive: async () => () => {},
    fetchEvents: async (filter) => {
      if (!filter.kinds?.includes(48104)) return history;
      filters.push(filter);
      return [];
    },
    subscribeToReconnects: () => () => {},
    onPresence: () => {},
    setLivenessTimer: (callback) => callback,
    clearLivenessTimer: () => {},
  });

  await settle();
  assert.deepEqual(
    filters.map((filter) => [filter["#h"].length, filter["#d"].length]),
    [
      [1, 128],
      [1, 1],
    ],
  );
  dispose();
});

test("drains failed liveness workers before recovery starts a replacement batch", async () => {
  const channels = Array.from(
    { length: 513 },
    (_, index) => `channel-${index}`,
  );
  const history = channels.map((channel, index) =>
    event({
      id: `start-${index}`,
      kind: 48100,
      session: `room-${index}`,
      tags: [["h", channel]],
      createdAt: index + 1,
    }),
  );
  let retry;
  let activeRequests = 0;
  let peakRequests = 0;
  let requestCount = 0;
  let failureDelivered = false;
  const held = [];
  const snapshots = [];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: channels,
    subscribeLive: async () => () => {},
    fetchEvents: async (filter) => {
      if (!filter.kinds?.includes(48104)) {
        return history.filter((item) =>
          item.tags.some(
            (tag) => tag[0] === "h" && filter["#h"]?.includes(tag[1]),
          ),
        );
      }
      requestCount += 1;
      activeRequests += 1;
      peakRequests = Math.max(peakRequests, activeRequests);
      if (!failureDelivered && requestCount === 4) {
        failureDelivered = true;
        activeRequests -= 1;
        throw new Error("temporary timeout");
      }
      if (!failureDelivered) {
        return new Promise((resolve) => held.push({ filter, resolve }));
      }
      activeRequests -= 1;
      return filter["#d"].map((session) => livenessEvent(session));
    },
    subscribeToReconnects: () => () => {},
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setRetryTimer: (callback) => {
      retry = callback;
      return callback;
    },
    clearRetryTimer: () => {
      retry = undefined;
    },
    setLivenessTimer: (callback) => callback,
    clearLivenessTimer: () => {},
  });

  await settle();
  assert.equal(requestCount, 4);
  assert.equal(retry, undefined);
  for (const request of held) {
    activeRequests -= 1;
    request.resolve(
      request.filter["#d"].map((session) => livenessEvent(session)),
    );
  }
  await settle();
  assert.equal(typeof retry, "function");

  retry();
  await settle();
  assert.equal(peakRequests, 4);
  assert.equal(requestCount, 9);
  assert.equal(snapshots.at(-1).size, 0);
  dispose();
});

test("stops dispatching queued liveness requests after disposal", async () => {
  const channels = Array.from(
    { length: 513 },
    (_, index) => `channel-${index}`,
  );
  const history = channels.map((channel, index) =>
    event({
      id: `start-${index}`,
      kind: 48100,
      session: `room-${index}`,
      tags: [["h", channel]],
      createdAt: index + 1,
    }),
  );
  const held = [];
  let requestCount = 0;
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: channels,
    subscribeLive: async () => () => {},
    fetchEvents: async (filter) => {
      if (!filter.kinds?.includes(48104)) {
        return history.filter((item) =>
          item.tags.some(
            (tag) => tag[0] === "h" && filter["#h"]?.includes(tag[1]),
          ),
        );
      }
      requestCount += 1;
      return new Promise((resolve) => held.push({ filter, resolve }));
    },
    subscribeToReconnects: () => () => {},
    onPresence: () => {},
    setRetryTimer: (callback) => callback,
    clearRetryTimer: () => {},
    setLivenessTimer: (callback) => callback,
    clearLivenessTimer: () => {},
  });

  await settle();
  assert.equal(requestCount, 4);
  dispose();
  for (const request of held) {
    request.resolve(
      request.filter["#d"].map((session) => livenessEvent(session)),
    );
  }
  await settle();
  assert.equal(requestCount, 4);
});

test("retries hydration when one bounded liveness request fails", async () => {
  let retry;
  let livenessRequests = 0;
  const snapshots = [];
  const history = [
    event({ id: "start", kind: 48100 }),
    participantEvent({
      id: "join",
      kind: 48101,
      admissionId: "admission",
      rosterRevision: 1,
    }),
  ];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: Array.from({ length: 129 }, (_, index) => `channel-${index}`),
    subscribeLive: async () => () => {},
    fetchEvents: async (filter) => {
      if (!filter.kinds?.includes(48104)) return history;
      livenessRequests += 1;
      if (livenessRequests === 1) throw new Error("temporary timeout");
      return [livenessEvent()];
    },
    subscribeToReconnects: () => () => {},
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setRetryTimer: (callback) => {
      retry = callback;
      return callback;
    },
    clearRetryTimer: () => {
      retry = undefined;
    },
    setLivenessTimer: (callback) => callback,
    clearLivenessTimer: () => {},
  });

  await settle();
  assert.deepEqual([...snapshots.at(-1)], []);
  assert.equal(typeof retry, "function");

  retry();
  await settle();
  assert.equal(livenessRequests, 2);
  assert.equal(snapshots.at(-1).has(BOB), true);
  dispose();
});

test("retries a failed hydration and tears down every recovery path", async () => {
  let attempts = 0;
  let retry;
  let reconnect;
  let liveDisposed = false;
  let reconnectDisposed = false;
  const snapshots = [];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["general"],
    subscribeLive: async () => () => {
      liveDisposed = true;
    },
    fetchEvents: async (filter) => {
      if (filter.kinds?.includes(48104)) return [livenessEvent()];
      attempts += 1;
      if (attempts === 1) throw new Error("temporary timeout");
      return [event({ id: "1", kind: 48100 })];
    },
    subscribeToReconnects: (listener) => {
      reconnect = listener;
      return () => {
        reconnectDisposed = true;
      };
    },
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setRetryTimer: (callback) => {
      retry = callback;
      return callback;
    },
    clearRetryTimer: () => {
      retry = undefined;
    },
    setLivenessTimer: (callback) => callback,
    clearLivenessTimer: () => {},
  });

  await settle();
  assert.deepEqual([...snapshots.at(-1)], []);
  assert.equal(typeof retry, "function");

  retry();
  await settle();
  assert.equal(snapshots.at(-1).has(ALICE), false);

  dispose();
  assert.equal(liveDisposed, true);
  assert.equal(reconnectDisposed, true);
  reconnect();
  await settle();
  assert.equal(attempts, 2);
});

test("does not let rejected lifecycle events reopen a dead room", async () => {
  let liveHandler;
  let resolveHistory;
  const snapshots = [];
  const history = [
    event({ id: "1", kind: 48100 }),
    participantEvent({
      id: "2",
      kind: 48101,
      admissionId: "old",
      rosterRevision: 1,
    }),
  ];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["general"],
    subscribeLive: async (_filter, handler) => {
      liveHandler = handler;
      return () => {};
    },
    fetchEvents: async (filter) => {
      if (filter.kinds?.includes(48104)) return [];
      return new Promise((resolve) => {
        resolveHistory = resolve;
      });
    },
    subscribeToReconnects: () => () => {},
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setLivenessTimer: (callback) => callback,
    clearLivenessTimer: () => {},
  });
  await settle();

  const forgedJoin = event({
    id: "3",
    kind: 48101,
    pubkey: ALICE,
    tags: [["p", BOB]],
    admissionId: "forged",
    rosterRevision: 2,
  });
  liveHandler(forgedJoin);
  resolveHistory(history);
  await settle();
  assert.deepEqual([...snapshots.at(-1)], []);

  liveHandler(forgedJoin);
  assert.deepEqual([...snapshots.at(-1)], []);
  dispose();
});

test("fails closed when persisted lifecycle has no authoritative live room", async () => {
  const snapshots = [];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["general"],
    subscribeLive: async () => () => {},
    fetchEvents: async (filter) =>
      filter.kinds?.includes(48104)
        ? []
        : [
            event({ id: "1", kind: 48100 }),
            participantEvent({
              id: "2",
              kind: 48101,
              admissionId: "before-restart",
              rosterRevision: 1,
            }),
          ],
    subscribeToReconnects: () => () => {},
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setLivenessTimer: (callback) => callback,
    clearLivenessTimer: () => {},
  });

  await settle();
  assert.deepEqual([...snapshots.at(-1)], []);
  dispose();
});

test("does not activate a start until liveness or an authenticated join", async () => {
  let liveHandler;
  const snapshots = [];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["general"],
    subscribeLive: async (_filter, handler) => {
      liveHandler = handler;
      return () => {};
    },
    fetchEvents: async () => [],
    subscribeToReconnects: () => () => {},
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setLivenessTimer: (callback) => callback,
    clearLivenessTimer: () => {},
  });
  await settle();

  liveHandler(event({ id: "start", kind: 48100, session: "new-room" }));
  assert.equal(snapshots.at(-1).has(ALICE), false);
  liveHandler(
    participantEvent({
      id: "join",
      kind: 48101,
      session: "new-room",
      admissionId: "admission",
      rosterRevision: 1,
    }),
  );
  assert.equal(snapshots.at(-1).has(BOB), true);
  dispose();
});

test("liveness refresh clears equal-revision admissions across generations", async () => {
  let livenessTimer;
  let generation = "1";
  const snapshots = [];
  const history = [
    event({ id: "start", kind: 48100, createdAt: 1 }),
    participantEvent({
      id: "old",
      kind: 48101,
      admissionId: "old-admission",
      rosterRevision: 1,
      createdAt: 2,
    }),
  ];
  const dispose = startHuddlePresenceRuntime({
    relaySelfPubkey: RELAY,
    channelIds: ["general"],
    subscribeLive: async () => () => {},
    fetchEvents: async (filter) =>
      filter.kinds?.includes(48104)
        ? [livenessEvent("room", generation)]
        : history,
    subscribeToReconnects: () => () => {},
    onPresence: (participants) => snapshots.push(new Set(participants)),
    setLivenessTimer: (callback) => {
      livenessTimer = callback;
      return callback;
    },
    clearLivenessTimer: () => {
      livenessTimer = undefined;
    },
  });
  await settle();
  assert.equal(snapshots.at(-1).has(BOB), true);

  generation = "2";
  livenessTimer();
  await settle();
  assert.equal(snapshots.at(-1).has(BOB), false);
  dispose();
});
