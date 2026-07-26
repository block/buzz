import assert from "node:assert/strict";
import test from "node:test";

import {
  agentsForAllActivityPanel,
  shouldShowViewAllAgentActivity,
} from "./botActivityViewAll.ts";

const AGENT_A = { pubkey: "aa".repeat(32), name: "Alpha", status: "running" };
const AGENT_B = { pubkey: "bb".repeat(32), name: "Beta", status: "running" };
const AGENT_C = { pubkey: "cc".repeat(32), name: "Gamma", status: "stopped" };

test("shows view all when two agents are working", () => {
  assert.equal(
    shouldShowViewAllAgentActivity({
      agents: [AGENT_A, AGENT_B, AGENT_C],
      workingBotPubkeys: [AGENT_A.pubkey, AGENT_B.pubkey],
    }),
    true,
  );
});

test("shows view all when two agents are running even if only one is working", () => {
  assert.equal(
    shouldShowViewAllAgentActivity({
      agents: [AGENT_A, AGENT_B],
      workingBotPubkeys: [AGENT_A.pubkey],
    }),
    true,
  );
});

test("hides view all for a single active agent with no peer work", () => {
  assert.equal(
    shouldShowViewAllAgentActivity({
      agents: [AGENT_A, AGENT_C],
      workingBotPubkeys: [AGENT_A.pubkey],
    }),
    false,
  );
});

test("all activity panel includes running and working agents", () => {
  const panelAgents = agentsForAllActivityPanel({
    agents: [AGENT_A, AGENT_B, AGENT_C],
    workingBotPubkeys: [AGENT_A.pubkey],
  });
  assert.deepEqual(
    panelAgents.map((agent) => agent.pubkey),
    [AGENT_A.pubkey, AGENT_B.pubkey],
  );
});
