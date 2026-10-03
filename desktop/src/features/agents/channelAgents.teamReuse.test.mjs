import assert from "node:assert/strict";
import test from "node:test";

import { createChannelManagedAgents } from "./channelAgents.ts";

const AGENT_PUBKEY = "a".repeat(64);

function rawAgent(overrides = {}) {
  return {
    pubkey: AGENT_PUBKEY,
    name: "corsai",
    persona_id: "persona-corsai",
    runtime: "codex",
    team_id: "team-dpwai",
    relay_url: "wss://relay.example",
    acp_command: "buzz-acp",
    agent_command: "codex-acp",
    agent_args: [],
    mcp_command: "",
    turn_timeout_seconds: 0,
    idle_timeout_seconds: 0,
    max_turn_duration_seconds: 0,
    parallelism: 1,
    system_prompt: "Support the team.",
    model: null,
    provider: null,
    persona_out_of_date: false,
    persona_orphaned: false,
    needs_restart: false,
    status: "running",
    pid: null,
    created_at: "2026-01-15T00:00:00Z",
    updated_at: "2026-01-15T00:00:00Z",
    last_started_at: null,
    last_stopped_at: null,
    last_exit_code: null,
    last_error: null,
    last_error_code: null,
    log_path: "",
    start_on_app_launch: true,
    backend: { type: "local" },
    backend_agent_id: null,
    respond_to: "owner-only",
    respond_to_allowlist: [],
    ...overrides,
  };
}

function input() {
  return {
    runtime: {
      id: "codex",
      label: "Codex",
      command: "codex-acp",
      defaultArgs: [],
      mcpCommand: "",
    },
    name: "corsai",
    personaId: "persona-corsai",
    teamId: "team-dpwai",
    systemPrompt: "Support the team.",
    respondTo: "owner-only",
    ensureRunning: false,
  };
}

function installTauriInvoke(handler) {
  const prior = globalThis.window;
  globalThis.window ??= {};
  window.__TAURI_INTERNALS__ = { invoke: handler };
  return () => {
    globalThis.window = prior;
  };
}

test("re-deploying a team reuses its persona already in the channel", async (t) => {
  const commands = [];
  t.after(
    installTauriInvoke((command) => {
      commands.push(command);
      if (command === "list_managed_agents")
        return Promise.resolve([rawAgent()]);
      if (command === "get_channel_members") {
        return Promise.resolve({
          members: [
            {
              pubkey: AGENT_PUBKEY,
              role: "bot",
              is_agent: true,
              joined_at: "2026-01-15T00:00:00Z",
              display_name: "corsai",
            },
          ],
          next_cursor: null,
        });
      }
      if (command === "add_channel_members") {
        return Promise.resolve({ added: [], errors: [] });
      }
      throw new Error(`Unexpected Tauri command: ${command}`);
    }),
  );

  const result = await createChannelManagedAgents("channel-1", [input()]);

  assert.equal(result.failures.length, 0);
  assert.equal(result.successes.length, 1);
  assert.equal(result.successes[0].created, false);
  assert.equal(result.successes[0].agent.pubkey, AGENT_PUBKEY);
  assert.equal(commands.includes("create_managed_agent"), false);
});

test("one batch never mints two keys for the same persona", async (t) => {
  const commands = [];
  t.after(
    installTauriInvoke((command) => {
      commands.push(command);
      if (command === "list_managed_agents") return Promise.resolve([]);
      if (command === "get_channel_members") {
        return Promise.resolve({ members: [], next_cursor: null });
      }
      if (command === "create_managed_agent") {
        return Promise.resolve({
          agent: rawAgent(),
          private_key_nsec: "nsec-test",
          profile_sync_error: null,
          spawn_error: null,
        });
      }
      if (command === "add_channel_members") {
        return Promise.resolve({ added: [AGENT_PUBKEY], errors: [] });
      }
      throw new Error(`Unexpected Tauri command: ${command}`);
    }),
  );

  const result = await createChannelManagedAgents("channel-1", [
    input(),
    input(),
  ]);

  assert.equal(result.failures.length, 0);
  assert.equal(result.successes.length, 2);
  assert.equal(result.successes[0].created, true);
  assert.equal(result.successes[1].created, false);
  assert.equal(
    commands.filter((command) => command === "create_managed_agent").length,
    1,
  );
});

test("an explicit fresh-instance request still mints a new key", async (t) => {
  const commands = [];
  t.after(
    installTauriInvoke((command) => {
      commands.push(command);
      if (command === "list_managed_agents")
        return Promise.resolve([rawAgent()]);
      if (command === "get_channel_members") {
        return Promise.resolve({
          members: [
            {
              pubkey: AGENT_PUBKEY,
              role: "bot",
              is_agent: true,
              joined_at: "2026-01-15T00:00:00Z",
              display_name: "corsai",
            },
          ],
          next_cursor: null,
        });
      }
      if (command === "create_managed_agent") {
        return Promise.resolve({
          agent: rawAgent({ pubkey: "b".repeat(64) }),
          private_key_nsec: "nsec-fresh-test",
          profile_sync_error: null,
          spawn_error: null,
        });
      }
      if (command === "add_channel_members") {
        return Promise.resolve({ added: ["b".repeat(64)], errors: [] });
      }
      throw new Error(`Unexpected Tauri command: ${command}`);
    }),
  );

  const freshInput = { ...input(), forceNewInstance: true };
  const result = await createChannelManagedAgents("channel-1", [freshInput]);

  assert.equal(result.failures.length, 0);
  assert.equal(result.successes.length, 1);
  assert.equal(result.successes[0].created, true);
  assert.equal(result.successes[0].agent.pubkey, "b".repeat(64));
  assert.equal(
    commands.filter((command) => command === "create_managed_agent").length,
    1,
  );
});
