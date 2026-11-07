import assert from "node:assert/strict";
import test from "node:test";

import { attachManagedAgentToChannel } from "./channelAgents.ts";

const AGENT_PUBKEY = "a".repeat(64);
const CHANNEL_ID = "11111111-2222-3333-4444-555555555555";

const { relayClient } = await import("../../shared/api/relayClient.ts");

function installTauriInvoke(handler) {
  const prior = globalThis.window;
  globalThis.window ??= {};
  window.__TAURI_INTERNALS__ = { invoke: handler };
  return () => {
    globalThis.window = prior;
  };
}

function deployedProvider() {
  return {
    pubkey: AGENT_PUBKEY,
    name: "slurms",
    status: "deployed",
    backend: { type: "provider", id: "kubernetes", config: {} },
    backendAgentId: "buzz-agent-aaaaaaaaaaaa",
  };
}

/** Attach once against a relay presence answer; return what it woke. */
async function wakesFor(t, { presence, connection = "connected" }) {
  const originalState = relayClient.getConnectionState;
  relayClient.getConnectionState = () => connection;
  const commands = [];
  const restore = installTauriInvoke((command) => {
    commands.push(command);
    if (command === "add_channel_members") {
      return Promise.resolve({ added: [AGENT_PUBKEY], errors: [] });
    }
    if (command === "get_presence") {
      if (presence instanceof Error) return Promise.reject(presence);
      return Promise.resolve(presence ? { [AGENT_PUBKEY]: presence } : {});
    }
    return Promise.resolve(null);
  });
  t.after(() => {
    restore();
    relayClient.getConnectionState = originalState;
  });

  const woken = [];
  await attachManagedAgentToChannel(CHANNEL_ID, {
    agent: deployedProvider(),
    detachedStart: (agent) => woken.push(agent.pubkey),
  });
  assert.ok(
    !commands.includes("start_managed_agent"),
    "a detached caller owns the start; attach never fires it",
  );
  return woken;
}

// The `deployed` receipt outlives a shut-down or inactivity-reaped pod; the
// attach reads presence itself, and only established offline means redeploy.
test("attach redeploys a deployed agent that presence shows offline", async (t) => {
  // A successful snapshot that omits the agent is established offline.
  assert.deepEqual(await wakesFor(t, { presence: null }), [AGENT_PUBKEY]);
});

test("attach trusts the receipt while the agent is present", async (t) => {
  assert.deepEqual(await wakesFor(t, { presence: "online" }), []);
});

test("attach never invents an offline agent from an unknown read", async (t) => {
  assert.deepEqual(
    await wakesFor(t, { presence: new Error("relay unavailable") }),
    [],
  );
  assert.deepEqual(
    await wakesFor(t, { presence: null, connection: "disconnected" }),
    [],
  );
});
