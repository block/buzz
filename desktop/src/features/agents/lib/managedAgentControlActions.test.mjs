import assert from "node:assert/strict";
import test from "node:test";

import {
  startManagedAgentWithRules,
  respawnManagedAgentWithRules,
} from "./managedAgentControlActions.ts";

function agent(overrides = {}) {
  return {
    pubkey: "deadbeef".repeat(8),
    name: "Mesh Agent",
    personaId: null,
    relayUrl: "ws://localhost:3000",
    selectedRelayUrl: "ws://localhost:3000",
    acpCommand: "buzz-acp",
    agentCommand: "goose",
    agentArgs: [],
    mcpCommand: "",
    turnTimeoutSeconds: 320,
    idleTimeoutSeconds: null,
    maxTurnDurationSeconds: null,
    parallelism: 1,
    systemPrompt: null,
    model: "hf://demo/model.gguf",
    envVars: {},
    status: "stopped",
    pid: null,
    createdAt: new Date(0).toISOString(),
    updatedAt: new Date(0).toISOString(),
    lastStartedAt: null,
    lastStoppedAt: null,
    lastExitCode: null,
    lastError: null,
    logPath: null,
    startOnAppLaunch: false,
    backend: { type: "local" },
    backendAgentId: null,
    respondTo: "owner-only",
    respondToAllowlist: [],
    ...overrides,
  };
}

test("relay-mesh agents delegate start to the backend preflight", async () => {
  const meshAgent = agent({
    envVars: {
      BUZZ_AGENT_PROVIDER: "openai",
      OPENAI_COMPAT_BASE_URL: "http://127.0.0.1:9337/v1/",
    },
  });

  let calledWith = null;
  await startManagedAgentWithRules({
    agent: meshAgent,
    startManagedAgent: async (pubkey) => {
      calledWith = pubkey;
    },
  });
  assert.deepEqual(calledWith, {
    pubkey: meshAgent.pubkey,
    expectedRelayUrl: meshAgent.selectedRelayUrl,
  });

  // Backend preflight failures (e.g. no live serve target) propagate as-is.
  await assert.rejects(
    startManagedAgentWithRules({
      agent: meshAgent,
      startManagedAgent: async () => {
        throw new Error("no live serve target is available for this model");
      },
    }),
    /no live serve target/,
  );
});

test("ordinary local agents still start normally", async () => {
  let calledWith = null;
  await startManagedAgentWithRules({
    agent: agent(),
    startManagedAgent: async (pubkey) => {
      calledWith = pubkey;
    },
  });
  assert.deepEqual(calledWith, {
    pubkey: "deadbeef".repeat(8),
    expectedRelayUrl: "ws://localhost:3000",
  });
});

// --- respawnManagedAgentWithRules: stop→clear→start boundary tests -----------

test("test_respawn_stop_success_start_failure_onStopped_still_fires", async () => {
  // Prove: onStopped fires at the stop-success boundary even when start later
  // throws.  This is the key discriminator: on round-1 code the clear only
  // ran after the full respawn, so a failed start left the badge intact.
  const runningAgent = agent({ status: "running" });
  let onStoppedFired = false;

  await assert.rejects(
    respawnManagedAgentWithRules({
      agent: runningAgent,
      stopManagedAgent: async () => {
        /* stop succeeds */
      },
      startManagedAgent: async () => {
        throw new Error("start failed");
      },
      onStopped: () => {
        onStoppedFired = true;
      },
    }),
    /start failed/,
  );

  assert.ok(
    onStoppedFired,
    "onStopped must fire at stop-success boundary even when start subsequently fails",
  );
});

test("test_respawn_stop_failure_onStopped_not_called", async () => {
  // Prove: onStopped does NOT fire when stop itself throws.  Clearing on a
  // failed stop would remove a badge that is still legitimately active.
  const runningAgent = agent({ status: "running" });
  let onStoppedFired = false;

  await assert.rejects(
    respawnManagedAgentWithRules({
      agent: runningAgent,
      stopManagedAgent: async () => {
        throw new Error("stop failed");
      },
      startManagedAgent: async () => {
        /* should not be reached */
      },
      onStopped: () => {
        onStoppedFired = true;
      },
    }),
    /stop failed/,
  );

  assert.ok(
    !onStoppedFired,
    "onStopped must NOT fire when stop itself fails — badge is still active",
  );
});

test("test_respawn_onStopped_fires_before_start_resolves", async () => {
  // Prove: onStopped fires strictly between stop resolution and start
  // invocation.  A clear that fires after start begins can tombstone genuine
  // new turns from the freshly spawned process.
  const runningAgent = agent({ status: "running" });
  const events = [];

  await respawnManagedAgentWithRules({
    agent: runningAgent,
    stopManagedAgent: async () => {
      events.push("stop");
    },
    startManagedAgent: async () => {
      events.push("start");
    },
    onStopped: () => {
      events.push("onStopped");
    },
  });

  assert.deepEqual(
    events,
    ["stop", "onStopped", "start"],
    "onStopped must fire after stop resolves and before start is called",
  );
});

test("ordinary Stop preserves clicked generation even when a successor appears", async () => {
  const { stopManagedAgentWithRules } = await import(
    "./managedAgentControlActions.ts"
  );
  const clicked = agent({
    status: "running",
    selectedRunId: "aa".repeat(16),
    selectedRelayUrl: "wss://clicked.example",
  });
  let request;
  await stopManagedAgentWithRules({
    agent: clicked,
    channels: [],
    relayAgents: [],
    stopManagedAgent: async (selected) => {
      clicked.selectedRunId = "bb".repeat(16);
      clicked.selectedRelayUrl = "wss://successor.example";
      request = selected;
    },
  });
  assert.deepEqual(request, {
    pubkey: clicked.pubkey,
    selectedRunId: "aa".repeat(16),
    expectedRelayUrl: "wss://clicked.example",
  });
});

test("Restart retains clicked community across the Stop await", async () => {
  const clicked = agent({ status: "running", selectedRunId: "aa".repeat(16) });
  const original = clicked.selectedRelayUrl;
  let stop, start;
  await respawnManagedAgentWithRules({
    agent: clicked,
    stopManagedAgent: async (input) => {
      stop = input;
      await Promise.resolve();
      clicked.selectedRelayUrl = "wss://switched.example";
    },
    startManagedAgent: async (input) => {
      start = input;
    },
  });
  assert.equal(stop.expectedRelayUrl, original);
  assert.equal(start.expectedRelayUrl, original);
  assert.equal(stop.selectedRunId, "aa".repeat(16));
});

test("missing community fails before either Restart leg, never falls back to record pin", async () => {
  let calls = 0;
  await assert.rejects(
    respawnManagedAgentWithRules({
      agent: agent({ status: "running", selectedRelayUrl: null }),
      stopManagedAgent: async () => {
        calls++;
      },
      startManagedAgent: async () => {
        calls++;
      },
    }),
    /selected community/,
  );
  assert.equal(calls, 0);
});
