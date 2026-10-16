import assert from "node:assert/strict";
import test from "node:test";

import {
  displayedManagedAgentStatus,
  getManagedAgentPrimaryActionLabel,
  isManagedAgentActive,
  isManagedAgentLive,
  respawnManagedAgentWithRules,
  startManagedAgentWithRules,
} from "./managedAgentControlActions.ts";

function agent(overrides = {}) {
  return {
    pubkey: "deadbeef".repeat(8),
    name: "Mesh Agent",
    personaId: null,
    relayUrl: "ws://localhost:3000",
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

test("a deployed provider agent established offline offers Deploy again", () => {
  const deployed = agent({
    status: "deployed",
    backend: { type: "provider", id: "kubernetes", config: {} },
    backendAgentId: "buzz-agent-deadbeef",
  });

  // The receipt outlives `!shutdown` and inactivity reaping, so offline
  // presence must route the primary action back to Deploy.
  assert.equal(isManagedAgentLive(deployed, "offline"), false);
  assert.equal(
    getManagedAgentPrimaryActionLabel(deployed, "offline"),
    "Deploy",
  );
  for (const availability of ["online", "away", undefined]) {
    assert.equal(isManagedAgentLive(deployed, availability), true);
    assert.equal(
      getManagedAgentPrimaryActionLabel(deployed, availability),
      "Shutdown",
    );
  }
});

test("liveness over every status and availability", () => {
  // Only a `deployed` receipt yields to presence, and only to established
  // offline. A process status (running/stopped) and a missing deploy are never
  // overridden: positive presence alone grants no lifecycle authority.
  const expected = {
    running: { online: true, away: true, offline: true, unknown: true },
    stopped: { online: false, away: false, offline: false, unknown: false },
    deployed: { online: true, away: true, offline: false, unknown: true },
    not_deployed: {
      online: false,
      away: false,
      offline: false,
      unknown: false,
    },
  };
  for (const [status, row] of Object.entries(expected)) {
    for (const [presence, active] of Object.entries(row)) {
      const availability = presence === "unknown" ? undefined : presence;
      assert.equal(
        isManagedAgentLive(agent({ status }), availability),
        active,
        `${status} × ${presence}`,
      );
    }
  }
});

test("control-plane activity stays receipt-only", () => {
  // Bookkeeping callers (e.g. the local-only respawn guard) must keep reading
  // the receipt; liveness is isManagedAgentLive's job.
  for (const [status, active] of [
    ["running", true],
    ["deployed", true],
    ["stopped", false],
    ["not_deployed", false],
  ]) {
    assert.equal(isManagedAgentActive(agent({ status })), active, status);
  }
});

test("displayed status over every status and availability", () => {
  // Only an established-offline `deployed` receipt changes what is shown.
  for (const status of ["running", "stopped", "deployed", "not_deployed"]) {
    for (const availability of ["online", "away", "offline", undefined]) {
      const expected =
        status === "deployed" && availability === "offline"
          ? "stopped"
          : status;
      assert.equal(
        displayedManagedAgentStatus(status, availability),
        expected,
        `${status} × ${availability ?? "unknown"}`,
      );
    }
  }
});

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
  assert.equal(calledWith, meshAgent.pubkey);

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
  assert.equal(calledWith, "deadbeef".repeat(8));
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

// --- respawn bound to the workspace relay -----------------------------------

const { markRelayRemoved } = await import("../managedAgentRelayCleanup.ts");

async function respawnAcrossStop(relayUrl, duringStop) {
  const started = [];
  const result = respawnManagedAgentWithRules({
    agent: agent({ status: "running" }),
    relayUrl,
    stopManagedAgent: async () => duringStop(),
    startManagedAgent: async (pubkey) => {
      started.push(pubkey);
    },
  });
  return { result, started };
}

test("a respawn whose own relay is removed and re-added during its stop starts nothing", async () => {
  const { result, started } = await respawnAcrossStop(
    "wss://respawn-own.example",
    () => markRelayRemoved("wss://respawn-own.example"),
  );
  await assert.rejects(result, /relay was removed from this device/);
  assert.deepEqual(started, []);
});

test("removing an unrelated relay during a respawn's stop still restarts it", async () => {
  const { result, started } = await respawnAcrossStop(
    "wss://respawn-kept.example",
    () => markRelayRemoved("wss://respawn-unrelated.example"),
  );
  await result;
  assert.equal(started.length, 1);
});
