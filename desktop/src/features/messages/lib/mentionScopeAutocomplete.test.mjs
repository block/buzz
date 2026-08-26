import assert from "node:assert/strict";
import test from "node:test";

import { buildGlobalMentionScopeCandidates } from "./mentionCandidates.ts";
import { rankMentionCandidates } from "./mentionRanking.ts";

// --- the candidate builder ---------------------------------------------------

test("a channel gets both @channel and @here entries", () => {
  const scopes = buildGlobalMentionScopeCandidates("channel");
  assert.deepEqual(
    scopes.map((entry) => entry.scope),
    ["channel", "here"],
  );
  for (const entry of scopes) {
    assert.equal(entry.kind, "scope");
    assert.equal(entry.isMember, false);
    assert.equal(entry.isAgent, false);
    assert.ok(entry.description.length > 0);
  }
  // The inserted word is the displayName, so `@${displayName}` must read back
  // as the literal marker the send path detects.
  assert.deepEqual(
    scopes.map((entry) => entry.displayName),
    ["channel", "here"],
  );
});

test("a DM offers no global-mention scopes", () => {
  assert.deepEqual(buildGlobalMentionScopeCandidates("dm"), []);
});

test("an unknown channel type offers no scopes", () => {
  assert.deepEqual(buildGlobalMentionScopeCandidates(null), []);
  assert.deepEqual(buildGlobalMentionScopeCandidates(undefined), []);
});

// --- ranking -----------------------------------------------------------------

const member = {
  kind: "identity",
  displayName: "Charlie",
  isAgent: false,
  isMember: true,
  pubkey: "a".repeat(64),
};

function rankableFor(channelType) {
  return [...buildGlobalMentionScopeCandidates(channelType), member];
}

test("scopes rank above members on an empty query", () => {
  const ranked = rankMentionCandidates(rankableFor("channel"), "");
  assert.deepEqual(
    ranked.slice(0, 2).map((item) => item.candidate.scope),
    ["channel", "here"],
  );
});

test("typing 'ch' surfaces @channel at the top", () => {
  const ranked = rankMentionCandidates(rankableFor("channel"), "ch");
  assert.equal(ranked[0].candidate.kind, "scope");
  assert.equal(ranked[0].candidate.scope, "channel");
});

test("typing 'here' surfaces @here and not @channel", () => {
  const ranked = rankMentionCandidates(rankableFor("channel"), "here");
  const scopes = ranked
    .filter((item) => item.candidate.kind === "scope")
    .map((item) => item.candidate.scope);
  assert.deepEqual(scopes, ["here"]);
});

test("a member whose name matches still ranks below a scope", () => {
  // "Chat" starts with "ch", same as "channel"; the scope must still win.
  const named = { ...member, displayName: "Chat" };
  const ranked = rankMentionCandidates(
    [...buildGlobalMentionScopeCandidates("channel"), named],
    "ch",
  );
  assert.equal(ranked[0].candidate.kind, "scope");
});
