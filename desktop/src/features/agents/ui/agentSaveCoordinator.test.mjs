import assert from "node:assert/strict";
import test from "node:test";

import { runAgentSaveCoordinator } from "./agentSaveCoordinator.ts";

// ── Shared fixtures ────────────────────────────────────────────────────────────

function makeDefinition(overrides = {}) {
  return {
    id: "def-1",
    displayName: "Alice",
    avatarUrl: "",
    systemPrompt: "Be helpful.",
    runtime: "goose",
    model: "gpt-4o",
    provider: null,
    isBuiltIn: false,
    isActive: true,
    namePool: [],
    envVars: {},
    respondTo: null,
    respondToAllowlist: [],
    parallelism: null,
    createdAt: "2025-01-01T00:00:00Z",
    updatedAt: "2025-01-01T00:00:00Z",
    ...overrides,
  };
}

function makeInstance(overrides = {}) {
  return {
    pubkey: "pk-abc",
    name: "Alice",
    avatarUrl: "",
    systemPrompt: null,
    model: null,
    provider: null,
    envVars: {},
    respondTo: null,
    respondToAllowlist: [],
    parallelism: null,
    autoRestartOnConfigChange: false,
    startOnAppLaunch: false,
    ...overrides,
  };
}

function makePersonaInput(overrides = {}) {
  return {
    id: "def-1",
    displayName: "Alice",
    systemPrompt: "Be helpful.",
    avatarUrl: "",
    runtime: "goose",
    model: "gpt-4o",
    provider: undefined,
    namePool: [],
    envVars: {},
    ...overrides,
  };
}

function makeAgentInput(overrides = {}) {
  return {
    pubkey: "pk-abc",
    ...overrides,
  };
}

/** Build minimal coordinator options. All mutations succeed by default. */
function makeOpts(overrides = {}) {
  const def = makeDefinition();
  const inst = makeInstance();

  const calls = {
    updatePersona: 0,
    updatePersonaAndPublish: 0,
    updateManagedAgent: 0,
    setAutoRestart: 0,
    setStartOnAppLaunch: 0,
    onDone: 0,
    onSavedWhileStopped: 0,
  };

  const opts = {
    ctx: { kind: "instance-with-definition", definition: def, instance: inst },
    personaInput: null,
    agentInput: null,
    policySets: [],
    publishCatalogUpdates: false,
    runtimes: undefined,
    updatePersona: async () => {
      calls.updatePersona++;
    },
    updatePersonaAndPublish: async () => {
      calls.updatePersonaAndPublish++;
      return { publicationStatus: "published" };
    },
    updateManagedAgent: async () => {
      calls.updateManagedAgent++;
      return { agent: inst, profileSyncError: null };
    },
    setAutoRestart: async () => {
      calls.setAutoRestart++;
    },
    setStartOnAppLaunch: async () => {
      calls.setStartOnAppLaunch++;
    },
    refetchStores: async () => ({ persona: def, agent: inst }),
    onDone: () => {
      calls.onDone++;
    },
    onSavedWhileStopped: () => {
      calls.onSavedWhileStopped++;
    },
    _calls: calls,
    ...overrides,
  };

  return opts;
}

// ── Test family 1: write ordering ─────────────────────────────────────────────
//
// Step 1 (definition write) must run before step 2 (instance write), and a
// step-1 error must prevent step 2 from being attempted.

test("test_write_ordering_definition_write_failure_skips_instance_write", async () => {
  const calls = { updatePersona: 0, updateManagedAgent: 0 };

  const opts = makeOpts({
    personaInput: makePersonaInput(),
    agentInput: makeAgentInput({ name: "Alice-renamed" }),
    updatePersona: async () => {
      calls.updatePersona++;
      throw new Error("Relay offline");
    },
    updateManagedAgent: async () => {
      calls.updateManagedAgent++;
      return { agent: makeInstance(), profileSyncError: null };
    },
    refetchStores: async () => ({ persona: null, agent: null }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(
    result,
    false,
    "should return false on definition write failure",
  );
  assert.equal(calls.updatePersona, 1, "definition write should be attempted");
  assert.equal(
    calls.updateManagedAgent,
    0,
    "instance write must NOT be attempted when definition write fails",
  );
});

test("test_write_ordering_instance_write_runs_after_definition_write_succeeds", async () => {
  const calls = { updatePersona: 0, updateManagedAgent: 0 };

  const opts = makeOpts({
    personaInput: makePersonaInput(),
    agentInput: makeAgentInput({ name: "Alice-renamed" }),
    updatePersona: async () => {
      calls.updatePersona++;
    },
    updateManagedAgent: async () => {
      // Must only be called after updatePersona
      assert.equal(
        calls.updatePersona,
        1,
        "definition write must precede instance write",
      );
      calls.updateManagedAgent++;
      return {
        agent: makeInstance({ name: "Alice-renamed" }),
        profileSyncError: null,
      };
    },
    refetchStores: async () => ({
      persona: makeDefinition(),
      agent: makeInstance({ name: "Alice-renamed" }),
    }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(result, true, "should return true on full success");
  assert.equal(calls.updatePersona, 1, "definition write should be called");
  assert.equal(calls.updateManagedAgent, 1, "instance write should be called");
});

test("test_write_ordering_policy_setters_run_only_after_both_data_writes_succeed", async () => {
  const calls = { updatePersona: 0, updateManagedAgent: 0, setAutoRestart: 0 };

  const opts = makeOpts({
    personaInput: makePersonaInput(),
    agentInput: makeAgentInput({ name: "Alice-renamed" }),
    policySets: [{ type: "autoRestart", pubkey: "pk-abc", value: true }],
    updatePersona: async () => {
      calls.updatePersona++;
    },
    updateManagedAgent: async () => {
      calls.updateManagedAgent++;
      return {
        agent: makeInstance({ name: "Alice-renamed" }),
        profileSyncError: null,
      };
    },
    setAutoRestart: async () => {
      // Must only be called after both data writes
      assert.equal(
        calls.updatePersona,
        1,
        "definition write must precede policy setter",
      );
      assert.equal(
        calls.updateManagedAgent,
        1,
        "instance write must precede policy setter",
      );
      calls.setAutoRestart++;
    },
    refetchStores: async () => ({
      persona: makeDefinition(),
      agent: makeInstance({ name: "Alice-renamed" }),
    }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(result, true);
  assert.equal(calls.setAutoRestart, 1, "policy setter should be called");
});

// ── Test family 2: local-save / publish failure ───────────────────────────────
//
// A definition write failure should surface as partial failure, reporting what
// did NOT persist. A publish failure (updatePersonaAndPublish throws) should
// also stop the sequence.

test("test_local_save_failure_returns_false_and_calls_settlement", async () => {
  let settlementCalled = false;

  const opts = makeOpts({
    personaInput: makePersonaInput(),
    updatePersona: async () => {
      throw new Error("Disk full");
    },
    refetchStores: async () => {
      settlementCalled = true;
      return { persona: null, agent: null };
    },
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(result, false, "should return false on local save failure");
  assert.equal(
    settlementCalled,
    true,
    "settlement (refetchStores) must be called even on failure",
  );
});

test("test_publish_failure_returns_false_stops_sequence", async () => {
  const calls = { updateManagedAgent: 0 };

  const opts = makeOpts({
    personaInput: makePersonaInput(),
    agentInput: makeAgentInput({ name: "Alice-renamed" }),
    publishCatalogUpdates: true,
    updatePersonaAndPublish: async () => {
      throw new Error("Relay rejected");
    },
    updateManagedAgent: async () => {
      calls.updateManagedAgent++;
      return { agent: makeInstance(), profileSyncError: null };
    },
    refetchStores: async () => ({ persona: null, agent: null }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(result, false, "should return false on publish failure");
  assert.equal(
    calls.updateManagedAgent,
    0,
    "instance write must not run if publish step failed",
  );
});

// ── Test family 3: observed mismatch ─────────────────────────────────────────
//
// Command success alone does not mean persistence. If the re-fetched observed
// state does not match what was submitted, the coordinator must return false
// and report the mismatch.

test("test_observed_mismatch_returns_false_when_persona_not_in_store_after_write", async () => {
  // updatePersona succeeds but refetchStores returns persona: null
  // (the write never actually persisted — e.g. a race with another write).
  const opts = makeOpts({
    personaInput: makePersonaInput({ displayName: "Alice-renamed" }),
    updatePersona: async () => {},
    // Observed store shows the original name (write lost)
    refetchStores: async () => ({
      persona: makeDefinition({ displayName: "Alice" }),
      agent: null,
    }),
  });

  const result = await runAgentSaveCoordinator(opts);

  // The submitted displayName "Alice-renamed" doesn't match observed "Alice"
  assert.equal(
    result,
    false,
    "should return false when observed state doesn't match submission",
  );
  assert.equal(
    opts._calls.onDone,
    0,
    "onDone must NOT be called when observed state doesn't match",
  );
});

test("test_observed_match_calls_onDone_and_returns_true", async () => {
  // Both the write succeeds and the observed state matches.
  const updatedPersona = makeDefinition({ displayName: "Alice-renamed" });

  const opts = makeOpts({
    personaInput: makePersonaInput({ displayName: "Alice-renamed" }),
    updatePersona: async () => {},
    refetchStores: async () => ({ persona: updatedPersona, agent: null }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(
    result,
    true,
    "should return true when observed state matches submission",
  );
  assert.equal(opts._calls.onDone, 1, "onDone must be called on full success");
});

test("test_absent_entity_after_refetch_is_not_persisted", async () => {
  // persona: null after refetch means the entity was not found → not persisted.
  const opts = makeOpts({
    personaInput: makePersonaInput(),
    updatePersona: async () => {},
    // Simulate write succeeding at command level but entity not appearing in store
    refetchStores: async () => ({ persona: null, agent: null }),
  });

  const result = await runAgentSaveCoordinator(opts);

  // Even though updatePersona didn't throw, the absent observed state means failure.
  assert.equal(
    result,
    false,
    "absent entity after refetch must be treated as not persisted",
  );
});

// ── Test family 4: partial policy failure ─────────────────────────────────────
//
// Multiple policy setters: if the first succeeds and the second fails, the
// coordinator must report the second as failed and return false. Unattempted
// policies (beyond the failing one) must also be reported as failed.

test("test_partial_policy_failure_first_succeeds_second_fails_returns_false", async () => {
  const calls = { setAutoRestart: 0, setStartOnAppLaunch: 0 };

  const inst = makeInstance({
    autoRestartOnConfigChange: false,
    startOnAppLaunch: false,
  });

  const opts = makeOpts({
    ctx: {
      kind: "instance-with-definition",
      definition: makeDefinition(),
      instance: inst,
    },
    policySets: [
      { type: "autoRestart", pubkey: "pk-abc", value: true },
      { type: "startOnAppLaunch", pubkey: "pk-abc", value: true },
    ],
    setAutoRestart: async () => {
      calls.setAutoRestart++;
    },
    setStartOnAppLaunch: async () => {
      calls.setStartOnAppLaunch++;
      throw new Error("Permission denied");
    },
    refetchStores: async () => ({ persona: makeDefinition(), agent: inst }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(
    result,
    false,
    "should return false when any policy setter fails",
  );
  assert.equal(calls.setAutoRestart, 1, "first policy should be attempted");
  assert.equal(
    calls.setStartOnAppLaunch,
    1,
    "second policy should be attempted",
  );
  assert.equal(
    opts._calls.onDone,
    0,
    "onDone must not be called on partial policy failure",
  );
});

test("test_early_policy_failure_skips_subsequent_policies", async () => {
  const calls = { setAutoRestart: 0, setStartOnAppLaunch: 0 };

  const inst = makeInstance();

  const opts = makeOpts({
    ctx: {
      kind: "instance-with-definition",
      definition: makeDefinition(),
      instance: inst,
    },
    policySets: [
      { type: "autoRestart", pubkey: "pk-abc", value: true },
      { type: "startOnAppLaunch", pubkey: "pk-abc", value: true },
    ],
    setAutoRestart: async () => {
      calls.setAutoRestart++;
      throw new Error("Store locked");
    },
    setStartOnAppLaunch: async () => {
      calls.setStartOnAppLaunch++;
    },
    refetchStores: async () => ({ persona: makeDefinition(), agent: inst }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(
    result,
    false,
    "should return false when first policy setter fails",
  );
  assert.equal(calls.setAutoRestart, 1, "first policy should be attempted");
  assert.equal(
    calls.setStartOnAppLaunch,
    0,
    "second policy must NOT be attempted after first failure (stop-at-first-failure per spec)",
  );
});

test("test_settlement_always_runs_even_when_no_writes_attempted", async () => {
  // No personaInput, no agentInput, no policySets: nothing to write.
  // Settlement (refetchStores) should still be called for the success path.
  let settlementCalled = false;

  const opts = makeOpts({
    refetchStores: async () => {
      settlementCalled = true;
      return { persona: null, agent: null };
    },
    onDone: () => {},
  });

  await runAgentSaveCoordinator(opts);

  assert.equal(
    settlementCalled,
    true,
    "settlement must always run regardless of writes",
  );
});
