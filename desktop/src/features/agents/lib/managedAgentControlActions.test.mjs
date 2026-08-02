import assert from "node:assert/strict";
import test from "node:test";

import {
  deleteManagedAgentWithRules,
  managedAgentPresenceStatus,
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

// --- managedAgentPresenceStatus: control-plane status is not liveness --------

const REMOTE_BACKEND = { type: "provider", id: "ssh", config: {} };

test("a local agent's own process table beats a silent relay", () => {
  // Relays need not retain ephemeral kind:20001 presence, and this desktop is
  // supervising the process — a blip must not grey out a running local agent.
  assert.equal(managedAgentPresenceStatus(agent(), undefined), "online");
  assert.equal(managedAgentPresenceStatus(agent(), {}), "online");
});

test("a deployed remote agent with no relay presence is not claimed online", () => {
  // `backend_agent_id` is written once at deploy and never cleared (there is
  // no undeploy), so "deployed" alone would light the dot green forever.
  const remote = agent({
    status: "deployed",
    backend: REMOTE_BACKEND,
    backendAgentId: "remote-1",
  });
  assert.equal(managedAgentPresenceStatus(remote, {}), "offline");
  assert.equal(managedAgentPresenceStatus(remote, null), "offline");
});

test("a deployed remote agent reports whatever the relay says, verbatim", () => {
  const remote = agent({
    pubkey: "AB".repeat(32),
    status: "deployed",
    backend: REMOTE_BACKEND,
    backendAgentId: "remote-1",
  });
  // Lookup keys are normalized pubkeys; a mixed-case record must still hit.
  const lookup = { ["ab".repeat(32)]: "away" };
  assert.equal(managedAgentPresenceStatus(remote, lookup), "away");
  assert.equal(
    managedAgentPresenceStatus(remote, { ["ab".repeat(32)]: "online" }),
    "online",
  );
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

// ── Provider delete: orphan disclosure ────────────────────────────────────
//
// There is no undeploy in the provider protocol, so deleting a provider-backed
// record removes what this app knows about the deployment and nothing else.
// Every path that deletes one must therefore say so — either through this
// function's own confirm, or through a caller that already did.
//
// These cases cover the branches that reach no channel (offline, and not in a
// channel at all), which are exactly the ones a `skipRemoteDeleteConfirm: true`
// caller used to slip through with neither a warning nor a `!shutdown`.

function deployedProviderAgent(overrides = {}) {
  return agent({
    name: "Remote Scout",
    backend: { type: "provider", id: "blox", config: null },
    backendAgentId: "buzz-agent-scout.service",
    status: "deployed",
    ...overrides,
  });
}

function withConfirm(answer, body) {
  const previous = globalThis.window;
  const prompts = [];
  globalThis.window = {
    confirm: (message) => {
      prompts.push(message);
      return answer;
    },
  };
  try {
    return body(prompts);
  } finally {
    globalThis.window = previous;
  }
}

test("offline provider delete warns and names the unit that keeps running", async () => {
  let deleted = null;
  const prompts = await withConfirm(true, async (captured) => {
    await deleteManagedAgentWithRules({
      agent: deployedProviderAgent(),
      channels: [],
      // Offline: presence lookup has no entry, so no !shutdown is deliverable.
      presenceLookup: {},
      relayAgents: [],
      preferredChannelId: "channel-1",
      deleteManagedAgent: async (input) => {
        deleted = input;
      },
    });
    return captured;
  });

  assert.equal(prompts.length, 1, "offline provider delete must warn");
  assert.match(
    prompts[0],
    /does not stop the remote deployment/,
    "warning must not imply a teardown that cannot happen",
  );
  assert.match(
    prompts[0],
    /buzz-agent-scout\.service/,
    "warning must name the unit so the owner can stop it by hand",
  );
  assert.deepEqual(deleted, {
    pubkey: "deadbeef".repeat(8),
    forceRemoteDelete: true,
  });
});

test("provider delete with no channel warns and names the unit", async () => {
  let deleted = null;
  const prompts = await withConfirm(true, async (captured) => {
    await deleteManagedAgentWithRules({
      agent: deployedProviderAgent(),
      channels: [],
      presenceLookup: { ["deadbeef".repeat(8)]: "online" },
      relayAgents: [],
      // No preferredChannelId and no relay agent match — unaddressable.
      deleteManagedAgent: async (input) => {
        deleted = input;
      },
    });
    return captured;
  });

  assert.equal(prompts.length, 1, "unreachable provider delete must warn");
  assert.match(prompts[0], /not in any channel/);
  assert.match(prompts[0], /buzz-agent-scout\.service/);
  assert.ok(deleted, "confirmed delete proceeds");
});

test("declining the warning cancels the delete", async () => {
  let deleted = null;
  await withConfirm(false, async () => {
    const result = await deleteManagedAgentWithRules({
      agent: deployedProviderAgent(),
      channels: [],
      presenceLookup: {},
      relayAgents: [],
      deleteManagedAgent: async (input) => {
        deleted = input;
      },
    });
    assert.deepEqual(result, { cancelled: true });
  });

  assert.equal(deleted, null, "a declined warning must delete nothing");
});

test("a caller that already disclosed the orphan is not prompted twice", async () => {
  let deleted = null;
  const prompts = await withConfirm(true, async (captured) => {
    await deleteManagedAgentWithRules({
      agent: deployedProviderAgent(),
      channels: [],
      presenceLookup: {},
      relayAgents: [],
      remoteOrphanDisclosedByCaller: true,
      deleteManagedAgent: async (input) => {
        deleted = input;
      },
    });
    return captured;
  });

  assert.deepEqual(prompts, [], "caller's own dialog is the disclosure");
  assert.ok(deleted, "delete still proceeds");
});

test("provider record that never deployed needs no orphan warning", async () => {
  let deleted = null;
  const prompts = await withConfirm(true, async (captured) => {
    await deleteManagedAgentWithRules({
      // Provider backend, but no backend_agent_id: nothing was ever deployed,
      // so there is no remote unit to leave running.
      agent: deployedProviderAgent({ backendAgentId: null }),
      channels: [],
      presenceLookup: {},
      relayAgents: [],
      deleteManagedAgent: async (input) => {
        deleted = input;
      },
    });
    return captured;
  });

  assert.deepEqual(prompts, [], "nothing deployed, nothing to disclose");
  assert.deepEqual(deleted, {
    pubkey: "deadbeef".repeat(8),
    forceRemoteDelete: undefined,
  });
});

test("local agent delete is untouched by the provider disclosure", async () => {
  let deleted = null;
  const prompts = await withConfirm(true, async (captured) => {
    await deleteManagedAgentWithRules({
      agent: agent(),
      channels: [],
      presenceLookup: {},
      relayAgents: [],
      deleteManagedAgent: async (input) => {
        deleted = input;
      },
    });
    return captured;
  });

  assert.deepEqual(prompts, [], "local deletes never orphan anything");
  assert.deepEqual(deleted, {
    pubkey: "deadbeef".repeat(8),
    forceRemoteDelete: undefined,
  });
});
