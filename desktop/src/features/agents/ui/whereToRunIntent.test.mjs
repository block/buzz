import assert from "node:assert/strict";
import test from "node:test";

import {
  applyProbeResult,
  canSubmitWhereToRun,
  draftFromBackend,
  emptyWhereToRunDraft,
  providerConfigComplete,
  resolveBackendIntent,
  resolveBackendEdit,
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

const savedBlox = {
  type: "provider",
  id: "blox",
  config: { region: "us", size: 3 },
};

test("an unchanged or closed run-on editor sends no backend", () => {
  const agent = { backend: savedBlox, backendAgentId: "dep-1" };
  assert.deepEqual(resolveBackendEdit(agent, null), {
    needsConfirmation: false,
    valid: true,
  });
  const reopened = { ...draftFromBackend(savedBlox), probedProvider: probed };
  assert.equal(resolveBackendEdit(agent, reopened).backend, undefined);
});

test("leaving a deployed provider needs confirmation; a settings edit does not", () => {
  const deployed = { backend: savedBlox, backendAgentId: "dep-1" };
  const toLocal = resolveBackendEdit(deployed, emptyWhereToRunDraft);
  assert.deepEqual(toLocal.backend, { type: "local" });
  assert.equal(toLocal.needsConfirmation, true);

  const resized = resolveBackendEdit(
    deployed,
    providerDraft({ providerConfig: { region: "us", size: "5" } }),
  );
  assert.deepEqual(resized.backend?.config, { region: "us", size: 5 });
  assert.equal(resized.needsConfirmation, false);

  const undeployed = { backend: savedBlox, backendAgentId: null };
  assert.equal(
    resolveBackendEdit(undeployed, emptyWhereToRunDraft).needsConfirmation,
    false,
  );
});

test("moving to a provider stays invalid until its probe completes", () => {
  const local = { backend: { type: "local" }, backendAgentId: null };
  const pending = providerDraft({ probedProvider: null });
  assert.equal(resolveBackendEdit(local, pending).valid, false);
  assert.equal(resolveBackendEdit(local, providerDraft()).valid, true);
});
