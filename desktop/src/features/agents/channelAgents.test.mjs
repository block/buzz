import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { attachManagedAgentToChannel } from "./channelAgents.ts";

const AGENT_PUBKEY = "a".repeat(64);

function rawAgent(status = "stopped", backend = { type: "local" }) {
  return {
    pubkey: AGENT_PUBKEY,
    name: "reviewer",
    persona_id: null,
    runtime: null,
    team_id: null,
    relay_url: "ws://localhost:3000",
    acp_command: "buzz-acp",
    agent_command: "codex",
    agent_command_override: null,
    agent_args: [],
    mcp_command: "",
    turn_timeout_seconds: 30,
    idle_timeout_seconds: null,
    max_turn_duration_seconds: null,
    parallelism: 1,
    system_prompt: null,
    avatar_url: null,
    model: null,
    provider: null,
    persona_out_of_date: false,
    persona_orphaned: false,
    needs_restart: false,
    restart_diff: [],
    env_vars: {},
    status,
    pid: status === "running" ? 123 : null,
    created_at: "2026-09-01T00:00:00Z",
    updated_at: "2026-09-01T00:00:00Z",
    last_started_at: null,
    last_stopped_at: null,
    last_exit_code: null,
    last_error: null,
    last_error_code: null,
    log_path: null,
    start_on_app_launch: false,
    auto_restart_on_config_change: true,
    backend,
    backend_agent_id: null,
    respond_to: "owner-only",
    respond_to_allowlist: [],
  };
}

function managedAgent(status = "stopped", backend = { type: "local" }) {
  const raw = rawAgent(status, backend);
  return {
    ...raw,
    personaId: raw.persona_id,
    teamId: raw.team_id,
    relayUrl: raw.relay_url,
    acpCommand: raw.acp_command,
    agentCommand: raw.agent_command,
    agentCommandOverride: raw.agent_command_override,
    agentArgs: raw.agent_args,
    mcpCommand: raw.mcp_command,
    turnTimeoutSeconds: raw.turn_timeout_seconds,
    idleTimeoutSeconds: raw.idle_timeout_seconds,
    maxTurnDurationSeconds: raw.max_turn_duration_seconds,
    systemPrompt: raw.system_prompt,
    avatarUrl: raw.avatar_url,
    personaOutOfDate: raw.persona_out_of_date,
    personaOrphaned: raw.persona_orphaned,
    needsRestart: raw.needs_restart,
    restartDiff: raw.restart_diff,
    envVars: raw.env_vars,
    createdAt: raw.created_at,
    updatedAt: raw.updated_at,
    lastStartedAt: raw.last_started_at,
    lastStoppedAt: raw.last_stopped_at,
    lastExitCode: raw.last_exit_code,
    lastError: raw.last_error,
    lastErrorCode: raw.last_error_code,
    logPath: raw.log_path,
    startOnAppLaunch: raw.start_on_app_launch,
    autoRestartOnConfigChange: raw.auto_restart_on_config_change,
    backendAgentId: raw.backend_agent_id,
    respondTo: raw.respond_to,
    respondToAllowlist: raw.respond_to_allowlist,
  };
}

function installTauriInvoke(t, handler) {
  const prior = globalThis.window;
  globalThis.window = { __TAURI_INTERNALS__: { invoke: handler } };
  t.after(() => {
    mock.restoreAll();
    globalThis.window = prior;
  });
}

test("add without starting verifies stopped state through the production seam", async (t) => {
  const calls = [];
  installTauriInvoke(t, async (command, args) => {
    calls.push([command, args]);
    if (command === "add_channel_members") {
      return { added: [AGENT_PUBKEY], errors: [] };
    }
    if (command === "list_managed_agents") {
      return [rawAgent("stopped")];
    }
    throw new Error(`unexpected command: ${command}`);
  });

  const result = await attachManagedAgentToChannel("channel-1", {
    agent: managedAgent("stopped"),
    ensureRunning: false,
  });

  assert.equal(result.membershipAdded, true);
  assert.equal(result.started, false);
  assert.equal(result.agent.status, "stopped");
  assert.deepEqual(
    calls.map(([command]) => command),
    ["add_channel_members", "list_managed_agents"],
  );
});

test("add without starting rejects stale running input before membership", async (t) => {
  const calls = [];
  installTauriInvoke(t, async (command, args) => {
    calls.push([command, args]);
    throw new Error(`unexpected command: ${command}`);
  });

  await assert.rejects(
    attachManagedAgentToChannel("channel-1", {
      agent: managedAgent("running"),
      ensureRunning: false,
    }),
    /no longer stopped/,
  );
  assert.deepEqual(calls, []);
});

test("add without starting fails closed when membership is not confirmed", async (t) => {
  const calls = [];
  installTauriInvoke(t, async (command, args) => {
    calls.push([command, args]);
    if (command === "add_channel_members") {
      return { added: [], errors: [] };
    }
    throw new Error(`unexpected command: ${command}`);
  });

  await assert.rejects(
    attachManagedAgentToChannel("channel-1", {
      agent: managedAgent("stopped"),
      ensureRunning: false,
    }),
    /membership was not confirmed/,
  );
  assert.deepEqual(
    calls.map(([command]) => command),
    ["add_channel_members"],
  );
});

test("post-membership stopped-state drift is a visible partial failure", async (t) => {
  const calls = [];
  installTauriInvoke(t, async (command, args) => {
    calls.push([command, args]);
    if (command === "add_channel_members") {
      return { added: [AGENT_PUBKEY], errors: [] };
    }
    if (command === "list_managed_agents") {
      return [rawAgent("running")];
    }
    throw new Error(`unexpected command: ${command}`);
  });

  await assert.rejects(
    attachManagedAgentToChannel("channel-1", {
      agent: managedAgent("stopped"),
      ensureRunning: false,
    }),
    /was added, but its stopped state could not be verified/,
  );
  assert.deepEqual(
    calls.map(([command]) => command),
    ["add_channel_members", "list_managed_agents"],
  );
});

test("the existing default still adds and starts a stopped local agent", async (t) => {
  const calls = [];
  installTauriInvoke(t, async (command, args) => {
    calls.push([command, args]);
    if (command === "add_channel_members") {
      return { added: [AGENT_PUBKEY], errors: [] };
    }
    if (command === "start_managed_agent") {
      return rawAgent("running");
    }
    throw new Error(`unexpected command: ${command}`);
  });

  const result = await attachManagedAgentToChannel("channel-1", {
    agent: managedAgent("stopped"),
  });

  assert.equal(result.membershipAdded, true);
  assert.equal(result.started, true);
  assert.equal(result.agent.status, "running");
  assert.deepEqual(
    calls.map(([command]) => command),
    ["add_channel_members", "start_managed_agent"],
  );
});
