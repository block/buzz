import assert from "node:assert/strict";
import test from "node:test";

import {
  getKeyboardSearchSelection,
  rankUserCandidatesBySearch,
  scoreUserCandidate,
} from "./userCandidateSearch.ts";
import { pubkeyToNpub } from "../../../shared/lib/nostrUtils.ts";

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

test("scoreUserCandidate matches the canonical npub", () => {
  const pubkey =
    "50d0af578a29c245c08c9e2ad95be422ba6e3a9df938dcf6b8863ebcdec399db";
  const npub = pubkeyToNpub(pubkey);
  const user = makeUser({ pubkey });

  assert.equal(scoreUserCandidate({ label: pubkey, query: npub, user }), 3);
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
