import assert from "node:assert/strict";
import test from "node:test";

import {
  pickDefaultAgentCandidate,
  rankMentionCandidates,
} from "./mentionRanking.ts";

const CHANNEL_BRAIN_PUBKEY = "1".repeat(64);
const OTHER_BRAIN_PUBKEY = "2".repeat(64);
const CAPTAIN_PUBKEY = "c".repeat(64);
const CARL_PUBKEY = "d".repeat(64);
const LOCAL_CHUCK_PUBKEY = "e".repeat(64);

function candidate(overrides = {}) {
  return {
    kind: "identity",
    displayName: "Brain",
    isAgent: false,
    isMember: false,
    pubkey: OTHER_BRAIN_PUBKEY,
    ...overrides,
  };
}

function rankedLabels(
  candidates,
  query = "brain",
  activePersonaIds = new Set(),
) {
  return rankMentionCandidates(candidates, query, activePersonaIds).map(
    (item) => item.label,
  );
}

function rankedPubkeys(
  candidates,
  query = "brain",
  activePersonaIds = new Set(),
) {
  return rankMentionCandidates(candidates, query, activePersonaIds).map(
    (item) => item.candidate.pubkey ?? `persona:${item.candidate.personaId}`,
  );
}

test("rankMentionCandidates: alphabetical by display name; no member/local-agent hard tier", () => {
  const persona = candidate({
    kind: "persona",
    displayName: "Brain Persona",
    personaId: "brain-persona",
    pubkey: undefined,
  });
  const remoteAgent = candidate({
    displayName: "Brain Remote",
    isAgent: true,
    pubkey: OTHER_BRAIN_PUBKEY,
  });
  const person = candidate({
    displayName: "Brain Person",
    pubkey: "6".repeat(64),
  });
  const channelMember = candidate({
    displayName: "Brain Member",
    isAgent: true,
    isMember: true,
    pubkey: CHANNEL_BRAIN_PUBKEY,
  });

  assert.deepEqual(
    rankedLabels([persona, remoteAgent, person, channelMember]),
    ["Brain Member", "Brain Person", "Brain Persona", "Brain Remote"],
  );
});

test("rankMentionCandidates: community bots compete alphabetically with members and local agents", () => {
  const captain = candidate({
    displayName: "Captain",
    isAgent: true,
    isMember: false,
    pubkey: CAPTAIN_PUBKEY,
  });
  const carlMember = candidate({
    displayName: "Carl",
    isAgent: false,
    isMember: true,
    pubkey: CARL_PUBKEY,
  });
  const chuckLocal = candidate({
    displayName: "Chuck",
    isAgent: true,
    isMember: true,
    isManagedAgent: true,
    pubkey: LOCAL_CHUCK_PUBKEY,
  });

  assert.deepEqual(rankedLabels([chuckLocal, carlMember, captain], "c"), [
    "Captain",
    "Carl",
    "Chuck",
  ]);
});

test("rankMentionCandidates: exact and prefix quality still outrank weaker matches", () => {
  const wordPrefix = candidate({
    displayName: "The Brain",
    isMember: false,
    pubkey: "3".repeat(64),
  });
  const exact = candidate({
    displayName: "Brain",
    isMember: false,
    pubkey: CHANNEL_BRAIN_PUBKEY,
  });
  const prefix = candidate({
    displayName: "Brainiac",
    isMember: true,
    pubkey: "4".repeat(64),
  });

  assert.deepEqual(rankedPubkeys([wordPrefix, exact, prefix]), [
    CHANNEL_BRAIN_PUBKEY,
    "4".repeat(64),
    "3".repeat(64),
  ]);
});

test("rankMentionCandidates: matching secondary labels participate in ranking", () => {
  const byHandle = candidate({
    displayName: "Acme Bot",
    secondaryLabel: "brain@example.com",
    isMember: true,
    pubkey: CHANNEL_BRAIN_PUBKEY,
  });
  const byName = candidate({
    displayName: "Brain",
    pubkey: OTHER_BRAIN_PUBKEY,
  });

  // Exact display-name match outranks secondary-label prefix, regardless of membership.
  assert.deepEqual(rankedPubkeys([byHandle, byName]), [
    OTHER_BRAIN_PUBKEY,
    CHANNEL_BRAIN_PUBKEY,
  ]);
});

test("rankMentionCandidates: equal match quality sorts alphabetically, not by active persona", () => {
  const activePersonaAgent = candidate({
    displayName: "Brain Zulu",
    isAgent: true,
    personaId: "brain-persona",
    pubkey: "5".repeat(64),
  });
  const remoteAgent = candidate({
    displayName: "Brain Alpha",
    isAgent: true,
    pubkey: OTHER_BRAIN_PUBKEY,
  });

  assert.deepEqual(
    rankedPubkeys(
      [activePersonaAgent, remoteAgent],
      "brain",
      new Set(["brain-persona"]),
    ),
    [OTHER_BRAIN_PUBKEY, "5".repeat(64)],
  );
});

test("rankMentionCandidates: catalog and truncated-pubkey labels are searchable", () => {
  const catalogBot = candidate({
    displayName: "mo",
    isAgent: true,
    isMember: true,
    pubkey: CHANNEL_BRAIN_PUBKEY,
  });
  const unnamedMember = candidate({
    displayName: null,
    isMember: true,
    pubkey: OTHER_BRAIN_PUBKEY,
  });

  assert.deepEqual(rankedPubkeys([catalogBot], "mo"), [CHANNEL_BRAIN_PUBKEY]);
  assert.deepEqual(rankedPubkeys([unnamedMember], ""), [OTHER_BRAIN_PUBKEY]);
  assert.deepEqual(
    rankedPubkeys([unnamedMember], OTHER_BRAIN_PUBKEY.slice(0, 8)),
    [OTHER_BRAIN_PUBKEY],
  );
});

test("rankMentionCandidates: owned teams sort alphabetically with agents on equal match quality", () => {
  const remoteAgent = candidate({
    displayName: "Launch Agent",
    isAgent: true,
  });
  const team = candidate({
    kind: "team",
    displayName: "Launch Team",
    pubkey: undefined,
  });

  assert.deepEqual(
    rankMentionCandidates([remoteAgent, team], "launch").map(
      (item) => item.candidate.kind,
    ),
    ["identity", "team"],
  );
});

test("pickDefaultAgentCandidate: active agents outrank stopped channel members", () => {
  const stoppedMember = candidate({
    displayName: "Ada",
    isActiveAgent: false,
    isAgent: true,
    isMember: true,
    pubkey: CHANNEL_BRAIN_PUBKEY,
  });
  const runningNonMember = candidate({
    displayName: "Bea",
    isActiveAgent: true,
    isAgent: true,
    pubkey: OTHER_BRAIN_PUBKEY,
  });

  assert.equal(
    pickDefaultAgentCandidate([stoppedMember, runningNonMember]),
    runningNonMember,
  );
});

test("pickDefaultAgentCandidate: stable labels break ties instead of roster order", () => {
  const vogue = candidate({
    displayName: "Vogue",
    isActiveAgent: true,
    isAgent: true,
    isMember: true,
    pubkey: OTHER_BRAIN_PUBKEY,
  });
  const morgarita = candidate({
    displayName: "Morgarita",
    isActiveAgent: true,
    isAgent: true,
    isMember: true,
    pubkey: CHANNEL_BRAIN_PUBKEY,
  });

  assert.equal(pickDefaultAgentCandidate([vogue, morgarita]), morgarita);
  assert.equal(pickDefaultAgentCandidate([morgarita, vogue]), morgarita);
});

test("pickDefaultAgentCandidate: runnable personas break otherwise equal ties", () => {
  const plain = candidate({
    displayName: "Zulu",
    isActiveAgent: true,
    isAgent: true,
    pubkey: OTHER_BRAIN_PUBKEY,
  });
  const runnable = candidate({
    displayName: "Zulu 2",
    isActiveAgent: true,
    isAgent: true,
    personaId: "active-persona",
    pubkey: CHANNEL_BRAIN_PUBKEY,
  });

  assert.equal(
    pickDefaultAgentCandidate([plain, runnable], new Set(["active-persona"])),
    runnable,
  );
});

test("pickDefaultAgentCandidate: recent eligible mentions outrank the fallback ranking", () => {
  const stoppedRecentMember = candidate({
    displayName: "Ada",
    isActiveAgent: false,
    isAgent: true,
    isMember: true,
    pubkey: CHANNEL_BRAIN_PUBKEY,
  });
  const runningNonMember = candidate({
    displayName: "Bea",
    isActiveAgent: true,
    isAgent: true,
    pubkey: OTHER_BRAIN_PUBKEY,
  });

  assert.equal(
    pickDefaultAgentCandidate(
      [runningNonMember, stoppedRecentMember],
      new Set(),
      [CHANNEL_BRAIN_PUBKEY],
    ),
    stoppedRecentMember,
  );
});

test("pickDefaultAgentCandidate: skips recent pubkeys that are not eligible candidates", () => {
  const runningAgent = candidate({
    isActiveAgent: true,
    isAgent: true,
    pubkey: OTHER_BRAIN_PUBKEY,
  });

  assert.equal(
    pickDefaultAgentCandidate([runningAgent], new Set(), ["f".repeat(64)]),
    runningAgent,
  );
});

test("pickDefaultAgentCandidate: returns null without an addressable agent", () => {
  assert.equal(pickDefaultAgentCandidate([]), null);
  assert.equal(pickDefaultAgentCandidate([candidate()]), null);
});
