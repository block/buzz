import assert from "node:assert/strict";
import test from "node:test";

import {
  LEADERSHIP_STALE_MS,
  buildLeadership,
  filterStaleInstances,
  parseLeadershipPayload,
  selectFreshestLeader,
} from "./leadershipHelpers.ts";

// `Date.parse` consumes RFC3339 strings (what the harness emits via
// chrono::Utc::now().to_rfc3339()), so tests build timestamps the same way.
const iso = (epochMs) => new Date(epochMs).toISOString();

function leadershipEvent({ seq, instanceId, isLeader, at, kind, payload }) {
  return {
    seq,
    timestamp: iso(at),
    kind: kind ?? "leadership_status",
    agentIndex: null,
    channelId: null,
    sessionId: null,
    turnId: null,
    payload: payload ?? { type: "leadership_status", instanceId, isLeader },
  };
}

// --- parseLeadershipPayload ---

test("parseLeadershipPayload accepts a well-formed payload", () => {
  const result = parseLeadershipPayload({
    type: "leadership_status",
    instanceId: "123-456",
    isLeader: true,
  });
  assert.deepEqual(result, { instanceId: "123-456", isLeader: true });
});

test("parseLeadershipPayload rejects non-object payloads", () => {
  for (const bad of [null, undefined, "string", 42, true, []]) {
    // Arrays are objects but lack the required string/boolean fields, so they
    // must also be rejected.
    assert.equal(parseLeadershipPayload(bad), null);
  }
});

test("parseLeadershipPayload rejects a missing or non-string instanceId", () => {
  assert.equal(parseLeadershipPayload({ isLeader: true }), null);
  assert.equal(parseLeadershipPayload({ instanceId: 5, isLeader: true }), null);
});

test("parseLeadershipPayload rejects a non-boolean isLeader", () => {
  assert.equal(
    parseLeadershipPayload({ instanceId: "a", isLeader: "yes" }),
    null,
  );
  assert.equal(parseLeadershipPayload({ instanceId: "a" }), null);
});

// --- buildLeadership ---

test("buildLeadership keeps the latest frame per instanceId", () => {
  const events = [
    leadershipEvent({ seq: 1, instanceId: "A", isLeader: true, at: 1000 }),
    leadershipEvent({ seq: 2, instanceId: "B", isLeader: false, at: 1500 }),
    leadershipEvent({ seq: 3, instanceId: "A", isLeader: false, at: 2000 }),
  ];
  const result = buildLeadership(events);
  assert.equal(result.length, 2);
  const a = result.find((i) => i.instanceId === "A");
  assert.deepEqual(a, { instanceId: "A", isLeader: false, lastSeen: 2000 });
});

test("buildLeadership ignores non-leadership events", () => {
  const events = [
    leadershipEvent({ seq: 1, kind: "turn_started", payload: {}, at: 500 }),
    leadershipEvent({ seq: 2, instanceId: "A", isLeader: true, at: 1000 }),
  ];
  const result = buildLeadership(events);
  assert.deepEqual(result, [
    { instanceId: "A", isLeader: true, lastSeen: 1000 },
  ]);
});

test("buildLeadership drops frames that fail the payload guard", () => {
  const events = [
    leadershipEvent({
      seq: 1,
      payload: { instanceId: 5, isLeader: true },
      at: 1000,
    }),
    leadershipEvent({ seq: 2, instanceId: "A", isLeader: true, at: 1500 }),
  ];
  const result = buildLeadership(events);
  assert.deepEqual(result, [
    { instanceId: "A", isLeader: true, lastSeen: 1500 },
  ]);
});

test("buildLeadership drops frames with an unparseable timestamp", () => {
  const bad = leadershipEvent({
    seq: 1,
    instanceId: "A",
    isLeader: true,
    at: 1000,
  });
  bad.timestamp = "not-a-date";
  const good = leadershipEvent({
    seq: 2,
    instanceId: "B",
    isLeader: false,
    at: 1500,
  });
  const result = buildLeadership([bad, good]);
  assert.deepEqual(result, [
    { instanceId: "B", isLeader: false, lastSeen: 1500 },
  ]);
});

test("buildLeadership returns an empty array for no leadership frames", () => {
  assert.deepEqual(buildLeadership([]), []);
});

test("buildLeadership prunes a zombie instance whose frame aged out of the window", () => {
  // Simulates the trimmed event window: the dead instance's frame is gone, so
  // only the survivor's frame remains in the input. The reduction therefore
  // never re-surfaces the zombie instanceId.
  const events = [
    leadershipEvent({
      seq: 9,
      instanceId: "survivor",
      isLeader: true,
      at: 5000,
    }),
  ];
  const result = buildLeadership(events);
  assert.deepEqual(
    result.map((i) => i.instanceId),
    ["survivor"],
  );
});

// --- filterStaleInstances ---

test("filterStaleInstances drops instances past the stale threshold", () => {
  const now = 100_000;
  const fresh = { instanceId: "fresh", isLeader: true, lastSeen: now - 1000 };
  const stale = {
    instanceId: "stale",
    isLeader: false,
    lastSeen: now - LEADERSHIP_STALE_MS - 1,
  };
  const result = filterStaleInstances([fresh, stale], now);
  assert.deepEqual(result, [fresh]);
});

test("filterStaleInstances keeps an instance exactly at the threshold", () => {
  const now = 100_000;
  const boundary = {
    instanceId: "boundary",
    isLeader: true,
    lastSeen: now - LEADERSHIP_STALE_MS,
  };
  assert.deepEqual(filterStaleInstances([boundary], now), [boundary]);
});

test("filterStaleInstances treats a NaN lastSeen as stale", () => {
  const now = 100_000;
  const nan = { instanceId: "nan", isLeader: true, lastSeen: Number.NaN };
  // now - NaN === NaN, and `NaN <= threshold` is false, so it is excluded.
  assert.deepEqual(filterStaleInstances([nan], now), []);
});

// --- selectFreshestLeader ---

test("selectFreshestLeader returns null when no instance leads", () => {
  const instances = [
    { instanceId: "A", isLeader: false, lastSeen: 1000 },
    { instanceId: "B", isLeader: false, lastSeen: 2000 },
  ];
  assert.equal(selectFreshestLeader(instances), null);
});

test("selectFreshestLeader picks the freshest among multiple leaders", () => {
  // The transient two-leader window after a crash: the dead leader's stale
  // isLeader:true and the survivor's fresh one coexist. Freshest wins.
  const dead = { instanceId: "dead", isLeader: true, lastSeen: 1000 };
  const survivor = { instanceId: "survivor", isLeader: true, lastSeen: 9000 };
  assert.equal(selectFreshestLeader([dead, survivor]), survivor);
});

test("selectFreshestLeader ignores non-leaders even if fresher", () => {
  const leader = { instanceId: "leader", isLeader: true, lastSeen: 1000 };
  const followerFresher = {
    instanceId: "follower",
    isLeader: false,
    lastSeen: 9000,
  };
  assert.equal(selectFreshestLeader([leader, followerFresher]), leader);
});

test("selectFreshestLeader returns null for an empty list", () => {
  assert.equal(selectFreshestLeader([]), null);
});
