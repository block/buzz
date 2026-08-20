import assert from "node:assert/strict";
import test from "node:test";

import {
  getKeyboardSearchSelection,
  rankUserCandidatesBySearch,
  scoreUserCandidate,
} from "./userCandidateSearch.ts";

function makeUser(overrides = {}) {
  return {
    avatarUrl: null,
    displayName: null,
    isAgent: false,
    nip05Handle: null,
    ownerPubkey: null,
    pubkey: "abcdef1234567890",
    ...overrides,
  };
}

test("scoreUserCandidate ranks display labels before pubkeys", () => {
  const user = makeUser({
    displayName: "Alice Johnson",
    nip05Handle: "alice@example.com",
  });

  assert.equal(
    scoreUserCandidate({ label: "Alice Johnson", query: "ali", user }),
    0,
  );
  assert.equal(
    scoreUserCandidate({ label: "Alice Johnson", query: "joh", user }),
    1,
  );
  assert.equal(
    scoreUserCandidate({ label: "Alice Johnson", query: "ice", user }),
    2,
  );
  assert.equal(
    scoreUserCandidate({ label: "Alice Johnson", query: "abcd", user }),
    3,
  );
  assert.equal(
    scoreUserCandidate({ label: "Alice Johnson", query: "3456", user }),
    4,
  );
});

test("scoreUserCandidate supports agent labels and empty-query defaults", () => {
  const agent = makeUser({ isAgent: true });

  assert.equal(
    scoreUserCandidate({ label: "Build Buddy", query: "agent", user: agent }),
    0,
  );
  assert.equal(
    scoreUserCandidate({ label: "Build Buddy", query: "", user: agent }),
    null,
  );
  assert.equal(
    scoreUserCandidate({
      allowEmptyQuery: true,
      label: "Build Buddy",
      query: "",
      user: agent,
    }),
    0,
  );
});

test("scoreUserCandidate tolerates one name typo as a lower-ranked fallback", () => {
  const user = makeUser({
    displayName: "Alice Johnson",
    nip05Handle: "alice@example.com",
  });

  assert.equal(
    scoreUserCandidate({ label: "Alice Johnson", query: "alcie", user }),
    5,
  );
  assert.equal(
    scoreUserCandidate({ label: "Alice Johnson", query: "alc", user }),
    null,
  );
});

test("rankUserCandidatesBySearch keeps exact matches ahead of typo matches", () => {
  const candidates = [
    makeUser({ displayName: "Ailce", pubkey: "2000" }),
    makeUser({ displayName: "Alice", pubkey: "1000" }),
  ];

  assert.deepEqual(
    rankUserCandidatesBySearch({
      candidates,
      getLabel: (user) => user.displayName ?? user.pubkey,
      limit: 2,
      query: "alice",
    }).map((user) => user.displayName),
    ["Alice", "Ailce"],
  );
});

test("rankUserCandidatesBySearch applies score, label, and stable order sorting", () => {
  const candidates = [
    makeUser({ displayName: "Charlie", pubkey: "3000" }),
    makeUser({ displayName: "Alice", pubkey: "1000" }),
    makeUser({ displayName: "Beta Team", pubkey: "2000" }),
    makeUser({ displayName: "Beta Build", pubkey: "2001" }),
  ];

  assert.deepEqual(
    rankUserCandidatesBySearch({
      candidates,
      getLabel: (user) => user.displayName ?? user.pubkey,
      limit: 3,
      query: "be",
    }).map((user) => user.displayName),
    ["Beta Build", "Beta Team"],
  );

  assert.deepEqual(
    rankUserCandidatesBySearch({
      allowEmptyQuery: true,
      candidates,
      getLabel: (user) => user.displayName ?? user.pubkey,
      limit: 2,
      query: "",
    }).map((user) => user.displayName),
    ["Alice", "Beta Build"],
  );
});

test("getKeyboardSearchSelection ignores stale ranked results", () => {
  const alice = makeUser({ displayName: "Alice", pubkey: "1000" });
  const charlie = makeUser({ displayName: "Charlie", pubkey: "3000" });

  assert.equal(
    getKeyboardSearchSelection({
      currentQuery: "charlie",
      rankedQuery: "",
      results: [alice],
    }),
    null,
  );
  assert.equal(
    getKeyboardSearchSelection({
      currentQuery: "charlie",
      rankedQuery: "charlie",
      results: [charlie],
    }),
    charlie,
  );
  assert.equal(
    getKeyboardSearchSelection({
      currentQuery: "   ",
      rankedQuery: "",
      results: [alice],
    }),
    null,
  );
});

test("rankUserCandidatesBySearch prefers exact human display names over crowded agent names", () => {
  const humanAviz = makeUser({
    displayName: "Aviz",
    pubkey: "221c47e3",
  });
  const agentMatches = Array.from({ length: 8 }, (_, index) =>
    makeUser({
      displayName: `Aviz-Agent-${index}`,
      isAgent: true,
      pubkey: `agent${index}`,
    }),
  );

  const candidates = [humanAviz, ...agentMatches].filter((user) => !user.isAgent);

  const ranked = rankUserCandidatesBySearch({
    candidates,
    getLabel: (user) => user.displayName ?? user.pubkey,
    limit: 50,
    query: "Aviz",
  });

  assert.equal(ranked[0]?.displayName, "Aviz");
  assert.equal(ranked.length, 1);
});

test("rankUserCandidatesBySearch keeps exact human matches ahead of agents when both are included", () => {
  const humanAviz = makeUser({
    displayName: "Aviz",
    pubkey: "221c47e3",
  });
  const avizAgent = makeUser({
    displayName: "Aviz-Agent",
    isAgent: true,
    pubkey: "agent-aviz",
  });
  const otherAgents = Array.from({ length: 8 }, (_, index) =>
    makeUser({
      displayName: `Aviz-Agent-${index}`,
      isAgent: true,
      pubkey: `agent${index}`,
    }),
  );

  const ranked = rankUserCandidatesBySearch({
    candidates: [humanAviz, avizAgent, ...otherAgents],
    getLabel: (user) => user.displayName ?? user.pubkey,
    limit: 50,
    query: "aviz",
  });

  assert.equal(ranked[0]?.displayName, "Aviz");
  assert.ok(ranked.some((user) => user.displayName === "Aviz-Agent"));
});
