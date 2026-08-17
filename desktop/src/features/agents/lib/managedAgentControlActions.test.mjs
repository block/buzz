import assert from "node:assert/strict";
import test from "node:test";

import {
  getManagedAgentSecondaryAction,
  REDEPLOY_SENT_NOTICE,
  redeployManagedAgentWithRules,
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

test("deployed provider agents expose redeploy as their secondary action", () => {
  assert.equal(
    getManagedAgentSecondaryAction(
      agent({
        backend: { type: "provider", id: "provider", config: {} },
        backendAgentId: "remote-agent",
        status: "deployed",
      }),
    ),
    "redeploy",
  );
  assert.equal(
    getManagedAgentSecondaryAction(
      agent({
        backend: { type: "provider", id: "provider", config: {} },
        status: "not_deployed",
      }),
    ),
    null,
  );
  assert.equal(
    getManagedAgentSecondaryAction(agent({ status: "running" })),
    "restart",
  );
  assert.equal(
    getManagedAgentSecondaryAction(agent({ status: "stopped" })),
    null,
  );
});

/**
 * The claim boundary the reviewer flagged. A provider that answers a deploy
 * with an `agent_id` has proved delivery, not application: the built-in
 * Kubernetes provider's live row is a strict, zero-mutation no-op that returns
 * the existing id (pinned in `crates/buzz-backend-kubernetes/src/reconcile.rs`,
 * `started_pod_returns_its_id_having_applied_nothing`), and
 * `docs/remote-agents.md` says edits reach a running agent only on its next
 * fresh generation. So the notice may report the send and the generation
 * boundary, and must not claim the agent was redeployed or updated.
 */
test("a successful redeploy reports delivery, never a proven runtime change", async () => {
  const deployed = agent({
    backend: { type: "provider", id: "kubernetes", config: {} },
    backendAgentId: "buzz-agent-abc123",
    status: "deployed",
  });

  let calledWith = null;
  const result = await redeployManagedAgentWithRules({
    agent: deployed,
    redeployManagedAgent: async (pubkey) => {
      calledWith = pubkey;
      // What a provider can actually return: an id. It is the same id the
      // live no-op row returns, so it distinguishes nothing.
      return { ...deployed };
    },
  });

  assert.equal(calledWith, deployed.pubkey);
  assert.equal(result.noticeMessage, REDEPLOY_SENT_NOTICE);
  assert.match(result.noticeMessage, /sent to the provider/i);
  assert.match(result.noticeMessage, /until it next restarts/i);
  for (const forbidden of [
    /redeployed/i,
    /\bapplied\b/i,
    /now running/i,
    /updated the agent/i,
  ]) {
    assert.doesNotMatch(
      result.noticeMessage,
      forbidden,
      `redeploy notice must not claim a proven runtime change: ${forbidden}`,
    );
  }
});

test("redeploy refuses an agent that has no provider deployment", async () => {
  let called = false;
  await assert.rejects(
    redeployManagedAgentWithRules({
      agent: agent({
        backend: { type: "provider", id: "kubernetes", config: {} },
        backendAgentId: null,
        status: "not_deployed",
      }),
      redeployManagedAgent: async () => {
        called = true;
      },
    }),
    /not deployed on a provider/i,
  );
  assert.equal(called, false, "refused redeploy still called the backend");
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
