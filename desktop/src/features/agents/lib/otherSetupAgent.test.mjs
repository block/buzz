import assert from "node:assert/strict";
import test from "node:test";

import {
  isOtherSetupAgent,
  isOwnedAgentNotManagedOnDevice,
  ownedAgentsNotManagedOnDevice,
} from "./otherSetupAgent.ts";

const OWNER = "a".repeat(64);
const AGENT = "b".repeat(64);

test("fails closed while the local managed directory is unresolved", () => {
  assert.equal(
    isOtherSetupAgent({
      agentDirectoriesReady: false,
      currentPubkey: OWNER,
      managedAgents: [],
      profileOwnerPubkey: OWNER,
      pubkey: AGENT,
      relayAgents: [],
    }),
    false,
  );
});

test("labels a viewer-owned identity as not managed on this device", () => {
  assert.equal(
    isOtherSetupAgent({
      agentDirectoriesReady: true,
      currentPubkey: OWNER,
      managedAgents: [],
      profileOwnerPubkey: OWNER,
      pubkey: AGENT,
      relayAgents: [],
    }),
    true,
  );
});

test("a locally managed provider is not labeled as another device", () => {
  assert.equal(
    isOtherSetupAgent({
      agentDirectoriesReady: true,
      currentPubkey: OWNER,
      managedAgents: [{ pubkey: AGENT, backend: { type: "provider" } }],
      profileOwnerPubkey: OWNER,
      pubkey: AGENT,
      relayAgents: [],
    }),
    false,
  );
});

for (const [name, overrides, expected] of [
  ["owned absent key", {}, true],
  ["loading local inventory", { localInventoryReady: false }, false],
  ["exact local provider record", { isLocallyManaged: true }, false],
  ["different owner", { ownerPubkey: "b".repeat(64) }, false],
  ["unknown ownership", { ownerPubkey: null }, false],
]) {
  test(`shared provenance: ${name}`, () => {
    assert.equal(
      isOwnedAgentNotManagedOnDevice({
        currentPubkey: "a".repeat(64),
        ownerPubkey: "A".repeat(64),
        localInventoryReady: true,
        isLocallyManaged: false,
        ...overrides,
      }),
      expected,
    );
  });
}

function relayAgent(pubkey, name, ownerPubkey = OWNER) {
  return {
    pubkey,
    ownerPubkey,
    name,
    agentType: "agent",
    channels: [],
    channelIds: [],
    capabilities: [],
    status: "unknown",
    respondTo: "owner-only",
    respondToAllowlist: [],
  };
}

const notArchived = () => false;

test("running elsewhere: lists owned relay agents without a local record, by name", () => {
  const listed = ownedAgentsNotManagedOnDevice({
    agentDirectoriesReady: true,
    currentPubkey: OWNER,
    isArchived: notArchived,
    managedAgents: [{ pubkey: "c".repeat(64) }],
    relayAgents: [
      relayAgent("d".repeat(64), "Zed"),
      relayAgent("c".repeat(64), "Local twin"),
      relayAgent("e".repeat(64), "Someone else's", "f".repeat(64)),
      relayAgent("1".repeat(64), "Unowned", null),
      relayAgent(AGENT, "Atlas"),
    ],
  });
  assert.deepEqual(
    listed.map((agent) => agent.name),
    ["Atlas", "Zed"],
  );
});

test("running elsewhere: empty until both directories are ready", () => {
  assert.deepEqual(
    ownedAgentsNotManagedOnDevice({
      agentDirectoriesReady: false,
      currentPubkey: OWNER,
      isArchived: notArchived,
      managedAgents: [],
      relayAgents: [relayAgent(AGENT, "Atlas")],
    }),
    [],
  );
  assert.deepEqual(
    ownedAgentsNotManagedOnDevice({
      agentDirectoriesReady: true,
      currentPubkey: undefined,
      isArchived: notArchived,
      managedAgents: [],
      relayAgents: [relayAgent(AGENT, "Atlas")],
    }),
    [],
  );
});

test("running elsewhere: matches keys case-insensitively and lists each once", () => {
  const listed = ownedAgentsNotManagedOnDevice({
    agentDirectoriesReady: true,
    currentPubkey: OWNER.toUpperCase(),
    isArchived: notArchived,
    managedAgents: [{ pubkey: "C".repeat(64) }],
    relayAgents: [
      relayAgent(AGENT, "Atlas"),
      relayAgent(AGENT.toUpperCase(), "Atlas"),
      relayAgent("c".repeat(64), "Local twin"),
    ],
  });
  assert.deepEqual(
    listed.map((agent) => agent.pubkey),
    [AGENT],
  );
});

test("running elsewhere: omits archived identities", () => {
  assert.deepEqual(
    ownedAgentsNotManagedOnDevice({
      agentDirectoriesReady: true,
      currentPubkey: OWNER,
      isArchived: (pubkey) => pubkey === AGENT,
      managedAgents: [],
      relayAgents: [relayAgent(AGENT, "Atlas")],
    }),
    [],
  );
});
