import assert from "node:assert/strict";
import test from "node:test";

import {
  describeWorkspaceAgent,
  resolveWorkspaceAgentAvailability,
  resolveWorkspaceOwnerLabel,
  selectWorkspaceAgents,
} from "./workspaceAgents.ts";

const MINE = "a".repeat(64);
const THEIRS = "b".repeat(64);
const OWNER = "c".repeat(64);

function relayAgent(overrides = {}) {
  return {
    pubkey: THEIRS,
    ownerPubkey: OWNER,
    name: "Scout",
    agentType: "omp",
    channels: ["general"],
    channelIds: ["ch-1"],
    capabilities: [],
    status: "unknown",
    respondTo: null,
    respondToAllowlist: [],
    ...overrides,
  };
}

test("excludes the viewer's managed agents by pubkey, case-insensitively", () => {
  const selected = selectWorkspaceAgents(
    [relayAgent({ pubkey: MINE.toUpperCase() }), relayAgent()],
    [{ pubkey: ` ${MINE}` }],
  );

  assert.deepEqual(
    selected.map((agent) => agent.pubkey),
    [THEIRS],
  );
});

test("collapses duplicate relay entries for one pubkey", () => {
  const selected = selectWorkspaceAgents(
    [relayAgent({ name: "First" }), relayAgent({ name: "Second" })],
    [],
  );

  assert.deepEqual(
    selected.map((agent) => agent.name),
    ["First"],
  );
});

test("blank names fall back to the canonical truncated pubkey", () => {
  const [agent] = selectWorkspaceAgents([relayAgent({ name: "  " })], []);

  assert.equal(agent.name, `${"b".repeat(8)}…${"b".repeat(4)}`);
});

test("sorts by display name and tolerates undefined inputs", () => {
  const selected = selectWorkspaceAgents(
    [
      relayAgent({ pubkey: "1".repeat(64), name: "Zed" }),
      relayAgent({ pubkey: "2".repeat(64), name: "Ada" }),
    ],
    undefined,
  );

  assert.deepEqual(
    selected.map((agent) => agent.name),
    ["Ada", "Zed"],
  );
  assert.deepEqual(selectWorkspaceAgents(undefined, undefined), []);
});

test("describeWorkspaceAgent joins owner, runtime, and channel count", () => {
  assert.equal(
    describeWorkspaceAgent(relayAgent(), "Alice"),
    "Managed by Alice · omp · in 1 channel",
  );
  assert.equal(
    describeWorkspaceAgent(
      relayAgent({ agentType: "buzz-acp", channels: ["a", "b", "c"] }),
      null,
    ),
    "buzz-acp · in 3 channels",
  );
  assert.equal(
    describeWorkspaceAgent(relayAgent({ agentType: " ", channels: [] }), null),
    "in 0 channels",
  );
});

test("owner label prefers display name, then name, then truncated pubkey", () => {
  assert.equal(resolveWorkspaceOwnerLabel(null, { displayName: "Alice" }), null);
  assert.equal(
    resolveWorkspaceOwnerLabel(OWNER, { displayName: " Alice ", name: "al" }),
    "Alice",
  );
  assert.equal(
    resolveWorkspaceOwnerLabel(OWNER, { displayName: "", name: "al" }),
    "al",
  );
  assert.equal(
    resolveWorkspaceOwnerLabel(OWNER, undefined),
    `${"c".repeat(8)}…${"c".repeat(4)}`,
  );
});

test("availability prefers relay presence over the directory snapshot", () => {
  assert.equal(resolveWorkspaceAgentAvailability("offline", "online"), "offline");
  assert.equal(resolveWorkspaceAgentAvailability(undefined, "away"), "away");
  assert.equal(resolveWorkspaceAgentAvailability(undefined, "unknown"), undefined);
});
