import assert from "node:assert/strict";
import test from "node:test";

import { applyReusableAgentAccessPolicy } from "./channelAgents.ts";

const AGENT_PUBKEY = "a".repeat(64);
const ALLOWED_PUBKEY = "b".repeat(64);

// `wrote` is load-bearing: the message-send path (useMentionSendFlow) uses it
// to decide whether an awaited relay round-trip separated its pre-side-effect
// mention-authorization pass from the publish, and therefore whether it must
// revalidate at the publish boundary (#5681). These tests pin the flag against
// the relay write itself, not against the identity of the returned record.

function rawAgent(overrides = {}) {
  return {
    pubkey: AGENT_PUBKEY,
    name: "fizz",
    persona_id: null,
    relay_url: "wss://relay.example",
    acp_command: "buzz-acp",
    agent_command: "goose",
    agent_args: [],
    mcp_command: "",
    turn_timeout_seconds: 0,
    idle_timeout_seconds: 0,
    max_turn_duration_seconds: 0,
    parallelism: 1,
    system_prompt: null,
    model: null,
    status: "running",
    pid: null,
    created_at: "2026-01-15T00:00:00Z",
    updated_at: "2026-01-15T00:00:00Z",
    last_started_at: null,
    last_stopped_at: null,
    last_exit_code: null,
    last_error: null,
    log_path: null,
    start_on_app_launch: false,
    backend: { type: "local" },
    backend_agent_id: null,
    respond_to: "owner-only",
    respond_to_allowlist: [],
    ...overrides,
  };
}

function managedAgent(overrides = {}) {
  return {
    pubkey: AGENT_PUBKEY,
    name: "fizz",
    respondTo: "owner-only",
    respondToAllowlist: [],
    ...overrides,
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

test("a matching access policy reports no write and returns the agent untouched", async (t) => {
  const calls = [];
  t.after(
    installTauriInvoke((command, args) => {
      calls.push([command, args]);
      return Promise.resolve(null);
    }),
  );

  const agent = managedAgent();
  const result = await applyReusableAgentAccessPolicy(agent, {});

  assert.equal(result.wrote, false);
  assert.equal(result.agent, agent);
  assert.deepEqual(calls, []);
});

test("a diverging access policy reports the write and returns the updated agent", async (t) => {
  const calls = [];
  t.after(
    installTauriInvoke((command, args) => {
      calls.push([command, args]);
      return Promise.resolve({
        agent: rawAgent({
          respond_to: "allowlist",
          respond_to_allowlist: [ALLOWED_PUBKEY],
        }),
        profile_sync_error: null,
      });
    }),
  );

  const agent = managedAgent();
  const result = await applyReusableAgentAccessPolicy(agent, {
    respondTo: "allowlist",
    respondToAllowlist: [ALLOWED_PUBKEY],
  });

  assert.equal(result.wrote, true);
  assert.equal(result.agent.respondTo, "allowlist");
  assert.deepEqual(result.agent.respondToAllowlist, [ALLOWED_PUBKEY]);
  assert.deepEqual(calls, [
    [
      "update_managed_agent",
      {
        input: {
          pubkey: AGENT_PUBKEY,
          respondTo: "allowlist",
          respondToAllowlist: [ALLOWED_PUBKEY],
        },
      },
    ],
  ]);
});

test("the write is reported even when the update hands back an unchanged record", async (t) => {
  // Callers must not re-derive the write by comparing the returned record
  // against the one they passed in — a backend that normalizes the policy
  // away, or a cache layer that mutates in place and hands the caller's own
  // object back, still wrote to the relay. Under such a comparison the send
  // path would silently skip the publish-boundary revalidation.
  let invoked = 0;
  t.after(
    installTauriInvoke(() => {
      invoked += 1;
      return Promise.resolve({
        agent: rawAgent(),
        profile_sync_error: null,
      });
    }),
  );

  const agent = managedAgent();
  const result = await applyReusableAgentAccessPolicy(agent, {
    respondTo: "anyone",
  });

  assert.equal(invoked, 1);
  assert.equal(result.wrote, true);
  assert.equal(result.agent.respondTo, agent.respondTo);
  assert.deepEqual(result.agent.respondToAllowlist, agent.respondToAllowlist);
});
