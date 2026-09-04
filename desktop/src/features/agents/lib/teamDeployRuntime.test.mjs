import assert from "node:assert/strict";
import test from "node:test";

import {
  createInputForTeamPersonaDeploy,
  isTeamDeploySourceReady,
  runtimeForTeamPersonaDeploy,
  sourceConfigurationForPersona,
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
    backend: { type: "local" },
    agentCommandOverride: "goose-cmd",
    agentArgs: ["--acp"],
    mcpCommand: "ignored-by-create",
    ...overrides,
  };
}

test("delayed managed-agents query blocks deploy until fetched", () => {
  assert.deepEqual(
    isTeamDeploySourceReady({
      isPending: true,
      isError: false,
      isFetched: false,
    }),
    { ready: false, blockReason: "loading" },
  );
  assert.deepEqual(
    isTeamDeploySourceReady({
      isPending: false,
      isError: true,
      isFetched: true,
    }),
    { ready: false, blockReason: "error" },
  );
  assert.deepEqual(
    isTeamDeploySourceReady({
      isPending: false,
      isError: false,
      isFetched: true,
    }),
    { ready: true, blockReason: null },
  );
});

test("team deploy copies a local pin onto create command, args, and harnessOverride", () => {
  const plan = runtimeForTeamPersonaDeploy({
    persona: persona(),
    runtimes: [gooseRuntime, buzzAgentRuntime],
    defaultProvider: buzzAgentRuntime,
    managedAgents: [managedAgent()],
  });
  assert.equal(plan.status, "ready");
  if (plan.status !== "ready") {
    return;
  }
  const create = createInputForTeamPersonaDeploy({
    persona: persona(),
    teamId: "team-1",
    plan,
  });
  assert.equal(create.agentCommand, "goose-cmd");
  assert.equal(plan.agentCommand, "goose-cmd");
  assert.equal(create.harnessOverride, true);
  assert.deepEqual(create.agentArgs, ["--acp"]);
  assert.equal(create.backend?.type, "local");
  assert.equal(
    "mcpCommand" in create,
    false,
    "create must not send mcpCommand — backend derives MCP from the catalog command",
  );
});

test("pinned Homebrew path matches the catalog command via commandsMatch", () => {
  const plan = runtimeForTeamPersonaDeploy({
    persona: persona(),
    runtimes: [gooseRuntime, buzzAgentRuntime],
    defaultProvider: buzzAgentRuntime,
    managedAgents: [
      managedAgent({
        agentCommandOverride: "/opt/homebrew/bin/goose-cmd",
        agentArgs: ["--acp"],
      }),
    ],
  });
  assert.equal(plan.status, "ready");
  if (plan.status !== "ready") {
    return;
  }
  assert.equal(plan.runtime.id, "goose");
  assert.equal(plan.agentCommand, "/opt/homebrew/bin/goose-cmd");
  assert.equal(plan.harnessOverride, true);
  assert.deepEqual(plan.agentArgs, ["--acp"]);
  const create = createInputForTeamPersonaDeploy({
    persona: persona(),
    teamId: "team-1",
    plan,
  });
  assert.equal(create.agentCommand, "/opt/homebrew/bin/goose-cmd");
});

test("claude-code-acp pin matches the claude-acp catalog command", () => {
  const claudeRuntime = {
    ...gooseRuntime,
    id: "claude-acp",
    label: "Claude",
    command: "claude-acp",
    mcpCommand: null,
  };
  const plan = runtimeForTeamPersonaDeploy({
    persona: persona({ runtime: "claude-acp" }),
    runtimes: [claudeRuntime, buzzAgentRuntime],
    defaultProvider: buzzAgentRuntime,
    managedAgents: [
      managedAgent({
        agentCommandOverride: "/opt/bin/claude-code-acp",
        agentArgs: [],
      }),
    ],
  });
  assert.equal(plan.status, "ready");
  if (plan.status !== "ready") {
    return;
  }
  assert.equal(plan.runtime.id, "claude-acp");
  assert.equal(plan.agentCommand, "/opt/bin/claude-code-acp");
});

test("unresolved local pin is Setup required, not a silent default fallback", () => {
  const plan = runtimeForTeamPersonaDeploy({
    persona: persona(),
    runtimes: [gooseRuntime, buzzAgentRuntime],
    defaultProvider: buzzAgentRuntime,
    managedAgents: [
      managedAgent({
        agentCommandOverride: "/home/user/.local/bin/openclaw-acp-buzz",
        agentArgs: ["--acp"],
      }),
    ],
  });
  assert.equal(plan.status, "setup-required");
  if (plan.status !== "setup-required") {
    return;
  }
  assert.match(plan.reason, /Setup required/);
});

test("team deploy without override falls back to persona runtime with empty args", () => {
  const plan = runtimeForTeamPersonaDeploy({
    persona: persona(),
    runtimes: [gooseRuntime, buzzAgentRuntime],
    defaultProvider: buzzAgentRuntime,
    managedAgents: [
      managedAgent({ agentCommandOverride: null, agentArgs: [] }),
    ],
  });
  assert.equal(plan.status, "ready");
  if (plan.status !== "ready") {
    return;
  }
  assert.equal(plan.harnessOverride, false);
  assert.equal(plan.runtime.id, "goose");
  assert.equal(plan.agentCommand, "goose-cmd");
  assert.deepEqual(plan.agentArgs, []);
});

test("source configuration ignores provider-backed and team clones", () => {
  const source = sourceConfigurationForPersona(
    [
      managedAgent({
        pubkey: "remote",
        backend: { type: "provider", id: "blox", config: {} },
        agentCommandOverride: "/remote/openclaw",
      }),
      managedAgent({ pubkey: "team-clone", teamId: "t-1" }),
      managedAgent({ pubkey: "pinned" }),
    ],
    "p-1",
  );
  assert.equal(source.status, "ready");
  if (source.status === "ready") {
    assert.equal(source.source.pubkey, "pinned");
  }
});

test("conflicting personal instance settings fail closed in either record order", () => {
  const goose = managedAgent({
    pubkey: "zz",
    name: "Zulu",
    agentCommandOverride: "goose-cmd",
    agentArgs: ["--profile", "goose-work"],
  });
  const codex = managedAgent({
    pubkey: "aa",
    name: "Alpha",
    agentCommandOverride: "codex-acp",
    agentArgs: ["--sandbox", "workspace-write"],
  });

  for (const managedAgents of [
    [goose, codex],
    [codex, goose],
  ]) {
    const source = sourceConfigurationForPersona(managedAgents, "p-1");
    assert.equal(source.status, "ambiguous");

    const plan = runtimeForTeamPersonaDeploy({
      persona: persona(),
      runtimes: [gooseRuntime, buzzAgentRuntime],
      defaultProvider: buzzAgentRuntime,
      managedAgents,
    });
    assert.equal(plan.status, "setup-required");
    if (plan.status === "setup-required") {
      assert.match(plan.reason, /different runtime settings/);
    }
  }
});

test("identical personal instance settings are unambiguous in either record order", () => {
  const first = managedAgent({ pubkey: "zz", name: "Zulu" });
  const second = managedAgent({ pubkey: "aa", name: "Alpha" });

  for (const managedAgents of [
    [first, second],
    [second, first],
  ]) {
    const source = sourceConfigurationForPersona(managedAgents, "p-1");
    assert.equal(source.status, "ready");
    if (source.status === "ready") {
      assert.equal(source.source.agentCommandOverride, "goose-cmd");
      assert.deepEqual(source.source.agentArgs, ["--acp"]);
    }
  }
});

test("provider-backed pin is not copied onto a local clone", () => {
  const plan = runtimeForTeamPersonaDeploy({
    persona: persona(),
    runtimes: [gooseRuntime, buzzAgentRuntime],
    defaultProvider: buzzAgentRuntime,
    managedAgents: [
      managedAgent({
        backend: { type: "provider", id: "blox", config: {} },
        agentCommandOverride: "/remote/openclaw",
      }),
    ],
  });
  assert.equal(plan.status, "ready");
  if (plan.status !== "ready") {
    return;
  }
  assert.equal(plan.harnessOverride, false);
  assert.equal(plan.runtime.id, "goose");
});
