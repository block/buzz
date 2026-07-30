import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { combineFleetAgents } from "./fleetAgents.ts";

const ME = "a1".repeat(32);
const OTHER_OWNER = "b2".repeat(32);
const MANAGED_PUBKEY = "c3".repeat(32);
const REMOTE_PUBKEY = "d4".repeat(32);
const UNOWNED_PUBKEY = "e5".repeat(32);

function makeManagedAgent(overrides = {}) {
  return {
    pubkey: MANAGED_PUBKEY,
    name: "Local Agent",
    personaId: null,
    runtime: null,
    relayUrl: "ws://localhost:3000",
    acpCommand: "buzz-acp",
    agentCommand: "goose",
    agentCommandOverride: null,
    agentArgs: [],
    mcpCommand: "buzz-dev-mcp",
    turnTimeoutSeconds: 600,
    idleTimeoutSeconds: null,
    maxTurnDurationSeconds: null,
    parallelism: 1,
    systemPrompt: null,
    avatarUrl: "https://example.com/a.png",
    model: "gpt-x",
    modelSource: null,
    provider: null,
    personaOutOfDate: false,
    personaOrphaned: false,
    needsRestart: false,
    envVars: {},
    status: "running",
    pid: 123,
    createdAt: "2026-07-01T00:00:00Z",
    updatedAt: "2026-07-01T00:00:00Z",
    lastStartedAt: "2026-07-29T08:00:00Z",
    lastStoppedAt: null,
    lastExitCode: null,
    lastError: null,
    lastErrorCode: null,
    logPath: "/tmp/agent.log",
    startOnAppLaunch: false,
    autoRestartOnConfigChange: false,
    backend: { type: "local" },
    backendAgentId: null,
    respondTo: "owner-only",
    respondToAllowlist: [],
    ...overrides,
  };
}

function makeRelayAgent(overrides = {}) {
  return {
    pubkey: REMOTE_PUBKEY,
    name: "Remote Agent",
    agentType: "goose",
    channels: [],
    channelIds: [],
    capabilities: [],
    status: "online",
    respondTo: null,
    respondToAllowlist: [],
    ...overrides,
  };
}

describe("combineFleetAgents", () => {
  it("includes managed agents with their full metadata", () => {
    const fleet = combineFleetAgents(
      [makeManagedAgent()],
      [],
      new Map(),
      new Map(),
      ME,
    );
    assert.equal(fleet.length, 1);
    assert.equal(fleet[0].isManaged, true);
    assert.equal(fleet[0].status, "running");
    assert.equal(fleet[0].model, "gpt-x");
  });

  it("adds declared-owned relay agents and skips agents owned by others", () => {
    const fleet = combineFleetAgents(
      [],
      [
        makeRelayAgent(),
        makeRelayAgent({ pubkey: UNOWNED_PUBKEY, name: "Someone else's" }),
      ],
      new Map([
        [REMOTE_PUBKEY, ME],
        [UNOWNED_PUBKEY, OTHER_OWNER],
      ]),
      new Map([[REMOTE_PUBKEY, "https://example.com/r.png"]]),
      ME,
    );
    assert.equal(fleet.length, 1);
    assert.equal(fleet[0].pubkey, REMOTE_PUBKEY);
    assert.equal(fleet[0].isManaged, false);
    assert.equal(fleet[0].status, "deployed");
    assert.equal(fleet[0].avatarUrl, "https://example.com/r.png");
  });

  it("maps offline remote presence to not_deployed", () => {
    const fleet = combineFleetAgents(
      [],
      [makeRelayAgent({ status: "offline" })],
      new Map([[REMOTE_PUBKEY, ME]]),
      new Map(),
      ME,
    );
    assert.equal(fleet[0].status, "not_deployed");
  });

  it("managed agents win pubkey collisions with the relay roster", () => {
    const fleet = combineFleetAgents(
      [makeManagedAgent()],
      [makeRelayAgent({ pubkey: MANAGED_PUBKEY, name: "Duplicate" })],
      new Map([[MANAGED_PUBKEY, ME]]),
      new Map(),
      ME,
    );
    assert.equal(fleet.length, 1);
    assert.equal(fleet[0].isManaged, true);
    assert.equal(fleet[0].name, "Local Agent");
  });

  it("ignores relay agents entirely before identity resolves", () => {
    const fleet = combineFleetAgents(
      [makeManagedAgent()],
      [makeRelayAgent()],
      new Map([[REMOTE_PUBKEY, ME]]),
      new Map(),
      undefined,
    );
    assert.equal(fleet.length, 1);
    assert.equal(fleet[0].isManaged, true);
  });

  it("sorts by name then pubkey for a stable grid", () => {
    const fleet = combineFleetAgents(
      [
        makeManagedAgent({ name: "Zeta" }),
        makeManagedAgent({ pubkey: REMOTE_PUBKEY, name: "Alpha" }),
      ],
      [],
      new Map(),
      new Map(),
      ME,
    );
    assert.deepEqual(
      fleet.map((agent) => agent.name),
      ["Alpha", "Zeta"],
    );
  });
});
