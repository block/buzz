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

// ── external backend: the user runs buzz-acp themselves ─────────────────────

test("external selection needs no probe to submit", () => {
  const draft = { ...emptyWhereToRunDraft, runOn: "external" };
  // There is no provider binary to probe and no config schema, so gating on
  // probedProvider (as the provider path does) would disable submit forever.
  assert.equal(draft.probedProvider, null);
  assert.equal(providerConfigComplete(draft), true);
  assert.equal(canSubmitWhereToRun(draft), true);
});

test("external selection resolves to the external intent", () => {
  assert.deepEqual(
    resolveBackendIntent({ ...emptyWhereToRunDraft, runOn: "external" }),
    { type: "external" },
  );
});

test("local still resolves to no intent, distinct from external", () => {
  assert.equal(resolveBackendIntent(emptyWhereToRunDraft), null);
});
