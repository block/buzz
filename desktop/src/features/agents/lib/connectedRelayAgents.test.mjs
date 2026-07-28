import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { connectedRelayAgents } from "./connectedRelayAgents.ts";

function relayAgent(name, pubkey) {
  return {
    pubkey,
    name,
    agentType: "external",
    channels: [],
    channelIds: [],
    capabilities: [],
    status: "online",
    respondTo: null,
    respondToAllowlist: [],
  };
}

describe("connectedRelayAgents", () => {
  it("excludes locally managed identities case-insensitively", () => {
    const managedPubkey = "A".repeat(64);
    const result = connectedRelayAgents(
      [
        relayAgent("Starter", managedPubkey),
        relayAgent("Alpha", "b".repeat(64)),
      ],
      new Set([managedPubkey.toLowerCase()]),
    );

    assert.deepEqual(
      result.map((agent) => agent.name),
      ["Alpha"],
    );
  });

  it("sorts connected agents by name without mutating the relay result", () => {
    const relayAgents = [
      relayAgent("Delta", "c".repeat(64)),
      relayAgent("Bravo", "d".repeat(64)),
    ];

    assert.deepEqual(
      connectedRelayAgents(relayAgents, new Set()).map((agent) => agent.name),
      ["Bravo", "Delta"],
    );
    assert.deepEqual(
      relayAgents.map((agent) => agent.name),
      ["Delta", "Bravo"],
    );
  });
});
