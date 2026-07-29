import assert from "node:assert/strict";
import test from "node:test";

import {
  applyProbeResult,
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

// Regression: a probe resolving after the user has typed must NOT wipe the
// user's input. Previously the probe handler set `providerConfig` to the schema
// defaults unconditionally, so an async probe completing mid-typing cleared the
// field (e.g. the Blox workstation_name input reset itself while typing).
test("applyProbeResult preserves user-entered config when probe resolves", () => {
  const draft = {
    ...emptyWhereToRunDraft,
    runOn: "blox",
    providerConfig: { region: "234" },
  };
  const next = applyProbeResult(draft, probed);
  assert.equal(next.providerConfig.region, "234");
  assert.equal(next.probedProvider, probed);
});

test("applyProbeResult seeds schema defaults on fresh provider selection", () => {
  const probedWithDefault = {
    ok: true,
    config_schema: {
      properties: {
        workstation_name: { type: "string" },
        bundle_tag: { type: "string", default: "sprig-latest" },
      },
      required: ["workstation_name"],
    },
  };
  const next = applyProbeResult(
    { ...emptyWhereToRunDraft, runOn: "blox", providerConfig: {} },
    probedWithDefault,
  );
  assert.equal(next.providerConfig.bundle_tag, "sprig-latest");
});

test("applyProbeResult: user-entered value beats a schema default", () => {
  const probedWithDefault = {
    ok: true,
    config_schema: {
      properties: { bundle_tag: { type: "string", default: "sprig-latest" } },
      required: [],
    },
  };
  const next = applyProbeResult(
    {
      ...emptyWhereToRunDraft,
      runOn: "blox",
      providerConfig: { bundle_tag: "custom" },
    },
    probedWithDefault,
  );
  assert.equal(next.providerConfig.bundle_tag, "custom");
});
