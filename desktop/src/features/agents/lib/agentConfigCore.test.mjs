import assert from "node:assert/strict";
import test from "node:test";

import {
  deriveAgentConfigFieldModel,
  migrateLegacyEffortPersistence,
} from "./agentConfigCore.ts";

const config = {
  env_vars: { BUZZ_AGENT_THINKING_EFFORT: "high" },
  model: "test-model",
  preferred_runtime: null,
  provider: "anthropic",
};

function runtime(id, metadata = {}) {
  return {
    id,
    label: id,
    avatarUrl: "",
    availability: "available",
    command: id,
    binaryPath: id,
    defaultArgs: [],
    mcpCommand: null,
    modelEnvVar: null,
    providerEnvVar: null,
    thinkingEnvVar: null,
    installHint: "",
    installInstructionsUrl: "",
    canAutoInstall: false,
    underlyingCliPath: null,
    nodeRequired: false,
    authStatus: { status: "not_applicable" },
    loginHint: null,
    ...metadata,
  };
}

function field(model, kind) {
  return model.fields.find((candidate) => candidate.kind === kind);
}

test("Buzz Agent exposes provider, model, and Buzz-owned effort", () => {
  const model = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("buzz-agent", {
      modelEnvVar: "BUZZ_AGENT_MODEL",
      providerEnvVar: "BUZZ_AGENT_PROVIDER",
      thinkingEnvVar: "BUZZ_AGENT_THINKING_EFFORT",
    }),
    scope: "global",
  });

  assert.deepEqual(
    model.fields.map((item) => item.kind),
    ["provider", "model", "effort"],
  );
  assert.equal(field(model, "effort").optionSource, "buzzAgentCatalog");
  assert.deepEqual(field(model, "effort").targetApplication, {
    kind: "envVar",
    key: "BUZZ_AGENT_THINKING_EFFORT",
  });
});

test("Goose reads legacy effort but persists and applies its native key", () => {
  const model = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("goose", {
      modelEnvVar: "GOOSE_MODEL",
      providerEnvVar: "GOOSE_PROVIDER",
      thinkingEnvVar: "GOOSE_THINKING_EFFORT",
    }),
    scope: "global",
  });

  assert.equal(field(model, "effort").optionSource, "harnessNative");
  assert.deepEqual(field(model, "effort").currentPersistence, {
    kind: "envVar",
    key: "GOOSE_THINKING_EFFORT",
  });
  assert.equal(field(model, "effort").value, "high");
  assert.deepEqual(field(model, "effort").targetApplication, {
    kind: "envVar",
    key: "GOOSE_THINKING_EFFORT",
  });
});

test("Goose legacy effort migrates once without overwriting its native value", () => {
  const legacyOnly = migrateLegacyEffortPersistence(
    config,
    "GOOSE_THINKING_EFFORT",
  );
  assert.equal(legacyOnly.env_vars.GOOSE_THINKING_EFFORT, "high");
  assert.equal(legacyOnly.env_vars.BUZZ_AGENT_THINKING_EFFORT, undefined);

  const nativeWins = migrateLegacyEffortPersistence(
    {
      ...config,
      env_vars: {
        BUZZ_AGENT_THINKING_EFFORT: "high",
        GOOSE_THINKING_EFFORT: "low",
      },
    },
    "GOOSE_THINKING_EFFORT",
  );
  assert.equal(nativeWins.env_vars.GOOSE_THINKING_EFFORT, "low");
  assert.equal(nativeWins.env_vars.BUZZ_AGENT_THINKING_EFFORT, undefined);

  assert.equal(
    migrateLegacyEffortPersistence(config, "BUZZ_AGENT_THINKING_EFFORT"),
    config,
  );
});

test("Claude renders and persists its native ACP effort option", () => {
  const model = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("claude"),
    scope: "global",
  });

  assert.deepEqual(
    model.fields.map((item) => item.kind),
    ["model", "effort"],
  );
  assert.equal(field(model, "effort").render, "control");
  assert.deepEqual(field(model, "effort").currentPersistence, {
    kind: "envVar",
    key: "BUZZ_ACP_EFFORT",
  });
  assert.deepEqual(field(model, "effort").targetApplication, {
    kind: "acpConfigOption",
    id: "effort",
    category: "thought_level",
  });
});

test("Codex omits separate effort because model IDs own it", () => {
  const model = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("codex"),
    scope: "global",
  });

  assert.deepEqual(
    model.fields.map((item) => item.kind),
    ["model"],
  );
  assert.deepEqual(model.omissions, [
    { kind: "effort", reason: "ownedByModelId" },
  ]);
});

test("catalog mismatch cleanup is named and restricted to onboarding", () => {
  const selectedRuntime = runtime("buzz-agent", {
    modelEnvVar: "BUZZ_AGENT_MODEL",
    providerEnvVar: "BUZZ_AGENT_PROVIDER",
    thinkingEnvVar: "BUZZ_AGENT_THINKING_EFFORT",
  });
  const onboarding = deriveAgentConfigFieldModel({
    config,
    runtime: selectedRuntime,
    scope: "onboarding",
  });
  const evergreen = deriveAgentConfigFieldModel({
    config,
    runtime: selectedRuntime,
    scope: "instance",
  });

  assert.deepEqual(onboarding.dependentValuePolicy, {
    onContextChange: "resetDependentValues",
    onCatalogMismatch: "onboardingCleanup",
  });
  assert.deepEqual(evergreen.dependentValuePolicy, {
    onContextChange: "resetDependentValues",
    onCatalogMismatch: "explainOnly",
  });
});
