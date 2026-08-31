import assert from "node:assert/strict";
import test from "node:test";
import { fromRawManagedAgent } from "./managedAgentWire.ts";
import { startManagedAgent } from "./tauriManagedAgents.ts";
import {
  startManagedAgentWithRules,
  respawnManagedAgentWithRules,
} from "../../features/agents/lib/managedAgentControlActions.ts";

// Cross the consumed wire -> shared summary/profile actions -> real IPC wrapper,
// not an injected Start callback that could accidentally discard the scope.
const raw = (overrides = {}) => ({
  pubkey: "ab".repeat(32),
  selected_relay_url: "wss://clicked.example",
  selected_run_id: null,
  relay_url: "wss://legacy-pin.example",
  backend: { type: "local" },
  status: "stopped",
  ...overrides,
});

async function withIpc(handler, run) {
  const previous = globalThis.window;
  globalThis.window = { __TAURI_INTERNALS__: { invoke: handler } };
  try {
    await run();
  } finally {
    globalThis.window = previous;
  }
}

const start = ({ pubkey, expectedRelayUrl }) =>
  startManagedAgent(pubkey, { expectedRelayUrl });

for (const backend of [
  { type: "local" },
  { type: "provider", id: "test", config: {} },
]) {
  test(`unstarted ${backend.type} summary carries selected workspace through IPC without a run`, async () => {
    const summary = raw({ backend });
    const calls = [];
    await withIpc(
      async (command, args) => {
        calls.push({ command, args });
        return summary;
      },
      async () => {
        await startManagedAgentWithRules({
          agent: fromRawManagedAgent(summary),
          startManagedAgent: start,
        });
      },
    );
    assert.deepEqual(calls, [
      {
        command: "start_managed_agent",
        args: {
          pubkey: summary.pubkey,
          expectedRelayUrl: summary.selected_relay_url,
          expectedSignerPubkey: null,
        },
      },
    ]);
  });
}

test("missing wire scope never falls back to the legacy pin or invokes Start", async () => {
  const summary = raw();
  delete summary.selected_relay_url;
  await withIpc(
    async () => assert.fail("must not invoke"),
    async () => {
      await assert.rejects(
        startManagedAgentWithRules({
          agent: fromRawManagedAgent(summary),
          startManagedAgent: start,
        }),
        /without a selected community/,
      );
    },
  );
});

test("Restart keeps the clicked wire scope through Stop and real Start IPC", async () => {
  const agent = fromRawManagedAgent(
    raw({ status: "running", selected_run_id: "clicked-run" }),
  );
  await withIpc(
    async (command, args) => {
      assert.equal(command, "start_managed_agent");
      assert.equal(args.expectedRelayUrl, "wss://clicked.example");
      return raw();
    },
    async () => {
      await respawnManagedAgentWithRules({
        agent,
        startManagedAgent: start,
        stopManagedAgent: async (input) => {
          assert.equal(input.selectedRunId, "clicked-run");
          assert.equal(input.expectedRelayUrl, "wss://clicked.example");
          await Promise.resolve();
          agent.selectedRelayUrl = "wss://other.example";
        },
      });
    },
  );
});
