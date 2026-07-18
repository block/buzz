import assert from "node:assert/strict";
import test from "node:test";

import {
  assertAgentBelongsToActiveRelay,
  pickPreferredChannelPresetAgent,
} from "./channelAgents.ts";

// Regression guard for the cross-community "silent-success trap": before this
// fix, a managed agent pinned to community A's relay could be selected and
// "attached" while operating in community B — membership was added against a
// process frozen on relay A that never hears B (Max's baseline). Both guards
// below close that path. They exercise the real selection/attach guards, not a
// re-derivation of `agentBelongsToRelay` (Mari's coverage requirement).

const RELAY_A = "wss://relay-a.example";
const RELAY_B = "wss://relay-b.example";
const PUB_A = "a".repeat(64);
const PUB_B = "b".repeat(64);

function makeAgent(overrides = {}) {
  return {
    pubkey: PUB_A,
    name: "goose",
    relayUrl: RELAY_A,
    agentCommand: "goose",
    status: "running",
    updatedAt: "2026-01-15T00:00:00Z",
    ...overrides,
  };
}

test("pickPreferredChannelPresetAgent: excludes a foreign-relay agent (in-channel branch)", () => {
  // A-pinned running agent that is already a member of the B channel: it must
  // NOT be reused, so the caller falls through to creating a B agent.
  const agentA = makeAgent({ pubkey: PUB_A, relayUrl: RELAY_A });
  const memberPubkeys = new Set([PUB_A]);

  const picked = pickPreferredChannelPresetAgent(
    [agentA],
    memberPubkeys,
    "goose",
    "goose",
    RELAY_B,
  );

  assert.equal(picked, undefined, "foreign-relay agent must not be selected");
});

test("pickPreferredChannelPresetAgent: excludes a foreign-relay agent (name-match branch)", () => {
  const agentA = makeAgent({ pubkey: PUB_A, relayUrl: RELAY_A });

  const picked = pickPreferredChannelPresetAgent(
    [agentA],
    new Set(), // not a member — exercises the name-match fallback branch
    "goose",
    "goose",
    RELAY_B,
  );

  assert.equal(picked, undefined, "foreign-relay agent must not match by name");
});

test("pickPreferredChannelPresetAgent: selects a same-relay agent", () => {
  const agentB = makeAgent({ pubkey: PUB_B, relayUrl: RELAY_B });

  const picked = pickPreferredChannelPresetAgent(
    [agentB],
    new Set([PUB_B]),
    "goose",
    "goose",
    RELAY_B,
  );

  assert.equal(picked?.pubkey, PUB_B, "home-relay agent is reusable");
});

test("pickPreferredChannelPresetAgent: blank-pin agent follows the active relay", () => {
  // Defense in depth: a record that escaped stamping (blank relayUrl) follows
  // the active community rather than being hidden everywhere.
  const blank = makeAgent({ pubkey: PUB_B, relayUrl: "" });

  const picked = pickPreferredChannelPresetAgent(
    [blank],
    new Set([PUB_B]),
    "goose",
    "goose",
    RELAY_B,
  );

  assert.equal(
    picked?.pubkey,
    PUB_B,
    "blank-pin agent is eligible in any community",
  );
});

test("assertAgentBelongsToActiveRelay: throws an actionable error for a foreign agent", () => {
  const agentA = makeAgent({ name: "haiku-bot", relayUrl: RELAY_A });

  assert.throws(
    () => assertAgentBelongsToActiveRelay(agentA, RELAY_B),
    (err) => {
      // Actionable per Eva's refinement: names the agent, its home relay, and
      // the active relay — not a bare boolean-y message.
      assert.ok(err instanceof Error);
      assert.match(err.message, /haiku-bot/, "names the agent");
      assert.match(err.message, /relay-a\.example/, "names the home relay");
      assert.match(err.message, /relay-b\.example/, "names the active relay");
      return true;
    },
  );
});

test("assertAgentBelongsToActiveRelay: does not throw for a same-relay agent", () => {
  const agentB = makeAgent({ relayUrl: RELAY_B });
  assert.doesNotThrow(() => assertAgentBelongsToActiveRelay(agentB, RELAY_B));
});

test("assertAgentBelongsToActiveRelay: does not throw for a blank-pin agent", () => {
  const blank = makeAgent({ relayUrl: "" });
  assert.doesNotThrow(() => assertAgentBelongsToActiveRelay(blank, RELAY_B));
});
