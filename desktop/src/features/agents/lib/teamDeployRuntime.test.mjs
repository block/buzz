import assert from "node:assert/strict";
import test from "node:test";

import {
  runtimeForTeamPersonaDeploy,
  sourceAgentForPersona,
} from "./teamDeployRuntime.ts";

const gooseRuntime = {
  id: "goose",
  label: "Goose",
  avatarUrl: "https://runtime/goose.png",
  availability: "available",
  command: "goose-cmd",
  binaryPath: "/bin/goose",
  defaultArgs: ["--acp"],
  mcpCommand: "goose-mcp",
  installHint: "",
  installInstructionsUrl: "",
  canAutoInstall: false,
  underlyingCliPath: null,
};

const buzzAgentRuntime = {
  ...gooseRuntime,
  id: "buzz-agent",
  label: "Buzz Agent",
  command: "buzz-agent-cmd",
  mcpCommand: null,
};

function persona(overrides = {}) {
  return {
    id: "p-1",
    displayName: "Dolomite",
    systemPrompt: "prompt",
    model: null,
    runtime: "goose",
    avatarUrl: "https://example.com/a.png",
    envVars: {},
    isBuiltIn: false,
    ...overrides,
  };
}

function managedAgent(overrides = {}) {
  return {
    pubkey: "aa",
    personaId: "p-1",
    teamId: null,
    agentCommandOverride: "/home/user/.local/bin/openclaw-acp-buzz",
    mcpCommand: "openclaw-mcp",
    ...overrides,
  };
}

test("team deploy copies personal agent_command_override and sets harnessOverride", () => {
  const result = runtimeForTeamPersonaDeploy({
    persona: persona(),
    runtimes: [gooseRuntime, buzzAgentRuntime],
    defaultProvider: buzzAgentRuntime,
    managedAgents: [managedAgent()],
  });
  assert.ok(result);
  assert.equal(result.harnessOverride, true);
  assert.equal(result.runtime.command, "/home/user/.local/bin/openclaw-acp-buzz");
  assert.equal(result.runtime.id, "custom");
  assert.equal(result.runtime.mcpCommand, "openclaw-mcp");
});

test("team deploy matches an available runtime by command and keeps that id", () => {
  const result = runtimeForTeamPersonaDeploy({
    persona: persona(),
    runtimes: [gooseRuntime, buzzAgentRuntime],
    defaultProvider: buzzAgentRuntime,
    managedAgents: [managedAgent({ agentCommandOverride: "goose-cmd" })],
  });
  assert.ok(result);
  assert.equal(result.harnessOverride, true);
  assert.equal(result.runtime.id, "goose");
  assert.equal(result.runtime.command, "goose-cmd");
});

test("team deploy without override falls back to persona runtime", () => {
  const result = runtimeForTeamPersonaDeploy({
    persona: persona(),
    runtimes: [gooseRuntime, buzzAgentRuntime],
    defaultProvider: buzzAgentRuntime,
    managedAgents: [managedAgent({ agentCommandOverride: null })],
  });
  assert.ok(result);
  assert.equal(result.harnessOverride, false);
  assert.equal(result.runtime.id, "goose");
});

test("sourceAgentForPersona prefers the personal instance that has an override", () => {
  const source = sourceAgentForPersona(
    [
      managedAgent({ pubkey: "team-clone", teamId: "t-1" }),
      managedAgent({
        pubkey: "plain",
        agentCommandOverride: null,
      }),
      managedAgent({ pubkey: "pinned" }),
    ],
    "p-1",
  );
  assert.equal(source?.pubkey, "pinned");
});
