import assert from "node:assert/strict";
import test from "node:test";

import {
  canSubmitWhereToRun,
  emptyWhereToRunDraft,
  providerConfigComplete,
  resolveBackendIntent,
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

function meshDraft(overrides = {}) {
  return {
    ...emptyWhereToRunDraft,
    runOn: "mesh",
    meshModelId: "mesh/model:Q4",
    meshTarget: { endpointAddr: "10.0.0.1:9337", modelId: "mesh/model:Q4" },
    meshPatch: {
      acpCommand: "buzz-acp",
      agentCommand: "buzz-agent",
      agentArgs: ["acp"],
      mcpCommand: "",
      model: "mesh/model:Q4",
      envVars: {},
    },
    ...overrides,
  };
}

// ── Submit gating ───────────────────────────────────────────────────────────

test("provider selection blocks submit until the probe completes", () => {
  const unprobed = providerDraft({ probedProvider: null });
  assert.equal(canSubmitWhereToRun(unprobed), false);
});

test("provider selection blocks submit while required config is missing", () => {
  const missing = providerDraft({ providerConfig: { size: "3" } });
  assert.equal(canSubmitWhereToRun(missing), false);
  assert.equal(providerConfigComplete(missing), false);
});

test("complete provider config allows submit", () => {
  assert.equal(canSubmitWhereToRun(providerDraft()), true);
});

test("mesh selection blocks submit without a concrete serve target", () => {
  assert.equal(
    canSubmitWhereToRun(meshDraft({ meshTarget: null })),
    false,
    "a model name alone is not a startable mesh selection",
  );
  assert.equal(canSubmitWhereToRun(meshDraft({ meshModelId: "" })), false);
  assert.equal(canSubmitWhereToRun(meshDraft()), true);
});

test("local never gates submit", () => {
  assert.equal(canSubmitWhereToRun(emptyWhereToRunDraft), true);
});

// ── Intent resolution ────────────────────────────────────────────────────────

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

test("mesh draft resolves with target and patch", () => {
  const intent = resolveBackendIntent(meshDraft());
  assert.equal(intent.type, "mesh");
  assert.equal(intent.modelId, "mesh/model:Q4");
  assert.equal(intent.target.endpointAddr, "10.0.0.1:9337");
  assert.equal(intent.patch.agentCommand, "buzz-agent");
});

test("mesh draft without patch or target resolves to null, not a broken intent", () => {
  assert.equal(resolveBackendIntent(meshDraft({ meshPatch: null })), null);
  assert.equal(resolveBackendIntent(meshDraft({ meshTarget: null })), null);
});
