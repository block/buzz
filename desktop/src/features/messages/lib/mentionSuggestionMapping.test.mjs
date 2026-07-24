import assert from "node:assert/strict";
import test from "node:test";

import { mapMentionCandidateToSuggestion } from "./mentionSuggestionMapping.ts";

const AGENT_PUBKEY = "a".repeat(64);

function agentCandidate(overrides = {}) {
  return {
    kind: "identity",
    pubkey: AGENT_PUBKEY,
    isAgent: true,
    isMember: true,
    ...overrides,
  };
}

function profileSummary(about) {
  return {
    displayName: "Bumble",
    name: null,
    avatarUrl: null,
    about,
    nip05Handle: null,
    ownerPubkey: null,
    isAgent: true,
  };
}

test("agent description comes from the candidate when resolved at build time", () => {
  const suggestion = mapMentionCandidateToSuggestion({
    candidate: agentCandidate({ description: "Researcher — deep dives" }),
    label: "Bumble",
    profiles: { [AGENT_PUBKEY]: profileSummary("stale profile about") },
  });

  assert.equal(suggestion.description, "Researcher — deep dives");
});

test("agent description falls back to the profile lookup's about", () => {
  const suggestion = mapMentionCandidateToSuggestion({
    candidate: agentCandidate(),
    label: "Bumble",
    profiles: { [AGENT_PUBKEY]: profileSummary("Researcher — deep dives") },
  });

  assert.equal(suggestion.description, "Researcher — deep dives");
});

test("agent description is null when about is missing everywhere", () => {
  const suggestion = mapMentionCandidateToSuggestion({
    candidate: agentCandidate(),
    label: "Bumble",
    profiles: { [AGENT_PUBKEY]: profileSummary(null) },
  });

  assert.equal(suggestion.description, null);
});

test("non-agent suggestions never carry a description", () => {
  const suggestion = mapMentionCandidateToSuggestion({
    candidate: agentCandidate({ isAgent: false }),
    label: "Alice",
    profiles: { [AGENT_PUBKEY]: profileSummary("A human bio") },
  });

  assert.equal(suggestion.description, null);
});

test("multi-line about collapses to a single trimmed line", () => {
  const suggestion = mapMentionCandidateToSuggestion({
    candidate: agentCandidate({
      description: "  Writer bee.\nDrafts docs\n\tand posts.  ",
    }),
    label: "Honey",
  });

  assert.equal(suggestion.description, "Writer bee. Drafts docs and posts.");
});

test("whitespace-only about degrades to null (name-only row)", () => {
  const suggestion = mapMentionCandidateToSuggestion({
    candidate: agentCandidate({ description: "   \n  " }),
    label: "Fizz",
  });

  assert.equal(suggestion.description, null);
});
