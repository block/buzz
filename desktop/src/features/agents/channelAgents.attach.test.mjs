import assert from "node:assert/strict";
import test from "node:test";

import { attachManagedAgentToChannel } from "./channelAgents.ts";

const AGENT_PUBKEY = "a".repeat(64);
const CHANNEL_ID = "11111111-2222-3333-4444-555555555555";

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

test("attaching a deployed provider agent redeploys only once presence shows it gone", async (t) => {
  const commands = [];
  t.after(
    installTauriInvoke((command) => {
      commands.push(command);
      return Promise.resolve(
        command === "add_channel_members"
          ? { added: [AGENT_PUBKEY], errors: [] }
          : null,
      );
    }),
  );

  const startsFor = async (availability) => {
    const woken = [];
    await attachManagedAgentToChannel(CHANNEL_ID, {
      agent: deployedProvider(),
      availability,
      detachedStart: (agent) => woken.push(agent.pubkey),
    });
    return woken;
  };

  // The `deployed` receipt outlives a shut-down or inactivity-reaped pod;
  // established offline presence is what says the body is gone.
  assert.deepEqual(await startsFor("offline"), [AGENT_PUBKEY]);
  assert.deepEqual(await startsFor("online"), []);
  assert.deepEqual(
    await startsFor(undefined),
    [],
    "unknown presence keeps trusting the receipt",
  );
  assert.ok(
    !commands.includes("start_managed_agent"),
    "a detached caller owns the start; attach never fires it",
  );
});
