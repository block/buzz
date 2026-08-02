import assert from "node:assert/strict";
import test from "node:test";

import {
  applyProbeResult,
  canSubmitWhereToRun,
  emptyWhereToRunDraft,
  executionNodeRunOnValue,
  isExecutionNodeRunOn,
  parseExecutionNodeRunOn,
  providerConfigComplete,
  resolveBackendChangeIntent,
  resolveBackendIntent,
  resolveEffectiveBackendChange,
  whereToRunDraftForBackend,
} from "./whereToRunIntent.ts";

const probed = {
  ok: true,
  config_schema: {
    properties: { region: { type: "string" }, size: { type: "integer" } },
    required: ["region"],
  },
};

function providerDraft(overrides = {}) {
  return {
    ...emptyWhereToRunDraft,
    runOn: "blox",
    probedProvider: probed,
    providerConfig: { region: "us", size: "3" },
    ...overrides,
  };
}

test("provider selection blocks submit until the probe completes", () => {
  assert.equal(
    canSubmitWhereToRun(providerDraft({ probedProvider: null })),
    false,
  );
});

test("provider selection blocks submit while required config is missing", () => {
  const missing = providerDraft({ providerConfig: { size: "3" } });
  assert.equal(canSubmitWhereToRun(missing), false);
  assert.equal(providerConfigComplete(missing), false);
});

test("complete provider config allows submit", () => {
  assert.equal(canSubmitWhereToRun(providerDraft()), true);
});

test("local never gates submit", () => {
  assert.equal(canSubmitWhereToRun(emptyWhereToRunDraft), true);
});

test("local draft resolves to null intent", () => {
  assert.equal(resolveBackendIntent(emptyWhereToRunDraft), null);
});

test("provider draft resolves with coerced config values", () => {
  const intent = resolveBackendIntent(providerDraft());
  assert.deepEqual(intent, {
    type: "provider",
    id: "blox",
    config: { region: "us", size: 3 },
  });
});

test("execution-node runOn values round-trip through the shared helpers", () => {
  const nodeId = "b".repeat(64);
  const runOn = executionNodeRunOnValue(nodeId);
  assert.equal(runOn, `execution-node:${nodeId}`);
  assert.equal(isExecutionNodeRunOn(runOn), true);
  assert.equal(parseExecutionNodeRunOn(runOn), nodeId);
  assert.equal(isExecutionNodeRunOn("local"), false);
  assert.equal(parseExecutionNodeRunOn("blox"), null);
});

test("execution-node selection is immediately submittable and resolves to a node intent", () => {
  const draft = {
    ...emptyWhereToRunDraft,
    runOn: `execution-node:${"a".repeat(64)}`,
  };
  assert.equal(canSubmitWhereToRun(draft), true);
  assert.deepEqual(resolveBackendIntent(draft), {
    type: "execution-node",
    nodeId: "a".repeat(64),
  });
});

test("edit draft pre-selects the agent's persisted backend", () => {
  assert.deepEqual(
    whereToRunDraftForBackend({ type: "local" }),
    emptyWhereToRunDraft,
  );
  const nodeId = "c".repeat(64);
  assert.equal(
    whereToRunDraftForBackend({ type: "execution_node", nodeId }).runOn,
    executionNodeRunOnValue(nodeId),
  );
  assert.equal(
    whereToRunDraftForBackend({ type: "provider", id: "blox", config: {} })
      .runOn,
    "blox",
  );
});

test("an unchanged selection resolves to no backend change", () => {
  const nodeId = "c".repeat(64);
  assert.equal(
    resolveBackendChangeIntent(emptyWhereToRunDraft, { type: "local" }),
    null,
  );
  assert.equal(
    resolveBackendChangeIntent(
      { ...emptyWhereToRunDraft, runOn: executionNodeRunOnValue(nodeId) },
      { type: "execution_node", nodeId },
    ),
    null,
  );
  assert.equal(
    resolveBackendChangeIntent(providerDraft(), {
      type: "provider",
      id: "blox",
      config: {},
    }),
    null,
  );
});

test("a changed selection resolves to the target backend, any direction", () => {
  const nodeId = "c".repeat(64);
  const nodeDraft = {
    ...emptyWhereToRunDraft,
    runOn: executionNodeRunOnValue(nodeId),
  };
  // local → node
  assert.deepEqual(resolveBackendChangeIntent(nodeDraft, { type: "local" }), {
    type: "execution-node",
    nodeId,
  });
  // node → local (explicit local variant — create encodes local as null)
  assert.deepEqual(
    resolveBackendChangeIntent(emptyWhereToRunDraft, {
      type: "execution_node",
      nodeId,
    }),
    { type: "local" },
  );
  // node A → node B
  const otherNodeId = "d".repeat(64);
  assert.deepEqual(
    resolveBackendChangeIntent(nodeDraft, {
      type: "execution_node",
      nodeId: otherNodeId,
    }),
    { type: "execution-node", nodeId },
  );
  // provider → node
  assert.deepEqual(
    resolveBackendChangeIntent(nodeDraft, {
      type: "provider",
      id: "blox",
      config: {},
    }),
    { type: "execution-node", nodeId },
  );
  // local → provider
  assert.deepEqual(
    resolveBackendChangeIntent(providerDraft(), { type: "local" }),
    {
      type: "provider",
      id: "blox",
      config: { region: "us", size: 3 },
    },
  );
});

test("an undeployed execution-node agent converges to a re-deploy on save", () => {
  const nodeId = "c".repeat(64);
  const sameNodeDraft = {
    ...emptyWhereToRunDraft,
    runOn: executionNodeRunOnValue(nodeId),
  };
  // Half-deployed (backend persisted, no confirmed workload): re-deploy.
  assert.deepEqual(
    resolveEffectiveBackendChange(sameNodeDraft, {
      backend: { type: "execution_node", nodeId },
      backendAgentId: null,
    }),
    { type: "execution-node", nodeId },
  );
  // Fully deployed and unchanged: still a no-op.
  assert.equal(
    resolveEffectiveBackendChange(sameNodeDraft, {
      backend: { type: "execution_node", nodeId },
      backendAgentId: "workload-1",
    }),
    null,
  );
  // A real change wins over the convergence rule.
  assert.deepEqual(
    resolveEffectiveBackendChange(emptyWhereToRunDraft, {
      backend: { type: "execution_node", nodeId },
      backendAgentId: null,
    }),
    { type: "local" },
  );
  // Local agents never re-deploy.
  assert.equal(
    resolveEffectiveBackendChange(emptyWhereToRunDraft, {
      backend: { type: "local" },
      backendAgentId: null,
    }),
    null,
  );
});
// ── applyProbeResult: probe resolution must merge, not overwrite ─────────────
//
// Pins the seam that fixed the "Typewriter Eraser" (agent-create dialog's
// provider config fields losing keystrokes): a probe resolution prefills
// schema defaults *beneath* the user's in-flight config, never over it. The
// effect in WhereToRunSection keys probing on the provider's binary path, so
// the only probe writes that reach providerConfig are the ones pinned here.

const probeWithDefaults = {
  ok: true,
  config_schema: {
    properties: {
      context: { type: "string", title: "Kubeconfig context" },
      namespace: { type: "string", default: "buzz-agents-x1y2z3" },
      inactivity_seconds: { type: "number", default: 1800 },
    },
    required: ["namespace"],
  },
};

const unprobedDraft = {
  ...emptyWhereToRunDraft,
  runOn: "kubernetes",
};

test("probe resolution prefills schema defaults on a fresh draft", () => {
  const next = applyProbeResult(unprobedDraft, probeWithDefaults);
  assert.equal(next.probedProvider, probeWithDefaults);
  assert.deepEqual(next.providerConfig, {
    namespace: "buzz-agents-x1y2z3",
    inactivity_seconds: "1800",
  });
});

test("probe resolution keeps user-typed values over schema defaults", () => {
  const typed = {
    ...unprobedDraft,
    providerConfig: { context: "prod-us-west", namespace: "my-ns" },
  };
  const next = applyProbeResult(typed, probeWithDefaults);
  assert.deepEqual(next.providerConfig, {
    context: "prod-us-west",
    namespace: "my-ns",
    inactivity_seconds: "1800",
  });
});

test("probe resolution keeps a user-cleared field cleared", () => {
  // "" is a deliberate user state — coerceConfigValues drops empty numerics
  // and required-gating treats "" as incomplete; the probe must not undo it.
  const cleared = { ...unprobedDraft, providerConfig: { namespace: "" } };
  const next = applyProbeResult(cleared, probeWithDefaults);
  assert.equal(next.providerConfig.namespace, "");
});

test("a schema-less probe result records the probe without touching config", () => {
  const typed = { ...unprobedDraft, providerConfig: { context: "abc" } };
  const next = applyProbeResult(typed, { ok: true });
  assert.deepEqual(next.providerConfig, { context: "abc" });
  assert.deepEqual(next.probedProvider, { ok: true });
});

test("probe resolution preserves unrelated draft fields", () => {
  assert.equal(
    applyProbeResult(unprobedDraft, probeWithDefaults).runOn,
    "kubernetes",
  );
});

