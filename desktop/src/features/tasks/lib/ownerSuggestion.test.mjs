import assert from "node:assert/strict";
import test from "node:test";

import {
  baselineOwnerOrder,
  suggestTaskOwners,
} from "./ownerSuggestion.ts";

const ALICE = "a".repeat(64);
const BOB = "b".repeat(64);
const CAROL = "c".repeat(64);
const RIPPER = "d".repeat(64);
const DOZER = "e".repeat(64);

function task(overrides = {}) {
  return {
    id: "task-1",
    channelId: "chan-1",
    title: "Ship the relay migration",
    status: "open",
    assignee: null,
    createdBy: null,
    updatedAt: 1,
    ...overrides,
  };
}

function candidate(overrides = {}) {
  return {
    kind: "identity",
    displayName: "Someone",
    isAgent: false,
    isMember: true,
    pubkey: OTHER,
    ...overrides,
  };
}

const OTHER = ALICE;

const ROSTER = [
  candidate({ displayName: "Alice", pubkey: ALICE, isMember: true }),
  candidate({ displayName: "Bob", pubkey: BOB, isMember: true }),
  candidate({ displayName: "Carol", pubkey: CAROL, isMember: true }),
  candidate({
    displayName: "Ripper",
    pubkey: RIPPER,
    isMember: false,
    isAgent: true,
    isActiveAgent: true,
    kind: "persona",
  }),
  candidate({
    displayName: "Dozer",
    pubkey: DOZER,
    isMember: false,
    isAgent: true,
    kind: "identity",
  }),
];

const labels = (out) => out.map((s) => s.label);
const codes = (out) => out.map((s) => s.reasons.map((r) => r.code));

// ---------------------------------------------------------------------------
// K3 — the live kill criterion. REG-16's signals MUST change the order vs the
// bare upstream tiering, otherwise this is a UI relabel of existing groupRank.
// ---------------------------------------------------------------------------
test("K3: own signals reorder vs bare upstream groupRank baseline", () => {
  const baseline = baselineOwnerOrder(ROSTER);
  const ranked = suggestTaskOwners(
    task({ title: "Ripper should ship the relay migration", createdBy: ALICE }),
    ROSTER,
    {
      recentParticipantPubkeys: [BOB, CAROL],
      openTaskCountByPubkey: new Map([[CAROL, 3]]),
      limit: 5,
    },
  );
  const got = ranked.map((s) => s.pubkey);

  assert.equal(baseline[0], ALICE, "baseline leads with insertion order");
  assert.notDeepEqual(got, baseline, "K3 would fire if these matched");
  // Mentioned-in-task wins outright despite being a non-member agent.
  assert.equal(got[0], RIPPER);
  // Author is demoted below plain members.
  assert.ok(got.indexOf(ALICE) > got.indexOf(BOB));
});

test("mentioned candidate outranks everyone, with the reason code", () => {
  const out = suggestTaskOwners(
    task({ title: "Carol owns the write-policy backfill" }),
    ROSTER,
  );
  assert.equal(out[0].label, "Carol");
  assert.ok(out[0].reasons.some((r) => r.code === "mentioned-in-task"));
});

// --- Edge cases -------------------------------------------------------------

test("already-assigned task yields no suggestions", () => {
  const out = suggestTaskOwners(task({ assignee: BOB }), ROSTER);
  assert.deepEqual(out, []);
});

test("empty candidate set renders nothing, does not throw", () => {
  assert.deepEqual(suggestTaskOwners(task(), []), []);
});

test("unknown status degrades, never filters the task out", () => {
  const out = suggestTaskOwners(task({ status: "wontfix-maybe" }), ROSTER);
  assert.ok(out.length > 0);
});

test("null displayName survives via the pubkey branch", () => {
  const anon = candidate({ displayName: null, pubkey: DOZER, isMember: true });
  const out = suggestTaskOwners(task(), [anon]);
  assert.equal(out.length, 1);
  assert.equal(out[0].pubkey, DOZER);
  assert.ok(out[0].label.length > 0, "falls back to truncated pubkey");
});

test("substring must not false-match a shorter name", () => {
  const al = candidate({ displayName: "Al", pubkey: BOB });
  const out = suggestTaskOwners(task({ title: "Alice reviews this" }), [al]);
  assert.ok(
    !out[0].reasons.some((r) => r.code === "mentioned-in-task"),
    "'Al' must not match inside 'Alice'",
  );
});

test("raw pubkey in the title counts as a mention", () => {
  const out = suggestTaskOwners(
    task({ title: `assign to ${DOZER}` }),
    ROSTER,
    { limit: 5 },
  );
  assert.equal(out[0].pubkey, DOZER);
});

test("recency is ordered: earlier in the list ranks higher", () => {
  const out = suggestTaskOwners(task(), ROSTER, {
    recentParticipantPubkeys: [CAROL, BOB],
    limit: 5,
  });
  assert.ok(out.findIndex((s) => s.pubkey === CAROL) < out.findIndex((s) => s.pubkey === BOB));
  const carol = out.find((s) => s.pubkey === CAROL);
  assert.deepEqual(
    carol.reasons.find((r) => r.code === "recent-participant").params,
    { rank: 1 },
  );
});

test("workload demotes a loaded candidate below an idle one", () => {
  const out = suggestTaskOwners(task(), ROSTER, {
    openTaskCountByPubkey: new Map([[ALICE, 9]]),
    limit: 5,
  });
  assert.ok(out.findIndex((s) => s.pubkey === ALICE) > 0);
});

test("limit is honored and defaults to 3", () => {
  assert.equal(suggestTaskOwners(task(), ROSTER).length, 3);
  assert.equal(suggestTaskOwners(task(), ROSTER, { limit: 1 }).length, 1);
  assert.equal(suggestTaskOwners(task(), ROSTER, { limit: 99 }).length, 5);
});

test("reason codes stay within the closed enum", () => {
  const allowed = new Set([
    "mentioned-in-task",
    "channel-member",
    "recent-participant",
    "agent-capability",
    "task-author",
    "light-workload",
  ]);
  const out = suggestTaskOwners(task({ createdBy: ALICE }), ROSTER, {
    recentParticipantPubkeys: [BOB],
    openTaskCountByPubkey: new Map([[CAROL, 2]]),
    limit: 5,
  });
  for (const list of codes(out)) {
    for (const code of list) {
      assert.ok(allowed.has(code), `unexpected reason code ${code}`);
    }
  }
});

test("ranking is deterministic across repeated calls", () => {
  const args = [
    task({ createdBy: ALICE }),
    ROSTER,
    { recentParticipantPubkeys: [BOB], limit: 5 },
  ];
  assert.deepEqual(labels(suggestTaskOwners(...args)), labels(suggestTaskOwners(...args)));
});
