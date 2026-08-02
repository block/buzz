import assert from "node:assert/strict";
import test from "node:test";

import { huddleAgentPicks } from "./huddleAgentPicks.ts";

const LOCAL = { type: "local" };
const REMOTE = { type: "provider", id: "ssh", config: {} };

function agent(overrides = {}) {
  return {
    pubkey: "aa".repeat(32),
    name: "agent",
    status: "running",
    backend: LOCAL,
    ...overrides,
  };
}

const remote = (overrides = {}) =>
  agent({
    pubkey: "bb".repeat(32),
    name: "remote",
    status: "deployed",
    backend: REMOTE,
    backendAgentId: "remote-1",
    ...overrides,
  });

const pick = ({ agents = [], presenceLookup = {}, currentAgentPubkeys = [] }) =>
  huddleAgentPicks({ agents, presenceLookup, currentAgentPubkeys });

// --- the live bug: a remote agent is never "running" -------------------------

test("a deployed remote agent the relay says is online is addable", () => {
  // The old filter asked `status === "running"`, which no provider-backed
  // record ever answers, so an alive remote fleet read as an empty huddle.
  const result = pick({
    agents: [remote()],
    presenceLookup: { ["bb".repeat(32)]: "online" },
  });
  assert.deepEqual(
    result.picks.map((p) => p.pubkey),
    ["bb".repeat(32)],
  );
  assert.equal(result.picks[0].presence, "online");
});

test("an away remote agent is still addable — it can hear the huddle", () => {
  const result = pick({
    agents: [remote()],
    presenceLookup: { ["bb".repeat(32)]: "away" },
  });
  assert.equal(result.picks.length, 1);
  assert.equal(result.picks[0].presence, "away");
});

test("a deployed remote agent with nothing on the relay is not offered", () => {
  // `backend_agent_id` is written once at deploy and never cleared, so
  // "deployed" alone would offer a process that died hours ago.
  assert.deepEqual(pick({ agents: [remote()] }).picks, []);
});

// --- local records keep the control plane's word -----------------------------

test("a running local agent is addable through a silent relay", () => {
  const result = pick({ agents: [agent()] });
  assert.equal(result.picks.length, 1);
  assert.equal(result.picks[0].presence, "online");
});

test("a stopped local agent is not offered", () => {
  // managedAgentPresenceStatus answers "online" for ANY local record, so the
  // control-plane gate is what keeps a stopped process out of the list.
  assert.deepEqual(pick({ agents: [agent({ status: "stopped" })] }).picks, []);
});

// --- already-joined agents ---------------------------------------------------

test("agents already in the huddle are dropped, case-insensitively", () => {
  const result = pick({
    agents: [agent(), remote()],
    presenceLookup: { ["bb".repeat(32)]: "online" },
    currentAgentPubkeys: ["AA".repeat(32)],
  });
  assert.deepEqual(
    result.picks.map((p) => p.pubkey),
    ["bb".repeat(32)],
  );
});

// --- empty-state copy tells the two cases apart ------------------------------

test("an empty fleet and a fully-joined one read differently", () => {
  assert.equal(
    pick({ agents: [remote()] }).emptyMessage,
    "No online agents found.",
  );
  assert.equal(
    pick({ agents: [agent()], currentAgentPubkeys: ["aa".repeat(32)] })
      .emptyMessage,
    "All online agents are already in this huddle.",
  );
});
