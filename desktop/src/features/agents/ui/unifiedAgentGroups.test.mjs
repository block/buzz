import assert from "node:assert/strict";
import test from "node:test";

import { filterLocalStartersWhenCommunityHasAgents } from "./unifiedAgentGroups.ts";

const starterPersonas = [
  { id: "builtin:fizz", displayName: "Fizz" },
  { id: "builtin:honey", displayName: "Honey" },
  { id: "builtin:bumble", displayName: "Bumble" },
];

test("keeps local starter agents in a new local workspace", () => {
  const result = filterLocalStartersWhenCommunityHasAgents(starterPersonas, [
    { name: "Fizz", pubkey: "", relayUrl: "", personaId: null },
  ]);

  assert.equal(result.personas.length, 3);
  assert.equal(result.agents.length, 1);
});

test("shows the community roster instead of local starter agents", () => {
  const liv = {
    name: "Liv",
    pubkey: "a".repeat(64),
    relayUrl: "wss://community.example.com",
    personaId: "liv",
  };
  const result = filterLocalStartersWhenCommunityHasAgents(starterPersonas, [
    { name: "Fizz", pubkey: "", relayUrl: "", personaId: null },
    { name: "Honey", pubkey: "", relayUrl: "", personaId: null },
    { name: "Bumble", pubkey: "", relayUrl: "", personaId: null },
    liv,
  ]);

  assert.deepEqual(result.personas, []);
  assert.deepEqual(result.agents, [liv]);
});
