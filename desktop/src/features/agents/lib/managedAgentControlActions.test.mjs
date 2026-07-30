import assert from "node:assert/strict";
import test from "node:test";

import {
  startManagedAgentWithRules,
  respawnManagedAgentWithRules,
  deleteManagedAgentsForPersonaWithRules,
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

// --- deleteManagedAgentsForPersonaWithRules ---
//
// Contract under test: the persona cascade must abort on the first cancelled
// or failed instance delete, so `deletePersona` is never reached with a
// half-torn persona. Mirrors deleteProfileManagedAgentsForPersona.

function personaAgent(pubkeyChar, overrides = {}) {
  return agent({
    pubkey: pubkeyChar.repeat(64),
    personaId: "persona-1",
    ...overrides,
  });
}

test("persona cascade deletes every instance backed by that persona", async () => {
  const deleted = [];
  const result = await deleteManagedAgentsForPersonaWithRules({
    persona: { id: "persona-1" },
    managedAgents: [personaAgent("a"), personaAgent("b")],
    channels: [],
    relayAgents: [],
    deleteManagedAgent: async ({ pubkey }) => {
      deleted.push(pubkey);
    },
  });

  assert.deepEqual(result, { deletedCount: 2 });
  assert.deepEqual(deleted, ["a".repeat(64), "b".repeat(64)]);
});

test("persona cascade ignores instances backed by other personas", async () => {
  const deleted = [];
  await deleteManagedAgentsForPersonaWithRules({
    persona: { id: "persona-1" },
    managedAgents: [
      personaAgent("a"),
      personaAgent("c", { personaId: "persona-2" }),
    ],
    channels: [],
    relayAgents: [],
    deleteManagedAgent: async ({ pubkey }) => {
      deleted.push(pubkey);
    },
  });

  assert.deepEqual(deleted, ["a".repeat(64)]);
});

test("persona cascade deletes a duplicated instance only once", async () => {
  const deleted = [];
  await deleteManagedAgentsForPersonaWithRules({
    persona: { id: "persona-1" },
    managedAgents: [personaAgent("a"), personaAgent("A"), personaAgent("a")],
    channels: [],
    relayAgents: [],
    deleteManagedAgent: async ({ pubkey }) => {
      deleted.push(pubkey);
    },
  });

  assert.equal(deleted.length, 1, "same pubkey must not be deleted twice");
});

test("persona cascade stops at a declined orphan confirm", async () => {
  const originalWindow = globalThis.window;
  globalThis.window = { confirm: () => false };
  try {
    const deleted = [];
    const result = await deleteManagedAgentsForPersonaWithRules({
      persona: { id: "persona-1" },
      // First instance is provider-deployed and in no channel, so the
      // orphan-warning confirm is what decides the cascade.
      managedAgents: [
        personaAgent("a", {
          backend: { type: "provider" },
          backendAgentId: "remote-1",
        }),
        personaAgent("b"),
      ],
      channels: [],
      relayAgents: [],
      deleteManagedAgent: async ({ pubkey }) => {
        deleted.push(pubkey);
      },
    });

    assert.equal(result.cancelled, true, "declining must report cancelled");
    assert.equal(
      result.deletedCount,
      0,
      "nothing was deleted before the abort",
    );
    assert.deepEqual(
      deleted,
      [],
      "no instance may be deleted once the confirm is declined",
    );
  } finally {
    globalThis.window = originalWindow;
  }
});

test("a declined confirm does not undo instances already deleted", async () => {
  // The aborting instance is deliberately NOT first: a local instance deletes
  // with no prompt, then the provider instance's confirm is declined. The
  // earlier delete is permanent, so the cascade must report it rather than let
  // it vanish silently behind a cancelled persona delete.
  const originalWindow = globalThis.window;
  globalThis.window = { confirm: () => false };
  try {
    const deleted = [];
    const result = await deleteManagedAgentsForPersonaWithRules({
      persona: { id: "persona-1" },
      managedAgents: [
        personaAgent("a"),
        personaAgent("b", {
          backend: { type: "provider" },
          backendAgentId: "remote-1",
        }),
        personaAgent("c"),
      ],
      channels: [],
      relayAgents: [],
      deleteManagedAgent: async ({ pubkey }) => {
        deleted.push(pubkey);
      },
    });

    assert.equal(result.cancelled, true);
    assert.deepEqual(
      deleted,
      ["a".repeat(64)],
      "the local instance ahead of the declined one is already gone",
    );
    assert.equal(
      result.deletedCount,
      1,
      "deletedCount must surface the partial teardown so the caller can report it",
    );
  } finally {
    globalThis.window = originalWindow;
  }
});

test("persona cascade stops at the first failed instance delete", async () => {
  const deleted = [];
  await assert.rejects(
    deleteManagedAgentsForPersonaWithRules({
      persona: { id: "persona-1" },
      managedAgents: [personaAgent("a"), personaAgent("b")],
      channels: [],
      relayAgents: [],
      deleteManagedAgent: async ({ pubkey }) => {
        if (pubkey === "a".repeat(64)) throw new Error("backend refused");
        deleted.push(pubkey);
      },
    }),
    /backend refused/,
  );

  assert.deepEqual(
    deleted,
    [],
    "instances after the failure must be left untouched",
  );
});
