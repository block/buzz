import assert from "node:assert/strict";
import test from "node:test";
import { buildMentionCandidates } from "./buildMentionCandidates.ts";
import { getMentionMemberPubkeys } from "./mentionMemberPubkeys.ts";

const A = "a".repeat(64),
  B = "b".repeat(64),
  VIEWER = "f".repeat(64);
function input(overrides = {}) {
  return {
    activeAgentPubkeys: new Set(),
    activePersonaById: new Map(),
    activePersonas: [],
    canSearchGlobalUsers: true,
    currentPubkey: VIEWER,
    isArchived: () => false,
    managedAgentDirectoryReady: true,
    managedAgentNamesByPubkey: new Map(),
    managedAgentPersonaIds: new Set(),
    managedAgentPersonaIdsByPubkey: new Map(),
    managedAgents: [],
    memberPubkeys: new Set(),
    members: [],
    mentionChannelId: "room",
    mentionableAgentPubkeys: new Set(),
    personaNameByPubkey: new Map(),
    profiles: {},
    relayAgentDirectoryReady: true,
    relayAgentNamesByPubkey: new Map(),
    relayAgents: [],
    userSearchResults: [],
    ...overrides,
  };
}
const relay = (pubkey = A, extra = {}) => ({
  pubkey,
  name: "Scout",
  ownerPubkey: VIEWER,
  status: "online",
  channelIds: ["room"],
  respondTo: "anyone",
  respondToAllowlist: [],
  ...extra,
});
const member = (pubkey = A, extra = {}) => ({
  pubkey,
  displayName: "Scout",
  isAgent: true,
  role: "bot",
  ...extra,
});

for (const role of ["member", "bot"])
  for (const owned of [true, false]) {
    test(`authoritative ${role} roster admits allowed ${owned ? "owned" : "nonowned"} exact agent despite lagging directory membership`, () => {
      const [row] = buildMentionCandidates(
        input({
          members: [member(A, { role })],
          relayAgents: [
            relay(A, { channelIds: [], ownerPubkey: owned ? VIEWER : B }),
          ],
        }),
      );
      assert.equal(row.action, "mention");
      assert.equal(row.isOwned, owned);
      assert.equal(row.isMember, true);
    });
  }
for (const ready of [false, true])
  for (const failed of [false, true]) {
    test(`missing directory evidence is not explicit policy denial (${ready}/${failed})`, () => {
      const [row] = buildMentionCandidates(
        input({
          members: [member()],
          relayAgentDirectoryReady: ready,
          verificationFailed: failed,
        }),
      );
      assert.equal(row.action, failed ? "unavailable" : "checking");
      assert.equal(row.ownerPubkey, null);
      assert.doesNotMatch(row.unavailableReason, /does not permit/);
    });
  }
test("denied member explains policy, denied nonmember omitted, people-search flags cannot bypass exact-key agent classification", () => {
  const data = input({
    relayAgents: [relay(A, { respondTo: "owner-only", ownerPubkey: B })],
    userSearchResults: [
      { pubkey: A, displayName: "Human disguise", isAgent: false },
    ],
  });
  assert.deepEqual(buildMentionCandidates(data), []);
  const [row] = buildMentionCandidates({
    ...data,
    members: [member(A, { isAgent: false, role: "member" })],
  });
  assert.equal(row.action, "unavailable");
  assert.equal(row.isAgent, true);
  assert.match(row.unavailableReason, /does not permit/);
});
test("known removed directory agent cannot reappear as a human; archive overrides every source", () => {
  const data = input({
    knownAgentPubkeys: new Set([A]),
    members: [member(A, { isAgent: false, role: "member" })],
  });
  assert.equal(buildMentionCandidates(data)[0].action, "checking");
  assert.deepEqual(
    buildMentionCandidates({ ...data, isArchived: () => true }),
    [],
  );
});
test("fresh roster removal beats stale directory and channel membership; owned nonmember is Invite, not Mention", () => {
  const membership = getMentionMemberPubkeys(
    "room",
    [{ id: "room", memberPubkeys: [A] }],
    [],
  );
  assert.equal(membership.has(A), false);
  const [row] = buildMentionCandidates(
    input({
      relayAgents: [relay()],
      mentionableAgentPubkeys: new Set([A]),
      memberPubkeys: new Set([A]),
    }),
  );
  assert.equal(row.isMember, false);
  assert.equal(row.action, "invite");
});
test("union keeps same-named people and marks collisions before the cap", () => {
  const rows = buildMentionCandidates(
    input({
      userSearchResults: [A, B].map((pubkey) => ({
        pubkey,
        displayName: "Sam",
        isAgent: false,
      })),
    }),
  );
  assert.equal(rows.length, 2);
  assert.ok(rows.every((row) => row.hasNameCollision));
});
test("relay presence and verified ownership are independent of local stopped state and unverified profile owner", () => {
  const [row] = buildMentionCandidates(
    input({
      members: [member()],
      managedAgents: [{ pubkey: A, name: "Scout", status: "stopped" }],
      relayAgents: [relay()],
      mentionableAgentPubkeys: new Set([A]),
      profiles: { [A]: { ownerPubkey: B } },
    }),
  );
  assert.equal(row.ownerPubkey, VIEWER);
  assert.equal(row.presence, "online");
  assert.equal(row.localLifecycle, "stopped");
  const [stale] = buildMentionCandidates(
    input({
      members: [member()],
      relayAgents: [relay()],
      presenceFresh: false,
    }),
  );
  assert.equal(stale.presence, "unknown");
});
