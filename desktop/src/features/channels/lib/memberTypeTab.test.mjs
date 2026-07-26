import assert from "node:assert/strict";
import test from "node:test";

import {
  filterAddSearchResultsForTypeTab,
  filterMembersForTypeTab,
} from "./memberTypeTab.ts";

function member(pubkey, role, isAgent) {
  return {
    pubkey,
    role,
    isAgent,
    joinedAt: "2026-07-01T00:00:00.000Z",
    displayName: null,
  };
}

const owner = member("owner-pk", "owner", false);
const human = member("human-pk", "member", false);
const roleBot = member("role-bot-pk", "bot", true);
// Backend-flagged agent added by someone else with a plain "member" role —
// classification must not depend on role alone.
const flaggedAgent = member("flagged-agent-pk", "member", true);

const members = [owner, human, roleBot, flaggedAgent];
const isBot = (candidate) => candidate.role === "bot" || candidate.isAgent;

test("filterMembersForTypeTab returns every member for the all tab", () => {
  assert.deepEqual(filterMembersForTypeTab(members, "all", isBot), members);
});

test("filterMembersForTypeTab keeps only non-agents on the people tab", () => {
  assert.deepEqual(filterMembersForTypeTab(members, "people", isBot), [
    owner,
    human,
  ]);
});

test("filterMembersForTypeTab keeps only agents on the agents tab", () => {
  assert.deepEqual(filterMembersForTypeTab(members, "agents", isBot), [
    roleBot,
    flaggedAgent,
  ]);
});

test("filterMembersForTypeTab preserves input order", () => {
  const reversed = [...members].reverse();
  assert.deepEqual(filterMembersForTypeTab(reversed, "agents", isBot), [
    flaggedAgent,
    roleBot,
  ]);
});

function searchResult(pubkey, isAgent) {
  return {
    pubkey,
    displayName: pubkey,
    avatarUrl: null,
    nip05Handle: null,
    ownerPubkey: null,
    isAgent,
  };
}

const humanResult = searchResult("human-result-pk", false);
const agentResult = searchResult("agent-result-pk", true);
const results = [humanResult, agentResult];

test("filterAddSearchResultsForTypeTab returns everything for the all tab", () => {
  assert.deepEqual(filterAddSearchResultsForTypeTab(results, "all"), results);
});

test("filterAddSearchResultsForTypeTab scopes the people tab to non-agents", () => {
  assert.deepEqual(filterAddSearchResultsForTypeTab(results, "people"), [
    humanResult,
  ]);
});

test("filterAddSearchResultsForTypeTab scopes the agents tab to agents", () => {
  assert.deepEqual(filterAddSearchResultsForTypeTab(results, "agents"), [
    agentResult,
  ]);
});
