import assert from "node:assert/strict";
import test from "node:test";

import {
  getManagedAgentPrimaryActionLabel,
  isManagedAgentLive,
  managedAgentPresence,
  startManagedAgentWithRules,
  respawnManagedAgentWithRules,
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

// ── Remote-agent liveness (I3: "Presence is the status") ─────────────────────
// Regression coverage for #4730: a provider-backed agent that has shut down kept reading as
// live because status is derived from backend_agent_id, which v1 never clears.

const remote = (overrides = {}) =>
  agent({
    backend: { type: "provider", id: "mjolnir" },
    backendAgentId: "vm-1234",
    status: "deployed",
    ...overrides,
  });

const presenceOf = (status, loaded = true) => ({ status, loaded });

test("remote agent with no presence reads as not live once presence has loaded", () => {
  // get_presence omits offline pubkeys, so "shut down" arrives as an absent entry.
  assert.equal(isManagedAgentLive(remote(), presenceOf(undefined)), false);
  assert.equal(isManagedAgentLive(remote(), presenceOf("offline")), false);
});

test("remote agent that is online or away reads as live", () => {
  assert.equal(isManagedAgentLive(remote(), presenceOf("online")), true);
  assert.equal(isManagedAgentLive(remote(), presenceOf("away")), true);
});

test("remote agent does not flash dead while presence is still loading", () => {
  // Falling back to the control-plane axis here keeps I3's promise of a *bounded* wrong
  // signal instead of trading one unbounded lie for another.
  assert.equal(
    isManagedAgentLive(remote(), presenceOf(undefined, false)),
    true,
  );
});

test("remote agent that was never deployed is not live regardless of presence", () => {
  const undeployed = remote({ backendAgentId: null, status: "not_deployed" });
  assert.equal(isManagedAgentLive(undeployed, presenceOf("online")), false);
});

test("local agents keep using the pid-probed status, not presence", () => {
  const running = agent({ status: "running" });
  // A local agent mid-start may have no presence yet; it is still running.
  assert.equal(isManagedAgentLive(running, presenceOf(undefined)), true);
  assert.equal(
    isManagedAgentLive(agent({ status: "stopped" }), presenceOf("online")),
    false,
  );
});

test("shut-down remote agent offers Deploy, not Shutdown", () => {
  // The bug: this returned "Shutdown" forever, making the deploy arm unreachable.
  assert.equal(
    getManagedAgentPrimaryActionLabel(remote(), presenceOf(undefined)),
    "Deploy",
  );
  assert.equal(
    getManagedAgentPrimaryActionLabel(remote(), presenceOf("online")),
    "Shutdown",
  );
});

test("local agent action labels are unchanged", () => {
  const p = presenceOf(undefined);
  assert.equal(
    getManagedAgentPrimaryActionLabel(agent({ status: "running" }), p),
    "Stop",
  );
  assert.equal(
    getManagedAgentPrimaryActionLabel(agent({ status: "stopped" }), p),
    "Restart Agent",
  );
  assert.equal(
    getManagedAgentPrimaryActionLabel(agent({ status: "not_deployed" }), p),
    "Start Agent",
  );
});

test("managedAgentPresence distinguishes an unloaded lookup from an absent entry", () => {
  const a = remote();
  assert.deepEqual(managedAgentPresence(a, undefined), {
    status: undefined,
    loaded: false,
  });
  assert.deepEqual(managedAgentPresence(a, {}), {
    status: undefined,
    loaded: true,
  });
  assert.deepEqual(
    managedAgentPresence(a, { [a.pubkey.toLowerCase()]: "online" }),
    { status: "online", loaded: true },
  );
});
