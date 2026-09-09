import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { toast } from "sonner";
import {
  PERSONA_REVISION_CONFLICT,
  runAgentSaveCoordinator,
} from "./agentSaveCoordinator.ts";

// Capture toast calls by kind. The coordinator and this test import the same
// `toast` object from sonner, so overriding its methods here intercepts the
// calls made inside runAgentSaveCoordinator. Returns a restore fn.
function captureToasts() {
  const captured = [];
  const original = {
    success: toast.success,
    warning: toast.warning,
    error: toast.error,
  };
  for (const kind of ["success", "warning", "error"]) {
    toast[kind] = (message) => {
      captured.push({ kind, message });
    };
  }
  return {
    captured,
    restore() {
      Object.assign(toast, original);
    },
  };
}

// ── Shared fixtures ────────────────────────────────────────────────────────────

const partialPublishOutcomeContract = JSON.parse(
  readFileSync(
    new URL(
      "../../../../../test-fixtures/update-persona-publish-partial-outcome.json",
      import.meta.url,
    ),
    "utf8",
  ),
);

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
  // The refetchStores must reflect each write as it happens so per-boundary
  // settlement passes and the coordinator can advance through all steps.
  // After I-write the agent has name "Alice-renamed"; after autoRestart the
  // agent also has autoRestartOnConfigChange: true.
  let refetchCount = 0;

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
    refetchStores: async () => {
      refetchCount++;
      // After D-write (refetch 1): persona matches, agent has original name.
      // After I-write (refetch 2): agent now has renamed name.
      // After autoRestart setter (refetch 3 + final): agent also has autoRestart=true.
      const agentName = refetchCount >= 2 ? "Alice-renamed" : "Alice";
      const autoRestart = refetchCount >= 3;
      return {
        persona: makeDefinition(),
        agent: makeInstance({
          name: agentName,
          autoRestartOnConfigChange: autoRestart,
        }),
      };
    },
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

test("test_definition_write_throws_but_persisted_is_success", async () => {
  // Observed state is authoritative over the command result: a definition
  // write that threw but whose write landed on disk must NOT be reported as a
  // failed step. The instance write proceeds and onDone is called.
  const updatedPersona = makeDefinition({ displayName: "Alice-renamed" });

  const opts = makeOpts({
    personaInput: makePersonaInput({ displayName: "Alice-renamed" }),
    agentInput: makeAgentInput(),
    updatePersona: async () => {
      throw new Error("Relay timeout after commit");
    },
    refetchStores: async () => ({
      persona: updatedPersona,
      agent: makeInstance(),
    }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(
    result,
    true,
    "a thrown-but-persisted definition write must be treated as success",
  );
  assert.equal(
    opts._calls.updateManagedAgent,
    1,
    "instance write must proceed when the definition write persisted",
  );
  assert.equal(
    opts._calls.onDone,
    1,
    "onDone must be called on persisted write",
  );
});

test("test_instance_write_throws_but_persisted_is_success", async () => {
  // Same authority rule for the instance step: a throw whose write persisted
  // is success.
  const updatedInstance = makeInstance({ name: "Alice-renamed" });

  const opts = makeOpts({
    agentInput: makeAgentInput({ name: "Alice-renamed" }),
    updateManagedAgent: async () => {
      throw new Error("Relay timeout after commit");
    },
    refetchStores: async () => ({ persona: null, agent: updatedInstance }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(
    result,
    true,
    "a thrown-but-persisted instance write must be treated as success",
  );
  assert.equal(
    opts._calls.onDone,
    1,
    "onDone must be called on persisted write",
  );
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

  // Per-boundary settlement: after the first policy setter (autoRestart) succeeds,
  // refetchStores must return autoRestartOnConfigChange: true for the check to
  // pass and the coordinator to advance to the second setter. The second setter
  // throws, so the second policy is attempted but fails.
  let refetchCount = 0;

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
    refetchStores: async () => {
      refetchCount++;
      // After first policy setter (autoRestart=true) succeeds, reflect it.
      // startOnAppLaunch stays false throughout (second setter throws).
      const autoRestart = refetchCount >= 1 && calls.setAutoRestart > 0;
      return {
        persona: makeDefinition(),
        agent: makeInstance({
          autoRestartOnConfigChange: autoRestart,
          startOnAppLaunch: false,
        }),
      };
    },
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

// -- CRITICAL-3: per-boundary mismatch tests --
//
// These verify Thufir's two probes: successful harness command whose refetched
// agent retains old harness fields must NOT call onDone; successful auto-restart
// setter whose refetched agent remains false must NOT call onDone.

test("test_harness_command_success_but_observed_mismatch_returns_false", async () => {
  // Thufir probe 1: agentCommand submitted, command returns success, but the
  // refetched agent still has the old command. Must NOT call onDone.
  let doneCalled = false;

  const staleAgent = makeInstance({
    agentCommand: "/old/harness",
    agentCommandOverride: null,
    agentArgs: [],
    acpCommand: "",
  });
  const opts = makeOpts({
    agentInput: { pubkey: "pk-abc", agentCommand: "/new/harness" },
    updateManagedAgent: async () => ({
      agent: staleAgent,
      profileSyncError: null,
    }),
    refetchStores: async () => ({
      persona: null,
      agent: staleAgent, // old command -- mismatch with submitted
    }),
    onDone: () => {
      doneCalled = true;
    },
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(result, false, "mismatch must return false");
  assert.equal(
    doneCalled,
    false,
    "onDone must NOT be called on harness mismatch",
  );
});

test("test_auto_restart_success_but_observed_unchanged_returns_false", async () => {
  // Thufir probe 2: autoRestart setter returns success (no throw), but the
  // refetched agent still has the old value (false). Must NOT call onDone.
  let doneCalled = false;

  const unchangedAgent = makeInstance({ autoRestartOnConfigChange: false });
  const opts = makeOpts({
    policySets: [{ type: "autoRestart", pubkey: "pk-abc", value: true }],
    setAutoRestart: async () => {},
    refetchStores: async () => ({
      persona: null,
      agent: unchangedAgent, // still false -- mismatch with submitted true
    }),
    onDone: () => {
      doneCalled = true;
    },
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(result, false, "auto-restart mismatch must return false");
  assert.equal(
    doneCalled,
    false,
    "onDone must NOT be called when policy did not persist",
  );
});

test("test_start_on_app_launch_success_and_observed_match_calls_onDone", async () => {
  // Positive case: startOnAppLaunch setter succeeds AND observed state matches.
  let doneCalled = false;

  const updatedAgent = makeInstance({ startOnAppLaunch: true });
  const opts = makeOpts({
    policySets: [{ type: "startOnAppLaunch", pubkey: "pk-abc", value: true }],
    setStartOnAppLaunch: async () => {},
    refetchStores: async () => ({
      persona: null,
      agent: updatedAgent, // matches submitted value
    }),
    onDone: () => {
      doneCalled = true;
    },
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(result, true, "matching policy must return true");
  assert.equal(doneCalled, true, "onDone must be called on policy success");
});

test("test_definition_write_not_persisted_stops_instance_write", async () => {
  // Per-boundary: if D-write succeeds but observed persona does not match,
  // the coordinator must not advance to the I-write.
  let instanceWriteCalled = false;

  const stalePersona = makeDefinition({
    displayName: "Old Name",
    systemPrompt: "Be helpful.",
  });
  const opts = makeOpts({
    personaInput: {
      id: "def-1",
      displayName: "Updated Name",
      systemPrompt: "Updated prompt.",
      namePool: [],
      envVars: {},
    },
    agentInput: { pubkey: "pk-abc", name: "updated-name" },
    updatePersona: async () => {},
    updateManagedAgent: async () => {
      instanceWriteCalled = true;
      return { agent: makeInstance(), profileSyncError: null };
    },
    refetchStores: async () => ({
      // Persona with OLD displayName = mismatch after D-write.
      persona: stalePersona,
      agent: makeInstance(),
    }),
    onDone: () => {},
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(result, false, "D-write mismatch must return false");
  assert.equal(
    instanceWriteCalled,
    false,
    "instance write must NOT be attempted when D-write did not persist",
  );
});

// ── Test family 5: thrown-but-persisted policy settlement (Thufir pass-1 CRITICAL) ──
//
// Both Tauri policy setters save the record BEFORE building their returned
// summary, so a post-save summary error yields a thrown-but-persisted write.
// Settlement must observe the store — not the command result — exactly as the
// D/I steps do: a throw whose write landed is success, the sequence continues,
// and onDone fires.

test("test_auto_restart_throws_but_persisted_advances_and_calls_onDone", async () => {
  // autoRestart setter throws, but the refetched agent shows the new value.
  const opts = makeOpts({
    policySets: [{ type: "autoRestart", pubkey: "pk-abc", value: true }],
    setAutoRestart: async () => {
      throw new Error("summary build failed after save");
    },
    refetchStores: async () => ({
      persona: null,
      agent: makeInstance({ autoRestartOnConfigChange: true }),
    }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(
    result,
    true,
    "a thrown-but-persisted autoRestart write must be treated as success",
  );
  assert.equal(
    opts._calls.onDone,
    1,
    "onDone must be called when the policy persisted despite the throw",
  );
});

test("test_start_on_app_launch_throws_but_persisted_advances_and_calls_onDone", async () => {
  // startOnAppLaunch setter throws, but the refetched agent shows the new value.
  const opts = makeOpts({
    policySets: [{ type: "startOnAppLaunch", pubkey: "pk-abc", value: true }],
    setStartOnAppLaunch: async () => {
      throw new Error("summary build failed after save");
    },
    refetchStores: async () => ({
      persona: null,
      agent: makeInstance({ startOnAppLaunch: true }),
    }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(
    result,
    true,
    "a thrown-but-persisted startOnAppLaunch write must be treated as success",
  );
  assert.equal(
    opts._calls.onDone,
    1,
    "onDone must be called when the policy persisted despite the throw",
  );
});

test("test_thrown_but_persisted_policy_continues_to_later_policy", async () => {
  const calls = { setAutoRestart: 0, setStartOnAppLaunch: 0 };
  // First policy (autoRestart) throws but persists; the coordinator must
  // observe persistence, advance to the second policy, and (with the second
  // also persisting) call onDone. The buggy behavior skipped the second policy.
  const opts = makeOpts({
    policySets: [
      { type: "autoRestart", pubkey: "pk-abc", value: true },
      { type: "startOnAppLaunch", pubkey: "pk-abc", value: true },
    ],
    setAutoRestart: async () => {
      calls.setAutoRestart++;
      throw new Error("summary build failed after save");
    },
    setStartOnAppLaunch: async () => {
      calls.setStartOnAppLaunch++;
    },
    // Both values are observed as persisted throughout — the first setter's
    // write landed before it threw, the second write is clean.
    refetchStores: async () => ({
      persona: null,
      agent: makeInstance({
        autoRestartOnConfigChange: true,
        startOnAppLaunch: true,
      }),
    }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(
    result,
    true,
    "a thrown-but-persisted first policy must not block a persisted second policy",
  );
  assert.equal(calls.setAutoRestart, 1, "first policy attempted");
  assert.equal(
    calls.setStartOnAppLaunch,
    1,
    "second policy must be attempted after the first policy persisted despite throwing",
  );
  assert.equal(opts._calls.onDone, 1, "onDone must fire on full persistence");
});

// ── Test family 6: full-replacement behavior-group settlement (Thufir pass-1 IMPORTANT) ──
//
// A submitted behavior group is replace-as-a-unit: the backend clears any
// OMITTED member to null/empty. Settlement must compare every member —
// including omitted ones — against the observed cleared value, so a clear the
// backend failed to apply cannot false-succeed.

test("test_parallelism_clear_not_applied_is_flagged_as_not_persisted", async () => {
  // The user cleared parallelism: the submitted behavior group omits it (the
  // clear signal). The store still shows the OLD value (4) — the clear did not
  // apply. Settlement must treat this as not persisted and return false.
  const opts = makeOpts({
    ctx: {
      kind: "definition-only",
      definition: makeDefinition({ respondTo: "anyone", parallelism: 4 }),
    },
    personaInput: makePersonaInput({
      // Behavior group carries respondTo but omits parallelism → clear it.
      behavior: { respondTo: "anyone" },
    }),
    updatePersona: async () => {},
    refetchStores: async () => ({
      // Clear failed: parallelism is still 4 in the observed store.
      persona: makeDefinition({ respondTo: "anyone", parallelism: 4 }),
      agent: null,
    }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(
    result,
    false,
    "an unapplied parallelism clear must be flagged as not persisted",
  );
  assert.equal(
    opts._calls.onDone,
    0,
    "onDone must NOT be called when the clear did not apply",
  );
});

test("test_parallelism_clear_applied_settles_as_persisted", async () => {
  // Same clear, but the store now shows parallelism cleared (null). Settlement
  // must treat the omitted member as matching the observed null and succeed.
  const opts = makeOpts({
    ctx: {
      kind: "definition-only",
      definition: makeDefinition({ respondTo: "anyone", parallelism: 4 }),
    },
    personaInput: makePersonaInput({ behavior: { respondTo: "anyone" } }),
    updatePersona: async () => {},
    refetchStores: async () => ({
      persona: makeDefinition({ respondTo: "anyone", parallelism: null }),
      agent: null,
    }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(
    result,
    true,
    "an applied parallelism clear (observed null) must settle as persisted",
  );
  assert.equal(
    opts._calls.onDone,
    1,
    "onDone must fire when the clear applied",
  );
});

// ── Test family 7: refetch-rejection verification-unknown (Carl review) ──────
//
// refetchStores() is awaited bare at every settlement boundary. If a
// verification refetch REJECTS after a write may have committed, the rejection
// must never escape: the coordinator returns false (dialog stays open), fires a
// "could not verify" warning — NOT a "write failed" error — and stops
// advancing. Each boundary (definition, instance, policy, final) is covered.

/** A refetchStores whose Nth call (1-based) rejects; earlier/later calls use `ok`. */
function refetchRejectingOnCall(rejectOn, ok) {
  let n = 0;
  return async () => {
    n += 1;
    if (n === rejectOn) throw new Error("Store refetch failed");
    return ok();
  };
}

test("test_refetch_rejection_after_definition_write_reports_unknown_not_failed", async () => {
  const cap = captureToasts();
  try {
    const opts = makeOpts({
      personaInput: makePersonaInput({ displayName: "Alice-renamed" }),
      agentInput: makeAgentInput({ name: "Alice-renamed" }),
      updatePersona: async () => {},
      // First refetch (after the definition write) rejects.
      refetchStores: refetchRejectingOnCall(1, () => ({
        persona: makeDefinition({ displayName: "Alice-renamed" }),
        agent: makeInstance({ name: "Alice-renamed" }),
      })),
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(result, false, "refetch rejection must return false");
    assert.equal(
      opts._calls.onDone,
      0,
      "onDone must NOT fire when persistence could not be verified",
    );
    assert.equal(
      opts._calls.updateManagedAgent,
      0,
      "must stop advancing to the instance write after a refetch rejection",
    );
    const warnings = cap.captured.filter((c) => c.kind === "warning");
    assert.equal(warnings.length, 1, "exactly one warning toast");
    assert.match(
      warnings[0].message,
      /could not verify/i,
      "toast must say persistence could not be verified",
    );
    assert.equal(
      cap.captured.some((c) => c.kind === "error"),
      false,
      "must NOT claim the write failed",
    );
  } finally {
    cap.restore();
  }
});

test("test_refetch_rejection_after_instance_write_reports_unknown", async () => {
  const cap = captureToasts();
  try {
    const opts = makeOpts({
      agentInput: makeAgentInput({ name: "Alice-renamed" }),
      updateManagedAgent: async () => ({
        agent: makeInstance({ name: "Alice-renamed" }),
        profileSyncError: null,
      }),
      // Only the instance write is present, so its settlement is the first
      // refetch call.
      refetchStores: refetchRejectingOnCall(1, () => ({
        persona: null,
        agent: makeInstance({ name: "Alice-renamed" }),
      })),
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(result, false, "refetch rejection must return false");
    assert.equal(opts._calls.onDone, 0, "onDone must NOT fire");
    const warnings = cap.captured.filter((c) => c.kind === "warning");
    assert.equal(warnings.length, 1);
    assert.match(warnings[0].message, /could not verify/i);
  } finally {
    cap.restore();
  }
});

test("test_refetch_rejection_after_policy_setter_reports_unknown", async () => {
  const cap = captureToasts();
  try {
    const opts = makeOpts({
      policySets: [{ type: "autoRestart", pubkey: "pk-abc", value: true }],
      setAutoRestart: async () => {},
      // The policy setter's settlement is the first refetch call.
      refetchStores: refetchRejectingOnCall(1, () => ({
        persona: null,
        agent: makeInstance({ autoRestartOnConfigChange: true }),
      })),
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(result, false, "refetch rejection must return false");
    assert.equal(opts._calls.onDone, 0, "onDone must NOT fire");
    const warnings = cap.captured.filter((c) => c.kind === "warning");
    assert.equal(warnings.length, 1);
    assert.match(warnings[0].message, /could not verify/i);
  } finally {
    cap.restore();
  }
});

test("test_refetch_rejection_at_final_settlement_reports_unknown", async () => {
  const cap = captureToasts();
  try {
    // No writes: the only refetch is the final settlement. Its rejection must
    // still be contained and reported as unverified.
    const opts = makeOpts({
      refetchStores: refetchRejectingOnCall(1, () => ({
        persona: null,
        agent: null,
      })),
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(result, false, "final-settlement rejection must return false");
    assert.equal(opts._calls.onDone, 0, "onDone must NOT fire");
    const warnings = cap.captured.filter((c) => c.kind === "warning");
    assert.equal(warnings.length, 1);
    assert.match(warnings[0].message, /could not verify/i);
  } finally {
    cap.restore();
  }
});

test("test_mutation_throws_then_refetch_rejects_reports_unknown_not_failed", async () => {
  // Carl's named case: a write throws AND the verification refetch then rejects.
  // The mutation may have committed on disk, so the coordinator must report
  // verification-unknown — never assert the write failed — and keep the dialog
  // open.
  const cap = captureToasts();
  try {
    const opts = makeOpts({
      personaInput: makePersonaInput({ displayName: "Alice-renamed" }),
      updatePersona: async () => {
        throw new Error("Relay timeout after commit");
      },
      refetchStores: refetchRejectingOnCall(1, () => ({
        persona: null,
        agent: null,
      })),
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(result, false, "must return false — dialog stays open");
    assert.equal(opts._calls.onDone, 0, "onDone must NOT fire");
    const warnings = cap.captured.filter((c) => c.kind === "warning");
    assert.equal(
      warnings.length,
      1,
      "exactly one verification-unknown warning",
    );
    assert.match(warnings[0].message, /could not verify/i);
    assert.equal(
      cap.captured.some(
        (c) => c.kind === "error" || /failed/i.test(String(c.message)),
      ),
      false,
      "must NOT claim the write failed when it may have committed",
    );
  } finally {
    cap.restore();
  }
});

// ── Test family 8: concurrent-edit drift guard (P1-2, Carl round-4) ──────────
//
// A definition write is built from the form baseline captured at seed time. The
// guard must read the AUTHORITATIVE persisted revision before the first write
// and compare it to the seed-time value — not the React-cached `ctx.updatedAt`,
// which can lag a concurrent writer's update. If the persisted revision has
// advanced, the stale full-replacement input would clobber the newer writer's
// values, so the coordinator aborts BEFORE any write — nothing persisted,
// dialog stays open (returns false), toast tells the user to reopen.

test("test_stale_cache_lets_two_writer_overwrite_through_cache_only_guard", async () => {
  // TOCTOU: writer B submits before its query cache received writer A's newer
  // write, so B's CACHED ctx revision still equals the seed (no cache drift),
  // but the persisted definition has advanced to R2. The authoritative
  // pre-write refetch must observe R2 and abort — a cache-only comparison
  // (ctx.updatedAt === seed) would pass and B would clobber A.
  const cap = captureToasts();
  try {
    const opts = makeOpts({
      ctx: {
        kind: "definition-only",
        // Cached revision is STILL the seed value — cache hasn't seen A's write.
        definition: makeDefinition({ updatedAt: "2025-01-01T00:00:00Z" }),
      },
      personaInput: makePersonaInput({ displayName: "Alice-renamed" }),
      expectedDefinitionUpdatedAt: "2025-01-01T00:00:00Z",
      // Authoritative persisted state: writer A advanced it to R2.
      refetchStores: async () => ({
        persona: makeDefinition({ updatedAt: "2025-06-01T00:00:00Z" }),
        agent: null,
      }),
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(
      result,
      false,
      "persisted drift must abort the save (dialog stays open)",
    );
    assert.equal(
      opts._calls.updatePersona,
      0,
      "no definition write may be attempted when the persisted revision advanced",
    );
    assert.equal(opts._calls.onDone, 0, "onDone must NOT fire on drift abort");
    const errors = cap.captured.filter((c) => c.kind === "error");
    assert.equal(errors.length, 1, "exactly one error toast");
    assert.match(
      errors[0].message,
      /changed while you were editing/i,
      "toast must tell the user the template changed — reopen",
    );
  } finally {
    cap.restore();
  }
});

test("test_no_drift_when_persisted_revision_matches_proceeds_with_write", async () => {
  const updated = makeDefinition({
    displayName: "Alice-renamed",
    updatedAt: "2025-01-01T00:00:00Z",
  });
  const opts = makeOpts({
    ctx: {
      kind: "definition-only",
      definition: makeDefinition({ updatedAt: "2025-01-01T00:00:00Z" }),
    },
    personaInput: makePersonaInput({ displayName: "Alice-renamed" }),
    expectedDefinitionUpdatedAt: "2025-01-01T00:00:00Z",
    // Authoritative pre-write fetch AND settlement both read this revision.
    refetchStores: async () => ({ persona: updated, agent: null }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(
    result,
    true,
    "matching persisted revision must not block the write",
  );
  assert.equal(opts._calls.updatePersona, 1, "definition write proceeds");
  assert.equal(opts._calls.onDone, 1, "onDone fires on success");
});

test("test_pre_write_refetch_failure_aborts_without_writing", async () => {
  // The authoritative pre-write fetch itself rejects. Nothing was attempted, so
  // this is a pre-save verification failure — abort without writing, and the
  // toast must NOT claim the write may have persisted.
  const cap = captureToasts();
  try {
    const opts = makeOpts({
      ctx: {
        kind: "definition-only",
        definition: makeDefinition({ updatedAt: "2025-01-01T00:00:00Z" }),
      },
      personaInput: makePersonaInput({ displayName: "Alice-renamed" }),
      expectedDefinitionUpdatedAt: "2025-01-01T00:00:00Z",
      refetchStores: async () => {
        throw new Error("relay unreachable");
      },
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(result, false, "pre-write fetch failure aborts the save");
    assert.equal(
      opts._calls.updatePersona,
      0,
      "no write may be attempted when the pre-write fetch fails",
    );
    assert.equal(opts._calls.onDone, 0, "onDone must NOT fire");
    const errors = cap.captured.filter((c) => c.kind === "error");
    assert.equal(errors.length, 1, "exactly one error toast");
    assert.match(
      errors[0].message,
      /nothing was changed/i,
      "pre-write failure must state nothing was changed, not that persistence is unknown",
    );
    assert.doesNotMatch(
      errors[0].message,
      /may have been applied|whether .* saved/i,
      "must not use the post-write unverified-persistence wording",
    );
  } finally {
    cap.restore();
  }
});

test("test_instance_only_save_skips_drift_guard", async () => {
  // Instance-only saves emit no personaInput; the guard must never fire even
  // when no expectedDefinitionUpdatedAt is supplied.
  const updated = makeInstance({ name: "Alice-renamed" });
  const opts = makeOpts({
    ctx: { kind: "instance-only", instance: makeInstance() },
    agentInput: makeAgentInput({ name: "Alice-renamed" }),
    updateManagedAgent: async () => ({
      agent: updated,
      profileSyncError: null,
    }),
    refetchStores: async () => ({ persona: null, agent: updated }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(result, true, "instance-only save is unaffected by the guard");
  assert.equal(opts._calls.onDone, 1);
});

test("test_drift_guard_inert_when_no_expected_updatedAt_supplied", async () => {
  // A personaInput with no expectedDefinitionUpdatedAt (null) must not abort —
  // the guard is opt-in and skips the pre-write fetch when the seed-time value
  // is absent.
  const updated = makeDefinition({ displayName: "Alice-renamed" });
  const opts = makeOpts({
    ctx: { kind: "definition-only", definition: makeDefinition() },
    personaInput: makePersonaInput({ displayName: "Alice-renamed" }),
    expectedDefinitionUpdatedAt: null,
    refetchStores: async () => ({ persona: updated, agent: null }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(result, true, "null expected updatedAt skips the guard");
  assert.equal(opts._calls.updatePersona, 1);
});

// ── Test family 8b: backend compare-and-swap threading + rejection (P1, Carl round-4) ──
//
// The pre-write refetch narrows but cannot close the check-to-write window: the
// authoritative read releases the store lock before the write reacquires it. So
// the coordinator threads the seed-time revision (`expectedUpdatedAt`) into the
// write, and the backend compares it under the SAME lock that guards the write.
// A rejection carries PERSONA_REVISION_CONFLICT and must surface as the drift
// toast — the same user situation as the Step-0 abort.

test("test_definition_write_carries_expected_updated_at_for_backend_cas", async () => {
  // The seed-time revision must reach updatePersona so the backend can enforce
  // its lock-held compare-and-swap. Capture the input the write received.
  let received = null;
  const updated = makeDefinition({
    displayName: "Alice-renamed",
    updatedAt: "2025-01-01T00:00:00Z",
  });
  const opts = makeOpts({
    ctx: {
      kind: "definition-only",
      definition: makeDefinition({ updatedAt: "2025-01-01T00:00:00Z" }),
    },
    personaInput: makePersonaInput({ displayName: "Alice-renamed" }),
    expectedDefinitionUpdatedAt: "2025-01-01T00:00:00Z",
    updatePersona: async (input) => {
      received = input;
    },
    refetchStores: async () => ({ persona: updated, agent: null }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(result, true, "matching revision proceeds");
  assert.equal(
    received?.expectedUpdatedAt,
    "2025-01-01T00:00:00Z",
    "the write must carry the seed-time revision for the backend CAS",
  );
});

test("test_publish_path_carries_expected_updated_at_for_backend_cas", async () => {
  // The publish path (updatePersonaAndPublish) must thread the revision too, so
  // "Save and publish" gets identical concurrency protection.
  let received = null;
  const updated = makeDefinition({
    displayName: "Alice-renamed",
    shared: true,
    updatedAt: "2025-01-01T00:00:00Z",
  });
  const opts = makeOpts({
    ctx: {
      kind: "definition-only",
      definition: makeDefinition({
        shared: true,
        updatedAt: "2025-01-01T00:00:00Z",
      }),
    },
    personaInput: makePersonaInput({ displayName: "Alice-renamed" }),
    publishCatalogUpdates: true,
    expectedDefinitionUpdatedAt: "2025-01-01T00:00:00Z",
    updatePersonaAndPublish: async (input) => {
      received = input;
      return { publicationStatus: "published" };
    },
    refetchStores: async () => ({ persona: updated, agent: null }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(result, true, "matching revision proceeds on the publish path");
  assert.equal(
    received?.expectedUpdatedAt,
    "2025-01-01T00:00:00Z",
    "the publish write must carry the seed-time revision for the backend CAS",
  );
});

test("test_backend_cas_rejection_surfaces_drift_toast_and_aborts", async () => {
  // The window a concurrent writer slips through: the pre-write refetch reads
  // the seed revision (passes), then a writer commits, then the backend write
  // rejects with the conflict marker. The coordinator must map it to the drift
  // toast and return false WITHOUT attempting settlement.
  const cap = captureToasts();
  try {
    const opts = makeOpts({
      ctx: {
        kind: "definition-only",
        definition: makeDefinition({ updatedAt: "2025-01-01T00:00:00Z" }),
      },
      personaInput: makePersonaInput({ displayName: "Alice-renamed" }),
      expectedDefinitionUpdatedAt: "2025-01-01T00:00:00Z",
      updatePersona: async () => {
        throw new Error(
          `${PERSONA_REVISION_CONFLICT}Alice changed while you were editing`,
        );
      },
      // Pre-write refetch sees the seed revision (passes); the write then loses
      // the race under the backend lock.
      refetchStores: async () => ({
        persona: makeDefinition({ updatedAt: "2025-01-01T00:00:00Z" }),
        agent: null,
      }),
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(result, false, "a CAS rejection aborts the save");
    assert.equal(
      opts._calls.onDone,
      0,
      "onDone must NOT fire on a CAS rejection",
    );
    const errors = cap.captured.filter((c) => c.kind === "error");
    assert.equal(errors.length, 1, "exactly one error toast");
    assert.match(
      errors[0].message,
      /changed while you were editing/i,
      "a backend CAS rejection must surface the drift toast",
    );
    assert.doesNotMatch(
      errors[0].message,
      new RegExp(PERSONA_REVISION_CONFLICT),
      "the raw conflict marker must not leak into the user-facing toast",
    );
  } finally {
    cap.restore();
  }
});

test("test_generic_write_failure_is_not_treated_as_a_drift_conflict", async () => {
  // A non-conflict throw must follow the ordinary settlement path (observed
  // non-persist), NOT the drift toast — the marker guard must be specific.
  const cap = captureToasts();
  try {
    const opts = makeOpts({
      ctx: {
        kind: "definition-only",
        definition: makeDefinition({ updatedAt: "2025-01-01T00:00:00Z" }),
      },
      personaInput: makePersonaInput({ displayName: "Alice-renamed" }),
      expectedDefinitionUpdatedAt: "2025-01-01T00:00:00Z",
      updatePersona: async () => {
        throw new Error("disk full");
      },
      // Settlement observes the write did NOT persist (still the old name).
      refetchStores: async () => ({
        persona: makeDefinition({ updatedAt: "2025-01-01T00:00:00Z" }),
        agent: null,
      }),
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(result, false, "a generic write failure returns false");
    const errors = cap.captured.filter((c) => c.kind === "error");
    assert.equal(errors.length, 1, "exactly one error toast");
    assert.match(
      errors[0].message,
      /disk full/i,
      "generic error surfaces its own message",
    );
    assert.doesNotMatch(
      errors[0].message,
      /changed while you were editing/i,
      "a non-conflict failure must not claim a concurrent edit",
    );
  } finally {
    cap.restore();
  }
});
test("test_error_containing_conflict_token_midstring_is_not_treated_as_drift", async () => {
  // The marker check uses startsWith, not includes. An unrelated error message
  // that happens to contain the conflict token somewhere inside it must not
  // be misidentified as a concurrent-edit conflict and trigger the drift toast.
  const cap = captureToasts();
  try {
    const opts = makeOpts({
      ctx: {
        kind: "definition-only",
        definition: makeDefinition({ updatedAt: "2025-01-01T00:00:00Z" }),
      },
      personaInput: makePersonaInput({ displayName: "Alice-renamed" }),
      expectedDefinitionUpdatedAt: "2025-01-01T00:00:00Z",
      updatePersona: async () => {
        // Message embeds the token mid-string — must NOT trigger the drift path.
        throw new Error(
          `unrelated failure containing ${PERSONA_REVISION_CONFLICT} inside`,
        );
      },
      refetchStores: async () => ({
        persona: makeDefinition({ updatedAt: "2025-01-01T00:00:00Z" }),
        agent: null,
      }),
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(result, false, "any write failure returns false");
    const errors = cap.captured.filter((c) => c.kind === "error");
    assert.equal(errors.length, 1, "exactly one error toast");
    assert.doesNotMatch(
      errors[0].message,
      /changed while you were editing/i,
      "mid-string token must not trigger the drift toast",
    );
  } finally {
    cap.restore();
  }
});

// ── Test family 9: success toast names the observed (persisted) agent (P2) ───
//
// `latestAgent` only advances on a NON-throwing updateManagedAgent. A rename
// that commits to disk but whose command throws afterward leaves `latestAgent`
// at the pre-save instance, so the success toast must read the name from the
// observed refetch — not from `latestAgent` — or a committed Alice→Bob rename
// falsely reports "Alice saved."

test("test_success_toast_uses_observed_name_on_thrown_but_persisted_rename", async () => {
  const cap = captureToasts();
  try {
    const opts = makeOpts({
      ctx: {
        kind: "instance-only",
        instance: makeInstance({ name: "Alice" }),
      },
      agentInput: makeAgentInput({ name: "Bob" }),
      // The rename commits to disk, but the command throws after commit, so
      // `latestAgent` is never reassigned and stays at the pre-save "Alice".
      updateManagedAgent: async () => {
        throw new Error("summary build failed after commit");
      },
      // The final refetch observes the persisted rename.
      refetchStores: async () => ({
        persona: null,
        agent: makeInstance({ name: "Bob" }),
      }),
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(
      result,
      true,
      "a thrown-but-persisted write is a full success (observed state matches)",
    );
    const successes = cap.captured.filter((c) => c.kind === "success");
    assert.equal(successes.length, 1, "exactly one success toast");
    assert.match(
      successes[0].message,
      /^Bob saved\./,
      "toast must name the observed (persisted) rename, not the stale pre-save name",
    );
    assert.doesNotMatch(
      successes[0].message,
      /Alice/,
      "toast must not report the stale pre-save name",
    );
  } finally {
    cap.restore();
  }
});

test("test_published_definition_rename_notice_uses_observed_persona_name", async () => {
  const cap = captureToasts();
  try {
    // Definition-only publish edit: both agent candidates are null, so the
    // publish notice must name the DEFINITION from the observed persona, not
    // fall back to the pre-save `def.displayName`.
    const opts = makeOpts({
      ctx: {
        kind: "definition-only",
        definition: makeDefinition({ displayName: "Alice" }),
      },
      personaInput: makePersonaInput({ displayName: "Bob" }),
      publishCatalogUpdates: true,
      updatePersonaAndPublish: async () => ({ publicationStatus: "published" }),
      refetchStores: async () => ({
        persona: makeDefinition({ displayName: "Bob" }),
        agent: null,
      }),
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(result, true, "published definition rename is a full success");
    const successes = cap.captured.filter((c) => c.kind === "success");
    assert.equal(successes.length, 1, "exactly one success toast");
    assert.match(
      successes[0].message,
      /^Updated Bob and published it/,
      "publish notice must name the observed (persisted) rename",
    );
    assert.doesNotMatch(
      successes[0].message,
      /Alice/,
      "publish notice must not report the stale pre-save definition name",
    );
  } finally {
    cap.restore();
  }
});

// ── Test family 10: P1-1 regression — publish failure after persona persists (Carl round-5) ─
//
// `update_persona_and_publish` saves the persona first then calls the retain
// callback (prepare_persona_publication). If the retain call throws, the command
// propagates the error — but the persona fields are already on disk. The
// coordinator catches the throw, sets caughtError, then runs a settlement refetch
// that sees the persona fields match the submission → `persisted = true`. Before
// this fix, `publishFailed` was never set and the coordinator entered the full-
// success branch, called `onDone`, and showed the publish success toast despite
// the relay never having been reached.
//
// After the fix: when `publishCatalogUpdates` is true AND the command threw AND
// the persona persisted, `publishFailed` is set. The `!observedRemainder &&
// publishFailed` guard fires BEFORE the full-success branch — `onDone` is NOT
// called, a warning toast names the failure, and the coordinator returns false so
// the dialog stays open for a retry.

test("test_publish_throws_post_persist_does_not_settle_as_full_success", async () => {
  // The critical scenario: updatePersonaAndPublish throws (the retain/enqueue
  // step failed) but the persona fields ARE observed as persisted in the
  // settlement refetch. Before the fix this reached `onDone` and returned true;
  // after the fix it must return false and show a warning, NOT a success toast.
  const cap = captureToasts();
  try {
    const persisted = makeDefinition({ displayName: "Alice" });
    const opts = makeOpts({
      ctx: {
        kind: "definition-only",
        definition: makeDefinition({ displayName: "Alice" }),
      },
      personaInput: makePersonaInput({ displayName: "Alice" }),
      publishCatalogUpdates: true,
      updatePersonaAndPublish: async () => {
        // Simulates prepare_persona_publication throwing after save_personas ran.
        throw new Error("failed to open retention db");
      },
      // Settlement: persona IS on disk (save_personas ran before the throw).
      refetchStores: async () => ({ persona: persisted, agent: null }),
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(
      result,
      false,
      "a publish-path throw must not settle as full success even when the persona persisted",
    );
    assert.equal(
      opts._calls.onDone,
      0,
      "onDone must NOT be called when publish failed — dialog must stay open for retry",
    );
    const successes = cap.captured.filter((c) => c.kind === "success");
    assert.equal(
      successes.length,
      0,
      "a publish failure must NOT show a success toast",
    );
    const warnings = cap.captured.filter((c) => c.kind === "warning");
    assert.equal(warnings.length, 1, "exactly one warning toast");
    assert.match(
      warnings[0].message,
      /saved locally.*not.*published|could not be published|saved.*not published/i,
      "warning must indicate the persona saved locally but publication failed",
    );
    assert.match(
      warnings[0].message,
      /retention db|failed to open|failed/i,
      "warning must include the underlying error text",
    );
  } finally {
    cap.restore();
  }
});

test("test_publish_throws_post_persist_returns_false_not_partial_failure_toast", async () => {
  // Distinguishes publishFailed from ordinary partial failure: the persona IS
  // in persistedParts (observed match), failedParts is empty (no other writes
  // failed), so `observedRemainder` is false. Only the publishFailed guard fires.
  // The warning must NOT say "profile saved. … failed: …" (the partial-failure
  // wording) — it must say the persona saved but publishing failed.
  const cap = captureToasts();
  try {
    const persisted = makeDefinition({ displayName: "Alice-renamed" });
    const opts = makeOpts({
      ctx: {
        kind: "definition-only",
        definition: makeDefinition({ displayName: "Alice" }),
      },
      personaInput: makePersonaInput({ displayName: "Alice-renamed" }),
      publishCatalogUpdates: true,
      updatePersonaAndPublish: async () => {
        throw new Error("relay unreachable");
      },
      refetchStores: async () => ({ persona: persisted, agent: null }),
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(result, false, "publish failure returns false");
    assert.equal(opts._calls.onDone, 0, "onDone must not be called");
    const warnings = cap.captured.filter((c) => c.kind === "warning");
    assert.equal(warnings.length, 1, "exactly one warning");
    // Must mention "saved" AND "publishing failed" or similar.
    assert.match(
      warnings[0].message,
      /Alice-renamed.*saved|saved.*Alice-renamed/i,
      "warning must name the saved persona",
    );
    assert.match(
      warnings[0].message,
      /could not be published|saved locally.*not.*published|not published/i,
      "warning must indicate publishing was the failure, not the save itself",
    );
    // Must NOT use the partial-failure wording "profile saved. profile failed"
    assert.doesNotMatch(
      warnings[0].message,
      /profile failed/i,
      "must not use the partial D/I-failure wording for a publish-only failure",
    );
  } finally {
    cap.restore();
  }
});

// ── Test family 11: P1-1 publish retry seam (Carl round-5 pass-2) ─────────────
//
// When the initial save+publish command throws after the persona is on disk, the
// coordinator now attempts a publish-only retry via publishRetry(def.id) before
// reporting failure. This keeps the toast copy honest:
//
//   - If the retry succeeds: full-success path fires — onDone is called and the
//     success toast names the persona and publication status.
//   - If the retry also fails: terminal warning without "reopen to retry"
//     (the flush loop holds the enqueued head and will retry automatically).
//   - If no publishRetry seam is provided: same terminal warning, no false
//     "reopen to retry" copy.
//
// The existing family-10 tests remain unchanged: they verify the publishFailed
// flag is set and onDone is not called on the initial throw. These family-11
// tests cover the retry layer on top.

test("test_publish_retry_succeeds_settles_as_full_success", async () => {
  // Consume the same observed partial-outcome contract as the Rust command
  // test: the rename persisted, the command returned the strict preparation
  // error, and recovery is therefore a publish-only retry.
  const cap = captureToasts();
  try {
    const contract = partialPublishOutcomeContract;
    const persisted = makeDefinition({
      id: contract.personaId,
      displayName: contract.afterDisplayName,
    });
    let retryCalls = 0;
    let announceRetryStarted;
    const retryStarted = new Promise((resolve) => {
      announceRetryStarted = resolve;
    });
    let allowRetryCompletion;
    const retryCompletionGate = new Promise((resolve) => {
      allowRetryCompletion = resolve;
    });
    const opts = makeOpts({
      ctx: {
        kind: "definition-only",
        definition: makeDefinition({
          id: contract.personaId,
          displayName: contract.beforeDisplayName,
        }),
      },
      personaInput: makePersonaInput({
        id: contract.personaId,
        displayName: contract.afterDisplayName,
      }),
      publishCatalogUpdates: true,
      updatePersonaAndPublish: async () => {
        throw new Error(contract.commandErrorContains);
      },
      refetchStores: async () => ({ persona: persisted, agent: null }),
      publishRetry: async (personaId) => {
        assert.equal(
          personaId,
          contract.personaId,
          "retry must target the definition from the observed command outcome",
        );
        retryCalls++;
        announceRetryStarted();
        await retryCompletionGate;
        return {
          persona: persisted,
          publicationStatus: contract.retryPublicationStatus,
        };
      },
    });

    const coordinatorResult = runAgentSaveCoordinator(opts);
    await retryStarted;
    assert.equal(
      opts._calls.onDone,
      0,
      "dialog must remain open while the publish retry is still in flight",
    );

    allowRetryCompletion();
    const result = await coordinatorResult;

    assert.equal(
      result,
      true,
      "coordinator must return true when the publish retry succeeds",
    );
    assert.equal(
      opts._calls.onDone,
      1,
      "dialog closes exactly once after retry completion",
    );
    assert.equal(retryCalls, 1, "publishRetry must be called exactly once");
    const warnings = cap.captured.filter((c) => c.kind === "warning");
    assert.equal(warnings.length, 0, "no warning toast when retry succeeds");
    const successes = cap.captured.filter((c) => c.kind === "success");
    assert.equal(successes.length, 1, "success toast must fire after retry");
  } finally {
    cap.restore();
  }
});

test("test_publish_retry_failure_shows_terminal_warning_not_reopen", async () => {
  // Both the initial publish and the retry fail — should report a terminal
  // state without "reopen to retry publishing".
  const cap = captureToasts();
  try {
    const persisted = makeDefinition({ displayName: "Alice" });
    const opts = makeOpts({
      ctx: {
        kind: "definition-only",
        definition: makeDefinition({ displayName: "Alice" }),
      },
      personaInput: makePersonaInput({ displayName: "Alice" }),
      publishCatalogUpdates: true,
      updatePersonaAndPublish: async () => {
        throw new Error("relay unreachable: connection refused");
      },
      refetchStores: async () => ({ persona: persisted, agent: null }),
      publishRetry: async () => {
        throw new Error("relay still unreachable");
      },
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(
      result,
      false,
      "coordinator must return false when retry also fails",
    );
    assert.equal(opts._calls.onDone, 0, "onDone must not be called");
    const warnings = cap.captured.filter((c) => c.kind === "warning");
    assert.equal(warnings.length, 1, "exactly one warning toast");
    // Positive match: must end with the honest terminal copy — saved locally,
    // could not be published, no recovery instruction of any kind.
    assert.match(
      warnings[0].message,
      /saved locally, but could not be published to the catalog:/i,
      "terminal retry-failure copy must state saved locally but not published",
    );
    // Robust ban on any recovery instruction — fragment-level to survive rewording.
    assert.doesNotMatch(
      warnings[0].message,
      /reopen/i,
      "terminal state must not instruct the user to reopen — a fresh open seeds persisted values and publishes nothing",
    );
    assert.doesNotMatch(
      warnings[0].message,
      /retried automatically/i,
      "terminal state must not claim automatic retry — preparation threw, nothing is durably queued",
    );
  } finally {
    cap.restore();
  }
});

test("test_no_publish_retry_seam_shows_terminal_warning_not_reopen", async () => {
  // No publishRetry provided — coordinator must still report only the honest
  // terminal state: saved locally, not published, no recovery instruction.
  const cap = captureToasts();
  try {
    const persisted = makeDefinition({ displayName: "Alice" });
    const opts = makeOpts({
      ctx: {
        kind: "definition-only",
        definition: makeDefinition({ displayName: "Alice" }),
      },
      personaInput: makePersonaInput({ displayName: "Alice" }),
      publishCatalogUpdates: true,
      updatePersonaAndPublish: async () => {
        throw new Error("failed to open retention db");
      },
      refetchStores: async () => ({ persona: persisted, agent: null }),
      // publishRetry intentionally omitted (no seam available).
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(
      result,
      false,
      "coordinator must return false with no retry seam",
    );
    assert.equal(opts._calls.onDone, 0, "onDone must not be called");
    const warnings = cap.captured.filter((c) => c.kind === "warning");
    assert.equal(warnings.length, 1, "exactly one warning toast");
    // Positive match: same honest terminal copy shape.
    assert.match(
      warnings[0].message,
      /saved locally, but could not be published to the catalog:/i,
      "no-seam terminal copy must state saved locally but not published",
    );
    // Robust ban on any recovery instruction — fragment-level.
    assert.doesNotMatch(
      warnings[0].message,
      /reopen/i,
      "no-seam terminal state must not instruct the user to reopen",
    );
    assert.doesNotMatch(
      warnings[0].message,
      /retried automatically/i,
      "no-seam terminal state must not claim automatic retry — nothing was enqueued",
    );
  } finally {
    cap.restore();
  }
});

// ── Test family 12: P1 — combined saves (D+I, D+L) bypass recovery ─────────────
//
// When `publishCatalogUpdates` is true and the command throws after persist,
// `publishFailed` is set and `firstError` suppresses I/L writes. A combined
// D+I or D+L save must settle publication independently — before advancing —
// so the full D+I or D+L path can complete when the retry succeeds, and must
// name the unpublished catalog alongside the unsaved I/L remainder when it
// fails.
//
// Mutation acceptance: restoring the original `!observedRemainder` gate must
// turn these tests RED because the retry is never called for combined saves.

test("test_combined_di_save_publish_retry_succeeds_continues_instance_write", async () => {
  // D+I combined save: publish throws after persist, retry succeeds — I write
  // must proceed and the whole save must close as full success.
  const cap = captureToasts();
  try {
    const persistedDef = makeDefinition({ displayName: "Alice" });
    const persistedInst = makeInstance({ name: "Alice" });
    let retryCalls = 0;
    let agentCalls = 0;
    const opts = makeOpts({
      ctx: {
        kind: "instance-with-definition",
        definition: makeDefinition({ displayName: "Alice" }),
        instance: makeInstance(),
      },
      personaInput: makePersonaInput({ displayName: "Alice" }),
      agentInput: makeAgentInput({ name: "Alice" }),
      publishCatalogUpdates: true,
      updatePersonaAndPublish: async () => {
        throw new Error("failed to open retention db");
      },
      updateManagedAgent: async () => {
        agentCalls++;
        return { agent: persistedInst, profileSyncError: null };
      },
      refetchStores: async () => ({
        persona: persistedDef,
        agent: persistedInst,
      }),
      publishRetry: async () => {
        retryCalls++;
        return { persona: persistedDef, publicationStatus: "published" };
      },
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(
      result,
      true,
      "full success when retry + I write both succeed",
    );
    assert.equal(
      opts._calls.onDone,
      1,
      "onDone must be called on full success",
    );
    assert.equal(
      retryCalls,
      1,
      "publishRetry must be called once for the early retry",
    );
    assert.equal(
      agentCalls,
      1,
      "instance write must proceed after retry success",
    );
    const warnings = cap.captured.filter((c) => c.kind === "warning");
    assert.equal(warnings.length, 0, "no warning when everything succeeds");
    const successes = cap.captured.filter((c) => c.kind === "success");
    assert.equal(successes.length, 1, "success toast must fire");
  } finally {
    cap.restore();
  }
});

test("test_combined_dl_save_publish_retry_succeeds_continues_policy_write", async () => {
  // D+L combined save: publish throws after persist, retry succeeds — policy
  // write must proceed and the whole save must close as full success.
  const cap = captureToasts();
  try {
    const persistedDef = makeDefinition({ displayName: "Alice" });
    const persistedInst = makeInstance({ autoRestartOnConfigChange: true });
    let retryCalls = 0;
    let policyCalls = 0;
    const opts = makeOpts({
      ctx: {
        kind: "instance-with-definition",
        definition: makeDefinition({ displayName: "Alice" }),
        instance: makeInstance(),
      },
      personaInput: makePersonaInput({ displayName: "Alice" }),
      agentInput: null,
      policySets: [{ type: "autoRestart", pubkey: "pk-abc", value: true }],
      publishCatalogUpdates: true,
      updatePersonaAndPublish: async () => {
        throw new Error("failed to open retention db");
      },
      setAutoRestart: async () => {
        policyCalls++;
      },
      refetchStores: async () => ({
        persona: persistedDef,
        agent: persistedInst,
      }),
      publishRetry: async () => {
        retryCalls++;
        return { persona: persistedDef, publicationStatus: "published" };
      },
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(
      result,
      true,
      "full success when retry + policy write both succeed",
    );
    assert.equal(
      opts._calls.onDone,
      1,
      "onDone must be called on full success",
    );
    assert.equal(retryCalls, 1, "publishRetry must be called once");
    assert.equal(
      policyCalls,
      1,
      "policy write must proceed after retry success",
    );
    const warnings = cap.captured.filter((c) => c.kind === "warning");
    assert.equal(warnings.length, 0, "no warning when everything succeeds");
  } finally {
    cap.restore();
  }
});

test("test_combined_di_save_publish_retry_failure_names_both_catalog_and_remainder", async () => {
  // D+I combined save: publish throws, retry also fails — toast must name
  // the profile as saved, instance settings as not saved, and catalog failure
  // as the reason. Never claim reopen can republish the catalog.
  const cap = captureToasts();
  try {
    const persistedDef = makeDefinition({ displayName: "Alice" });
    const opts = makeOpts({
      ctx: {
        kind: "instance-with-definition",
        definition: makeDefinition({ displayName: "Alice" }),
        instance: makeInstance(),
      },
      personaInput: makePersonaInput({ displayName: "Alice" }),
      agentInput: makeAgentInput({ name: "Alice-renamed" }),
      publishCatalogUpdates: true,
      updatePersonaAndPublish: async () => {
        throw new Error("relay unreachable");
      },
      updateManagedAgent: async () => {
        throw new Error("should not be called");
      },
      refetchStores: async () => ({ persona: persistedDef, agent: null }),
      publishRetry: async () => {
        throw new Error("still unreachable");
      },
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(result, false, "combined failure returns false");
    assert.equal(opts._calls.onDone, 0, "onDone must not be called");
    // Instance write must NOT have been attempted.
    assert.equal(
      opts._calls.updateManagedAgent,
      0,
      "instance write must be skipped",
    );
    const warnings = cap.captured.filter((c) => c.kind === "warning");
    assert.equal(warnings.length, 1, "exactly one warning");
    // Must name D as saved and publication failure.
    assert.match(
      warnings[0].message,
      /profile saved/i,
      "warning must name profile as saved",
    );
    assert.match(
      warnings[0].message,
      /catalog publication failed|could not be published/i,
      "warning must name the publication failure",
    );
    // Must name the I/L remainder as not saved.
    assert.match(
      warnings[0].message,
      /instance settings/i,
      "warning must name instance settings as the unsaved remainder",
    );
  } finally {
    cap.restore();
  }
});

test("test_combined_di_save_no_retry_seam_names_both_catalog_and_remainder", async () => {
  // D+I combined save: publish throws, no retry seam provided — toast must
  // still name catalog failure + unsaved I/L remainder without a publishRetry.
  const cap = captureToasts();
  try {
    const persistedDef = makeDefinition({ displayName: "Alice" });
    const opts = makeOpts({
      ctx: {
        kind: "instance-with-definition",
        definition: makeDefinition({ displayName: "Alice" }),
        instance: makeInstance(),
      },
      personaInput: makePersonaInput({ displayName: "Alice" }),
      agentInput: makeAgentInput({ name: "Alice-renamed" }),
      publishCatalogUpdates: true,
      updatePersonaAndPublish: async () => {
        throw new Error("relay unreachable");
      },
      refetchStores: async () => ({ persona: persistedDef, agent: null }),
      // publishRetry intentionally omitted.
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(result, false, "combined failure without seam returns false");
    assert.equal(opts._calls.onDone, 0, "onDone must not be called");
    assert.equal(
      opts._calls.updateManagedAgent,
      0,
      "instance write must be skipped",
    );
    const warnings = cap.captured.filter((c) => c.kind === "warning");
    assert.equal(warnings.length, 1, "exactly one warning");
    assert.match(
      warnings[0].message,
      /profile saved/i,
      "warning must name profile as saved",
    );
    assert.match(
      warnings[0].message,
      /catalog publication failed|could not be published/i,
      "warning must name the catalog failure",
    );
    assert.match(
      warnings[0].message,
      /instance settings/i,
      "warning must name instance settings as the unsaved remainder",
    );
  } finally {
    cap.restore();
  }
});

test("test_combined_dl_save_publish_retry_failure_names_both_catalog_and_remainder", async () => {
  // D+L combined save: publish throws after persist, retry also fails — toast
  // must name the profile as saved, the policy remainder as not saved, and
  // catalog publication as the failure reason. Policy write must NOT be called.
  //
  // Symmetric to test_combined_di_save_publish_retry_failure_names_both_catalog_and_remainder
  // for the policy (L) path. Mutation acceptance: restoring the original
  // !observedRemainder gate suppresses the early retry entirely, so the policy
  // write is also never blocked — the toast no longer names the policy remainder
  // and these assertions turn RED.
  const cap = captureToasts();
  try {
    const persistedDef = makeDefinition({ displayName: "Alice" });
    const persistedInst = makeInstance({ autoRestartOnConfigChange: false });
    let setAutoRestartCalls = 0;
    const opts = makeOpts({
      ctx: {
        kind: "instance-with-definition",
        definition: makeDefinition({ displayName: "Alice" }),
        instance: makeInstance(),
      },
      personaInput: makePersonaInput({ displayName: "Alice" }),
      agentInput: null,
      policySets: [{ type: "autoRestart", pubkey: "pk-abc", value: true }],
      publishCatalogUpdates: true,
      updatePersonaAndPublish: async () => {
        throw new Error("relay unreachable");
      },
      setAutoRestart: async () => {
        setAutoRestartCalls++;
      },
      refetchStores: async () => ({
        persona: persistedDef,
        agent: persistedInst,
      }),
      publishRetry: async () => {
        throw new Error("still unreachable");
      },
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(result, false, "combined D+L failure returns false");
    assert.equal(opts._calls.onDone, 0, "onDone must not be called");
    // Policy write must NOT have been attempted — firstError blocks step 3.
    assert.equal(
      setAutoRestartCalls,
      0,
      "policy write must be skipped when publication retry failed",
    );
    const warnings = cap.captured.filter((c) => c.kind === "warning");
    assert.equal(warnings.length, 1, "exactly one warning");
    // Must name D as saved.
    assert.match(
      warnings[0].message,
      /profile saved/i,
      "warning must name profile as saved",
    );
    // Must name publication as the failure reason.
    assert.match(
      warnings[0].message,
      /catalog publication failed|could not be published/i,
      "warning must name the catalog publication failure",
    );
    // Must name the L remainder (policy) as not saved.
    assert.match(
      warnings[0].message,
      /auto-restart policy/i,
      "warning must name the auto-restart policy as the unsaved remainder",
    );
  } finally {
    cap.restore();
  }
});

// ── Test family 13: description settlement (Carl round-6 P2) ─────────────────
//
// `observedStateMatchesPersonaInput` must compare description using the same
// canonical normalization as the Rust backend (normalize_description): trim,
// blank/absent → null. Prohibited bytes (e.g. U+200B) must NOT be stripped —
// a submitted description containing U+200B returns a non-null normalized value
// that will not match the observed null (unchanged store), so settlement
// correctly reports the edit as not persisted.

test("test_rejected_description_ordinary_save_does_not_close", async () => {
  // Rust rejects the U+200B description before writing — persona store is
  // unchanged. observedStateMatchesPersonaInput must detect the mismatch
  // (submitted "\u200B" normalizes to "\u200B", not null) and NOT close.
  const cap = captureToasts();
  try {
    const originalPersona = makeDefinition({ description: null });
    const opts = makeOpts({
      personaInput: makePersonaInput({ description: "\u200B" }),
      updatePersona: async () => {
        throw new Error("description contains prohibited characters");
      },
      refetchStores: async () => ({ persona: originalPersona, agent: null }),
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(result, false, "rejected description edit must not succeed");
    assert.equal(
      opts._calls.onDone,
      0,
      "dialog must not close when description was rejected by the backend",
    );
  } finally {
    cap.restore();
  }
});

test("test_rejected_description_publish_path_does_not_retry_or_close", async () => {
  // Same rejection on the save-and-publish path. The backend rejects before
  // writing; no old-record publication retry must fire and the dialog must stay
  // open (onDone 0).
  const cap = captureToasts();
  try {
    const originalPersona = makeDefinition({ description: null });
    let retryCalls = 0;
    const opts = makeOpts({
      ctx: {
        kind: "definition-only",
        definition: makeDefinition({ description: null }),
      },
      personaInput: makePersonaInput({ description: "\u200B" }),
      publishCatalogUpdates: true,
      updatePersonaAndPublish: async () => {
        throw new Error("description contains prohibited characters");
      },
      refetchStores: async () => ({ persona: originalPersona, agent: null }),
      publishRetry: async () => {
        retryCalls++;
        return { persona: originalPersona, publicationStatus: "published" };
      },
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(
      result,
      false,
      "rejected description publish must not succeed",
    );
    assert.equal(
      opts._calls.onDone,
      0,
      "dialog must not close when publish description was rejected",
    );
    // No retry must fire: the write never persisted, so there is no old record
    // to republish and publishRetry is for post-persist relay failures only.
    // Settlement detects non-persistence and routes to partial-failure, not retry.
    assert.equal(
      retryCalls,
      0,
      "publishRetry must not fire when the description edit was rejected before writing",
    );
  } finally {
    cap.restore();
  }
});

test("test_rejected_description_combined_di_does_not_advance_to_instance_write", async () => {
  // Combined D+I save: rejected description means D did not persist. The
  // coordinator must stop at D settlement and NOT advance to the I write.
  const cap = captureToasts();
  try {
    const originalPersona = makeDefinition({ description: null });
    let agentCalls = 0;
    const opts = makeOpts({
      ctx: {
        kind: "instance-with-definition",
        definition: makeDefinition({ description: null }),
        instance: makeInstance(),
      },
      personaInput: makePersonaInput({ description: "\u200B" }),
      agentInput: makeAgentInput({ name: "Alice-renamed" }),
      publishCatalogUpdates: false,
      updatePersona: async () => {
        throw new Error("description contains prohibited characters");
      },
      updateManagedAgent: async () => {
        agentCalls++;
        return {
          agent: makeInstance({ name: "Alice-renamed" }),
          profileSyncError: null,
        };
      },
      refetchStores: async () => ({ persona: originalPersona, agent: null }),
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(
      result,
      false,
      "combined D+I with rejected description must not succeed",
    );
    assert.equal(
      opts._calls.onDone,
      0,
      "dialog must not close when D was not persisted",
    );
    assert.equal(
      agentCalls,
      0,
      "I write must not advance when D settlement detected non-persistence",
    );
  } finally {
    cap.restore();
  }
});

test("test_description_clear_blank_to_null_settles_as_persisted", async () => {
  // Submitting a blank description clears it (normalize: blank → null).
  // The observed persona's description is null (cleared). Settlement must
  // recognise this as a successful write.
  const persistedPersona = makeDefinition({ description: null });
  const opts = makeOpts({
    personaInput: makePersonaInput({ description: "   " }),
    refetchStores: async () => ({ persona: persistedPersona, agent: null }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(
    result,
    true,
    "blank description clearing must settle as success",
  );
  assert.equal(
    opts._calls.onDone,
    1,
    "onDone must be called after successful clear",
  );
});

test("test_description_trim_and_preserve_existing_settles_correctly", async () => {
  // A description with leading/trailing whitespace trims to the stored value.
  // Settlement must recognise "  Good agent.  " as matching observed "Good agent.".
  // A concurrent unrelated definition field (displayName) is also updated and
  // must not disturb the description comparison.
  const persistedPersona = makeDefinition({
    displayName: "Alice-renamed",
    description: "Good agent.",
  });
  const opts = makeOpts({
    personaInput: makePersonaInput({
      displayName: "Alice-renamed",
      description: "  Good agent.  ",
    }),
    refetchStores: async () => ({ persona: persistedPersona, agent: null }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(
    result,
    true,
    "trimmed description must settle as success against observed stored value",
  );
  assert.equal(
    opts._calls.onDone,
    1,
    "onDone must be called when trimmed description matches observed",
  );
});

// ── Test family 14: effort settlement (Thufir round-1 IMPORTANT-1) ────────────
//
// `observedStateMatchesAgentInput` must compare effortLevel against the
// persisted record column (ManagedAgent.effortLevel) with tri-state semantics:
//   absent submission → skip comparison (field not being written)
//   null submission   → clear; settled when observed column is null/absent
//   string submission → set; settled when observed column equals submitted value
//
// The backend can reject before persistence (e.g. non-local agent rejects
// effort write). Without the comparison, a rejected effort edit settles as
// success and closes the dialog — the exact defect Thufir's probe confirmed.

test("test_effort_set_settles_when_observed_matches_submitted", async () => {
  // Submit effortLevel:"high" — backend persists it. Observed agent has
  // effortLevel:"high". Settlement must recognise the write succeeded.
  const inst = makeInstance({ effortLevel: "high" });
  const opts = makeOpts({
    ctx: { kind: "instance-only", instance: makeInstance() },
    agentInput: makeAgentInput({ effortLevel: "high" }),
    refetchStores: async () => ({ persona: null, agent: inst }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(result, true, "effort set must settle when observed matches");
  assert.equal(
    opts._calls.onDone,
    1,
    "onDone must be called on effort set success",
  );
});

test("test_effort_clear_settles_when_observed_is_null", async () => {
  // Submit effortLevel:null — a clear. Observed agent has no effortLevel.
  // Settlement must recognise the clear succeeded.
  const inst = makeInstance({ effortLevel: null });
  const opts = makeOpts({
    ctx: {
      kind: "instance-only",
      instance: makeInstance({ effortLevel: "low" }),
    },
    agentInput: makeAgentInput({ effortLevel: null }),
    refetchStores: async () => ({ persona: null, agent: inst }),
  });

  const result = await runAgentSaveCoordinator(opts);

  assert.equal(
    result,
    true,
    "effort clear must settle when observed is null/absent",
  );
  assert.equal(
    opts._calls.onDone,
    1,
    "onDone must be called on effort clear success",
  );
});

test("test_rejected_effort_ordinary_save_does_not_close", async () => {
  // Backend rejects the effort write before persistence — agent store is
  // unchanged (effortLevel:null). observedStateMatchesAgentInput must detect
  // the mismatch (submitted "high" ≠ observed null) and NOT close the dialog.
  const cap = captureToasts();
  try {
    const originalInst = makeInstance({ effortLevel: null });
    const opts = makeOpts({
      ctx: { kind: "instance-only", instance: makeInstance() },
      agentInput: makeAgentInput({ effortLevel: "high" }),
      updateManagedAgent: async () => {
        throw new Error("effort write rejected — non-local agent");
      },
      refetchStores: async () => ({ persona: null, agent: originalInst }),
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(result, false, "rejected effort edit must not succeed");
    assert.equal(
      opts._calls.onDone,
      0,
      "dialog must not close when effort was rejected by the backend",
    );
  } finally {
    cap.restore();
  }
});

test("test_rejected_effort_combined_di_does_not_advance_to_instance_write", async () => {
  // Combined D+I save: rejected effort (I-side) must report failure; the D
  // write succeeds but the coordinator must not report overall success.
  const cap = captureToasts();
  try {
    const persistedPersona = makeDefinition({ displayName: "Alice-renamed" });
    const originalInst = makeInstance({ effortLevel: null });
    const opts = makeOpts({
      ctx: {
        kind: "instance-with-definition",
        definition: makeDefinition(),
        instance: makeInstance(),
      },
      personaInput: makePersonaInput({ displayName: "Alice-renamed" }),
      agentInput: makeAgentInput({ effortLevel: "high" }),
      updateManagedAgent: async () => {
        throw new Error("effort write rejected — non-local agent");
      },
      refetchStores: async () => ({
        persona: persistedPersona,
        agent: originalInst,
      }),
    });

    const result = await runAgentSaveCoordinator(opts);

    assert.equal(
      result,
      false,
      "combined D+I with rejected effort must not succeed",
    );
    assert.equal(
      opts._calls.onDone,
      0,
      "dialog must not close when I effort was rejected",
    );
  } finally {
    cap.restore();
  }
});
