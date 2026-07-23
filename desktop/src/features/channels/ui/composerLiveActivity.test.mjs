import assert from "node:assert/strict";
import test from "node:test";

import { resolveSelectedActivityAgent } from "./composerLiveActivity.ts";

const alice = { pubkey: "ALICE-pubkey", name: "Alice" };
const bob = { pubkey: "bob-pubkey", name: "Bob" };

test("explicit selection wins, case-insensitively", () => {
  const agent = resolveSelectedActivityAgent({
    openAgentSessionPubkey: "bob-pubkey",
    selectedPubkey: "alice-PUBKEY",
    workingAgents: [alice, bob],
  });
  assert.equal(agent, alice);
});

test("falls back to the open session agent", () => {
  const agent = resolveSelectedActivityAgent({
    openAgentSessionPubkey: "bob-pubkey",
    selectedPubkey: null,
    workingAgents: [alice, bob],
  });
  assert.equal(agent, bob);
});

test("falls back to the first working agent", () => {
  const agent = resolveSelectedActivityAgent({
    openAgentSessionPubkey: "gone-pubkey",
    selectedPubkey: "also-gone",
    workingAgents: [alice, bob],
  });
  assert.equal(agent, alice);
});

test("selection that stopped working falls through", () => {
  const agent = resolveSelectedActivityAgent({
    openAgentSessionPubkey: null,
    selectedPubkey: "bob-pubkey",
    workingAgents: [alice],
  });
  assert.equal(agent, alice);
});

test("returns null with no working agents", () => {
  const agent = resolveSelectedActivityAgent({
    openAgentSessionPubkey: null,
    selectedPubkey: null,
    workingAgents: [],
  });
  assert.equal(agent, null);
});
