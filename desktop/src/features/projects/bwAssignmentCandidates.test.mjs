import assert from "node:assert/strict";
import test from "node:test";
import {
  buildBwAssignmentCandidates,
  filterBwAssignmentCandidates,
  resolveBwAssignmentSelection,
} from "./bwAssignmentCandidates.ts";

const AGENT = "a".repeat(64);
const MEMBER = "b".repeat(64);
const BOTH = "c".repeat(64);
const UNNAMED = "d".repeat(64);

test("relay agents and channel members merge into one deduplicated, name-sorted list", () => {
  const candidates = buildBwAssignmentCandidates({
    relayAgents: [{ pubkey: AGENT, displayName: "Zed Agent" }],
    members: [{ pubkey: MEMBER, displayName: "Alice", isAgent: false }],
    profiles: {},
  });

  assert.deepEqual(
    candidates.map((c) => c.displayName),
    ["Alice", "Zed Agent"],
  );
  assert.equal(candidates[1].isAgent, true);
  assert.equal(candidates[0].isAgent, false);
});

test("a pubkey present in both sources is deduplicated and keeps its agent flag", () => {
  const candidates = buildBwAssignmentCandidates({
    relayAgents: [{ pubkey: BOTH, displayName: "Bot" }],
    members: [{ pubkey: BOTH, displayName: "Bot", isAgent: false }],
    profiles: {},
  });

  assert.equal(candidates.length, 1);
  assert.equal(candidates[0].isAgent, true);
});

test("a profile fills in the name/avatar when the source carries none", () => {
  const candidates = buildBwAssignmentCandidates({
    relayAgents: [],
    members: [{ pubkey: MEMBER, displayName: null, isAgent: false }],
    profiles: {
      [MEMBER]: {
        avatarUrl: "https://example.test/a.png",
        displayName: "Resolved Name",
        isAgent: false,
        name: null,
        nip05Handle: null,
        ownerPubkey: null,
      },
    },
  });

  assert.equal(candidates[0].displayName, "Resolved Name");
  assert.equal(candidates[0].avatarUrl, "https://example.test/a.png");
});

test("a candidate with no name anywhere falls back to its truncated pubkey, never hidden", () => {
  const candidates = buildBwAssignmentCandidates({
    relayAgents: [],
    members: [{ pubkey: UNNAMED, displayName: null }],
    profiles: {},
  });

  assert.equal(candidates.length, 1);
  assert.equal(
    candidates[0].displayName,
    `${UNNAMED.slice(0, 8)}…${UNNAMED.slice(-4)}`,
  );
});

test("filtering is a case-insensitive substring match on the resolved display name", () => {
  const candidates = buildBwAssignmentCandidates({
    relayAgents: [{ pubkey: AGENT, displayName: "Zed Agent" }],
    members: [{ pubkey: MEMBER, displayName: "Alice" }],
    profiles: {},
  });

  assert.deepEqual(
    filterBwAssignmentCandidates(candidates, "ali").map((c) => c.displayName),
    ["Alice"],
  );
  assert.deepEqual(
    filterBwAssignmentCandidates(candidates, "").map((c) => c.displayName),
    ["Alice", "Zed Agent"],
  );
  assert.deepEqual(filterBwAssignmentCandidates(candidates, "zzz"), []);
});

test("a picker selection resolves to the exact normalized pubkey submitBwAssignment sends as delegate", () => {
  assert.equal(
    resolveBwAssignmentSelection({ pubkey: AGENT.toUpperCase() }),
    AGENT,
  );
  assert.equal(resolveBwAssignmentSelection({}), null);
});
