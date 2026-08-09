import assert from "node:assert/strict";
import test from "node:test";

import {
  deriveAgentConfigFieldModel,
  deriveNumericDescriptors,
  structuredEnvKeys,
} from "./agentConfigCore.ts";
import { NUMERIC_KIND_MIN } from "../ui/buzzAgentModelTuningFields.tsx";

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
    maxTokensEnvVar: null,
    contextLimitEnvVar: null,
    maxRoundsEnvVar: null,
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

test("Goose exposes provider, model, and its real effort application key", () => {
  const model = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("goose", {
      modelEnvVar: "GOOSE_MODEL",
      providerEnvVar: "GOOSE_PROVIDER",
      thinkingEnvVar: "GOOSE_THINKING_EFFORT",
    }),
    scope: "global",
  });

  assert.equal(
    field(model, "effort").optionSource,
    "legacyProviderModelCatalog",
  );
  assert.deepEqual(field(model, "effort").currentPersistence, {
    kind: "envVar",
    key: "BUZZ_AGENT_THINKING_EFFORT",
  });
  assert.deepEqual(field(model, "effort").targetApplication, {
    kind: "envVar",
    key: "GOOSE_THINKING_EFFORT",
  });
});

test("Claude models effort as a deferred native ACP option", () => {
  const model = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("claude"),
    scope: "global",
  });

  assert.deepEqual(
    model.fields.map((item) => item.kind),
    ["model", "effort"],
  );
  assert.equal(
    field(model, "effort").render,
    "deferredUntilNativeOptionsAvailable",
  );
  assert.deepEqual(field(model, "effort").currentPersistence, {
    kind: "unavailable",
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

// ── Numeric descriptor derivation per runtime ─────────────────────────────
//
// The catalog-projected fields (maxTokensEnvVar, contextLimitEnvVar,
// maxRoundsEnvVar) determine which numeric descriptors appear in the field
// model. Capability facts flow catalog → descriptor → UI; no runtime-ID
// comparison decides numeric-field visibility.

test("buzz-agent derives three numeric descriptors from catalog fields", () => {
  const model = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("buzz-agent", {
      modelEnvVar: "BUZZ_AGENT_MODEL",
      providerEnvVar: "BUZZ_AGENT_PROVIDER",
      thinkingEnvVar: "BUZZ_AGENT_THINKING_EFFORT",
      maxTokensEnvVar: "BUZZ_AGENT_MAX_OUTPUT_TOKENS",
      contextLimitEnvVar: "BUZZ_AGENT_MAX_CONTEXT_TOKENS",
      maxRoundsEnvVar: "BUZZ_AGENT_MAX_ROUNDS",
    }),
    scope: "global",
  });

  const numericKinds = model.fields
    .filter((f) =>
      ["maxOutputTokens", "contextLimit", "maxRounds"].includes(f.kind),
    )
    .map((f) => f.kind);
  assert.deepEqual(numericKinds, [
    "maxOutputTokens",
    "contextLimit",
    "maxRounds",
  ]);

  const maxOutput = field(model, "maxOutputTokens");
  assert.equal(maxOutput.render, "control");
  assert.deepEqual(maxOutput.currentPersistence, {
    kind: "envVar",
    key: "BUZZ_AGENT_MAX_OUTPUT_TOKENS",
  });
  assert.deepEqual(maxOutput.targetApplication, {
    kind: "envVar",
    key: "BUZZ_AGENT_MAX_OUTPUT_TOKENS",
  });

  const ctx = field(model, "contextLimit");
  assert.deepEqual(ctx.currentPersistence, {
    kind: "envVar",
    key: "BUZZ_AGENT_MAX_CONTEXT_TOKENS",
  });

  const rounds = field(model, "maxRounds");
  assert.deepEqual(rounds.currentPersistence, {
    kind: "envVar",
    key: "BUZZ_AGENT_MAX_ROUNDS",
  });
});

test("Goose derives two numeric descriptors and no maxRounds", () => {
  const model = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("goose", {
      modelEnvVar: "GOOSE_MODEL",
      providerEnvVar: "GOOSE_PROVIDER",
      thinkingEnvVar: "GOOSE_THINKING_EFFORT",
      maxTokensEnvVar: "GOOSE_MAX_TOKENS",
      contextLimitEnvVar: "GOOSE_CONTEXT_LIMIT",
      maxRoundsEnvVar: null, // Goose has no max-rounds env var
    }),
    scope: "global",
  });

  const numericKinds = model.fields
    .filter((f) =>
      ["maxOutputTokens", "contextLimit", "maxRounds"].includes(f.kind),
    )
    .map((f) => f.kind);
  assert.deepEqual(numericKinds, ["maxOutputTokens", "contextLimit"]);
  assert.equal(
    field(model, "maxRounds"),
    undefined,
    "maxRounds must be absent for Goose",
  );

  assert.deepEqual(field(model, "maxOutputTokens").currentPersistence, {
    kind: "envVar",
    key: "GOOSE_MAX_TOKENS",
  });
  assert.deepEqual(field(model, "contextLimit").currentPersistence, {
    kind: "envVar",
    key: "GOOSE_CONTEXT_LIMIT",
  });
});

test("Claude derives no numeric descriptors", () => {
  const model = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("claude"),
    scope: "global",
  });

  const hasNumeric = model.fields.some((f) =>
    ["maxOutputTokens", "contextLimit", "maxRounds"].includes(f.kind),
  );
  assert.equal(hasNumeric, false, "Claude must have no numeric descriptors");
});

test("Codex derives no numeric descriptors", () => {
  const model = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("codex"),
    scope: "global",
  });

  const hasNumeric = model.fields.some((f) =>
    ["maxOutputTokens", "contextLimit", "maxRounds"].includes(f.kind),
  );
  assert.equal(hasNumeric, false, "Codex must have no numeric descriptors");
});

test("numeric descriptor value is read from env_vars when set", () => {
  const cfgWithTuning = {
    env_vars: {
      BUZZ_AGENT_MAX_OUTPUT_TOKENS: "8192",
      BUZZ_AGENT_MAX_CONTEXT_TOKENS: "100000",
      BUZZ_AGENT_MAX_ROUNDS: "25",
    },
    model: "test-model",
    preferred_runtime: null,
    provider: "anthropic",
  };
  const model = deriveAgentConfigFieldModel({
    config: cfgWithTuning,
    runtime: runtime("buzz-agent", {
      maxTokensEnvVar: "BUZZ_AGENT_MAX_OUTPUT_TOKENS",
      contextLimitEnvVar: "BUZZ_AGENT_MAX_CONTEXT_TOKENS",
      maxRoundsEnvVar: "BUZZ_AGENT_MAX_ROUNDS",
    }),
    scope: "global",
  });

  assert.equal(field(model, "maxOutputTokens").value, "8192");
  assert.equal(field(model, "contextLimit").value, "100000");
  assert.equal(field(model, "maxRounds").value, "25");
});

test("numeric descriptor value is null when env var is absent", () => {
  const cfgEmpty = {
    env_vars: {},
    model: "test-model",
    preferred_runtime: null,
    provider: null,
  };
  const model = deriveAgentConfigFieldModel({
    config: cfgEmpty,
    runtime: runtime("buzz-agent", {
      maxTokensEnvVar: "BUZZ_AGENT_MAX_OUTPUT_TOKENS",
      contextLimitEnvVar: "BUZZ_AGENT_MAX_CONTEXT_TOKENS",
      maxRoundsEnvVar: "BUZZ_AGENT_MAX_ROUNDS",
    }),
    scope: "global",
  });

  assert.equal(field(model, "maxOutputTokens").value, null);
  assert.equal(field(model, "contextLimit").value, null);
  assert.equal(field(model, "maxRounds").value, null);
});

// ── structuredEnvKeys: rendered-descriptor ownership ─────────────────────
//
// structuredEnvKeys accepts the descriptors a surface ACTUALLY renders and
// returns the env-var keys that surface owns. Keys only appear in the output
// when a first-class control for them renders — a persisted value must never
// have zero editors.
//
// Critical invariant: per-agent Goose passes only its two numeric descriptors
// (no effort descriptor, because no effort control renders there). The effort
// key (BUZZ_AGENT_THINKING_EFFORT) must NOT appear in the output — it must
// stay a visible generic env row where any saved value can be edited.

test("structuredEnvKeys_global_includes_effort_key_and_numeric_keys", () => {
  // Global surface renders effort + all numeric descriptors.
  const buzzAgentModel = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("buzz-agent", {
      modelEnvVar: "BUZZ_AGENT_MODEL",
      providerEnvVar: "BUZZ_AGENT_PROVIDER",
      thinkingEnvVar: "BUZZ_AGENT_THINKING_EFFORT",
      maxTokensEnvVar: "BUZZ_AGENT_MAX_OUTPUT_TOKENS",
      contextLimitEnvVar: "BUZZ_AGENT_MAX_CONTEXT_TOKENS",
      maxRoundsEnvVar: "BUZZ_AGENT_MAX_ROUNDS",
    }),
    scope: "global",
  });

  // Global renders all renderable descriptors.
  const renderedDescriptors = buzzAgentModel.fields.filter(
    (f) => f.render === "control",
  );
  const keys = structuredEnvKeys(renderedDescriptors);

  assert.ok(
    keys.includes("BUZZ_AGENT_THINKING_EFFORT"),
    "effort key must be hidden on global (effort control renders)",
  );
  assert.ok(
    keys.includes("BUZZ_AGENT_MAX_OUTPUT_TOKENS"),
    "maxOutputTokens key must be hidden on global",
  );
  assert.ok(
    keys.includes("BUZZ_AGENT_MAX_CONTEXT_TOKENS"),
    "contextLimit key must be hidden on global",
  );
  assert.ok(
    keys.includes("BUZZ_AGENT_MAX_ROUNDS"),
    "maxRounds key must be hidden on global",
  );
});

test("structuredEnvKeys_per_agent_buzz_agent_includes_effort_and_numeric_keys", () => {
  // Per-agent buzz-agent renders effort + all 3 numeric descriptors.
  const buzzAgentModel = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("buzz-agent", {
      thinkingEnvVar: "BUZZ_AGENT_THINKING_EFFORT",
      maxTokensEnvVar: "BUZZ_AGENT_MAX_OUTPUT_TOKENS",
      contextLimitEnvVar: "BUZZ_AGENT_MAX_CONTEXT_TOKENS",
      maxRoundsEnvVar: "BUZZ_AGENT_MAX_ROUNDS",
    }),
    scope: "definition",
  });

  const renderedDescriptors = buzzAgentModel.fields.filter(
    (f) => f.render === "control",
  );
  const keys = structuredEnvKeys(renderedDescriptors);

  assert.ok(keys.includes("BUZZ_AGENT_THINKING_EFFORT"), "effort key present");
  assert.ok(keys.includes("BUZZ_AGENT_MAX_OUTPUT_TOKENS"), "maxTokens present");
  assert.ok(
    keys.includes("BUZZ_AGENT_MAX_CONTEXT_TOKENS"),
    "contextLimit present",
  );
  assert.ok(keys.includes("BUZZ_AGENT_MAX_ROUNDS"), "maxRounds present");
});

test("structuredEnvKeys_per_agent_goose_excludes_effort_key_discriminating_invariant", () => {
  // Per-agent Goose: effort migration is out of scope, so no effort control
  // renders on the per-agent surface for Goose. Only the 2 numeric descriptors
  // are passed as the rendered set. The effort persistence key
  // (BUZZ_AGENT_THINKING_EFFORT) must NOT appear in the output — any saved
  // value must remain visible and editable as a generic env row.
  const gooseModel = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("goose", {
      thinkingEnvVar: "GOOSE_THINKING_EFFORT",
      maxTokensEnvVar: "GOOSE_MAX_TOKENS",
      contextLimitEnvVar: "GOOSE_CONTEXT_LIMIT",
    }),
    scope: "definition",
  });

  // Simulate per-agent surface: only the numeric descriptors render (no effort
  // control for Goose per-agent — effort migration is out of scope).
  const numericDescriptorsOnly = gooseModel.fields.filter((f) =>
    ["maxOutputTokens", "contextLimit", "maxRounds"].includes(f.kind),
  );

  const keys = structuredEnvKeys(numericDescriptorsOnly);

  assert.equal(
    keys.includes("BUZZ_AGENT_THINKING_EFFORT"),
    false,
    "effort persistence key must NOT be hidden for Goose per-agent — no editor would replace it",
  );
  assert.ok(
    keys.includes("GOOSE_MAX_TOKENS"),
    "maxTokens key must be present (control renders)",
  );
  assert.ok(
    keys.includes("GOOSE_CONTEXT_LIMIT"),
    "contextLimit key must be present (control renders)",
  );
});

test("structuredEnvKeys_deferred_effort_excluded_from_result", () => {
  // A deferred effort descriptor (render !== "control") must not contribute
  // its key to the hidden set — the value has no editor on this surface.
  const claudeModel = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("claude"),
    scope: "global",
  });

  const allDescriptors = claudeModel.fields; // includes deferred effort
  const keys = structuredEnvKeys(allDescriptors);

  // Claude's deferred effort has currentPersistence.kind === "unavailable"
  // and render === "deferredUntilNativeOptionsAvailable"; no key emitted.
  assert.equal(
    keys.length,
    0,
    "deferred effort and model descriptors must not contribute hidden keys",
  );
});

// ── deriveNumericDescriptors: standalone helper ───────────────────────────
//
// The same logic that populates the numeric portion of deriveAgentConfigFieldModel
// is available as a standalone helper for per-agent surfaces that don't need
// the full field model.

test("deriveNumericDescriptors_undefined_runtime_returns_empty", () => {
  const ds = deriveNumericDescriptors(undefined);
  assert.deepEqual(ds, []);
});

test("deriveNumericDescriptors_runtime_with_all_three_fields", () => {
  const ds = deriveNumericDescriptors(
    runtime("buzz-agent", {
      maxTokensEnvVar: "BUZZ_AGENT_MAX_OUTPUT_TOKENS",
      contextLimitEnvVar: "BUZZ_AGENT_MAX_CONTEXT_TOKENS",
      maxRoundsEnvVar: "BUZZ_AGENT_MAX_ROUNDS",
    }),
  );
  assert.deepEqual(
    ds.map((d) => d.kind),
    ["maxOutputTokens", "contextLimit", "maxRounds"],
  );
  for (const d of ds) {
    assert.equal(d.render, "control");
    assert.equal(d.currentPersistence.kind, "envVar");
    assert.equal(d.value, null, "standalone helper returns null values");
  }
});

test("deriveNumericDescriptors_partial_fields_match_catalog_projection", () => {
  // Goose: two numeric fields, no maxRounds.
  const ds = deriveNumericDescriptors(
    runtime("goose", {
      maxTokensEnvVar: "GOOSE_MAX_TOKENS",
      contextLimitEnvVar: "GOOSE_CONTEXT_LIMIT",
      maxRoundsEnvVar: null,
    }),
  );
  assert.deepEqual(
    ds.map((d) => d.kind),
    ["maxOutputTokens", "contextLimit"],
  );
});

test("deriveNumericDescriptors_matches_deriveAgentConfigFieldModel_numeric_subset", () => {
  // The standalone helper must produce the same descriptor set (without values)
  // that deriveAgentConfigFieldModel embeds, so surfaces that call the helper
  // directly get a consistent policy with the full field model.
  const runtimeEntry = runtime("buzz-agent", {
    maxTokensEnvVar: "BUZZ_AGENT_MAX_OUTPUT_TOKENS",
    contextLimitEnvVar: "BUZZ_AGENT_MAX_CONTEXT_TOKENS",
    maxRoundsEnvVar: "BUZZ_AGENT_MAX_ROUNDS",
  });

  const standalone = deriveNumericDescriptors(runtimeEntry);
  const fromModel = deriveAgentConfigFieldModel({
    config,
    runtime: runtimeEntry,
    scope: "global",
  }).fields.filter((f) =>
    ["maxOutputTokens", "contextLimit", "maxRounds"].includes(f.kind),
  );

  // Kinds and keys must match; values differ (standalone returns null, model
  // reads from config).
  assert.deepEqual(
    standalone.map((d) => d.kind),
    fromModel.map((d) => d.kind),
    "descriptor kinds must match",
  );
  for (let i = 0; i < standalone.length; i++) {
    assert.deepEqual(
      standalone[i].currentPersistence,
      fromModel[i].currentPersistence,
      `persistence must match for descriptor ${i}`,
    );
  }
});

// ── NUMERIC_KIND_MIN: kind-specific input minima ──────────────────────────
//
// max output tokens and context limit must have min=1 (buzz-agent rejects 0).
// max rounds allows 0 (meaning unlimited).

test("NUMERIC_KIND_MIN_maxOutputTokens_is_1", () => {
  assert.equal(NUMERIC_KIND_MIN.maxOutputTokens, 1);
});

test("NUMERIC_KIND_MIN_contextLimit_is_1", () => {
  assert.equal(NUMERIC_KIND_MIN.contextLimit, 1);
});

test("NUMERIC_KIND_MIN_maxRounds_is_0", () => {
  assert.equal(NUMERIC_KIND_MIN.maxRounds, 0);
});
