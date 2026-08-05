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

// Goose alias pairs — must come from the runtime descriptor (no fallback table).
// Kept here as a fixture constant so alias tests don't hand-roll the same array.
const GOOSE_ALIASES = [
  ["none", "off"],
  ["disabled", "off"],
  ["med", "medium"],
  ["xhigh", "max"],
];

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
    acceptedEffortValues: null,
    effortAliases: null,
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
      acceptedEffortValues: ["off", "low", "medium", "high", "max"],
    }),
    scope: "global",
  });

  // With acceptedEffortValues, optionSource is "harnessNative" (single metadata authority).
  assert.equal(field(model, "effort").optionSource, "harnessNative");
  // Persistence key is the native key, not the legacy key.
  assert.deepEqual(field(model, "effort").currentPersistence, {
    kind: "envVar",
    key: "GOOSE_THINKING_EFFORT",
  });
  assert.deepEqual(field(model, "effort").targetApplication, {
    kind: "envVar",
    key: "GOOSE_THINKING_EFFORT",
  });
});

test("Goose without acceptedEffortValues falls back to legacyProviderModelCatalog", () => {
  // When a Goose catalog entry predates acceptedEffortValues (null), the field
  // falls back to legacyProviderModelCatalog — existing behavior preserved.
  const model = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("goose", {
      modelEnvVar: "GOOSE_MODEL",
      providerEnvVar: "GOOSE_PROVIDER",
      thinkingEnvVar: "GOOSE_THINKING_EFFORT",
      acceptedEffortValues: null, // explicit null → legacy path
    }),
    scope: "global",
  });

  assert.equal(
    field(model, "effort").optionSource,
    "legacyProviderModelCatalog",
  );
  // Native key is still used as persistence key (v3 Phase 2 always uses native key).
  assert.deepEqual(field(model, "effort").currentPersistence, {
    kind: "envVar",
    key: "GOOSE_THINKING_EFFORT",
  });
});

test("Goose reads value from native key, falls back to legacy pre-migration save", () => {
  // With both keys present, native wins (global scope — native-wins applies at all tiers).
  const modelBothKeys = deriveAgentConfigFieldModel({
    config: {
      ...config,
      env_vars: {
        GOOSE_THINKING_EFFORT: "medium",
        BUZZ_AGENT_THINKING_EFFORT: "high",
      },
    },
    runtime: runtime("goose", {
      thinkingEnvVar: "GOOSE_THINKING_EFFORT",
      acceptedEffortValues: ["off", "low", "medium", "high", "max"],
    }),
    scope: "global",
  });
  assert.equal(field(modelBothKeys, "effort").value, "medium");

  // With only legacy key, read-old fallback kicks in at record/persona tier.
  // At global scope, legacy is excluded (Delta 4 / effort_tier_alias(global_tier=true)).
  const modelLegacyOnly = deriveAgentConfigFieldModel({
    config: {
      ...config,
      env_vars: { BUZZ_AGENT_THINKING_EFFORT: "high" },
    },
    runtime: runtime("goose", {
      thinkingEnvVar: "GOOSE_THINKING_EFFORT",
      acceptedEffortValues: ["off", "low", "medium", "high", "max"],
    }),
    scope: "instance",
  });
  assert.equal(field(modelLegacyOnly, "effort").value, "high");
});

test("Claude models effort as harnessNative via CLAUDE_CODE_EFFORT_LEVEL", () => {
  const CLAUDE_ACCEPTED = ["low", "medium", "high", "xhigh", "max"];
  const model = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("claude", {
      thinkingEnvVar: "CLAUDE_CODE_EFFORT_LEVEL",
      acceptedEffortValues: CLAUDE_ACCEPTED,
      effortAliases: [],
    }),
    scope: "global",
  });

  assert.deepEqual(
    model.fields.map((item) => item.kind),
    ["model", "effort"],
  );
  const effortField = field(model, "effort");
  assert.equal(effortField.render, "control");
  assert.equal(effortField.optionSource, "harnessNative");
  assert.deepEqual(effortField.currentPersistence, {
    kind: "envVar",
    key: "CLAUDE_CODE_EFFORT_LEVEL",
  });
  assert.deepEqual(effortField.targetApplication, {
    kind: "envVar",
    key: "CLAUDE_CODE_EFFORT_LEVEL",
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

test("structuredEnvKeys_per_agent_goose_includes_effort_and_numeric_keys", () => {
  // Per-agent Goose with acceptedEffortValues now renders an effort control —
  // the native key (GOOSE_THINKING_EFFORT) must appear in the structured set.
  // The legacy key (BUZZ_AGENT_THINKING_EFFORT) must NOT appear — it has no
  // editor on this surface and must stay visible as a generic env row.
  const gooseModel = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("goose", {
      thinkingEnvVar: "GOOSE_THINKING_EFFORT",
      acceptedEffortValues: ["off", "low", "medium", "high", "max"],
      maxTokensEnvVar: "GOOSE_MAX_TOKENS",
      contextLimitEnvVar: "GOOSE_CONTEXT_LIMIT",
    }),
    scope: "definition",
  });

  const renderedDescriptors = gooseModel.fields.filter(
    (f) => f.render === "control",
  );
  const keys = structuredEnvKeys(renderedDescriptors);

  assert.ok(
    keys.includes("GOOSE_THINKING_EFFORT"),
    "Goose native effort key must be hidden (control renders)",
  );
  assert.equal(
    keys.includes("BUZZ_AGENT_THINKING_EFFORT"),
    false,
    "legacy key must NOT be hidden — no editor owns it for Goose",
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
    runtime: runtime("claude", {
      thinkingEnvVar: "CLAUDE_CODE_EFFORT_LEVEL",
      acceptedEffortValues: ["low", "medium", "high", "xhigh", "max"],
      effortAliases: [],
    }),
    scope: "global",
  });

  const allDescriptors = claudeModel.fields; // includes harnessNative effort
  const keys = structuredEnvKeys(allDescriptors);

  // Claude's harnessNative effort has currentPersistence.kind === "envVar"
  // with key "CLAUDE_CODE_EFFORT_LEVEL" — it is a hidden key.
  assert.deepEqual(
    keys,
    ["CLAUDE_CODE_EFFORT_LEVEL"],
    "Claude effort env key must appear as a hidden structured key",
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

test("unconsumed_legacy_effort_row_stays_visible_for_goose", () => {
  // An invalid-for-Goose legacy value in a Goose record (BUZZ_AGENT_THINKING_EFFORT=minimal)
  // must remain visible as a generic advanced-env row — it is not consumed as structured
  // effort and must not be hidden. Only the native key (GOOSE_THINKING_EFFORT) is in
  // structuredEnvKeys for Goose. (plan v3 pass-3 ★ pin: unconsumed legacy rows stay visible.)
  // Uses `minimal` — invalid for Goose's canonical vocabulary — to pin the normalization
  // rejection path: the value is present but normalization returns null → not consumed.
  const gooseModel = deriveAgentConfigFieldModel({
    config: {
      ...config,
      env_vars: { BUZZ_AGENT_THINKING_EFFORT: "minimal" },
    },
    runtime: runtime("goose", {
      thinkingEnvVar: "GOOSE_THINKING_EFFORT",
      acceptedEffortValues: ["off", "low", "medium", "high", "max"],
    }),
    scope: "definition",
  });
  const renderedDescriptors = gooseModel.fields.filter(
    (f) => f.render === "control",
  );
  const keys = structuredEnvKeys(renderedDescriptors);
  // Legacy key is NOT in structured keys for Goose → stays visible as advanced row.
  assert.equal(
    keys.includes("BUZZ_AGENT_THINKING_EFFORT"),
    false,
    "legacy key must not be hidden for Goose — no structured control owns it",
  );
  // Invalid legacy value is not consumed → effort control shows empty.
  assert.equal(
    field(gooseModel, "effort").value,
    null,
    "invalid legacy `minimal` must not be consumed — effort control is empty",
  );
});

test("goose_legacy_only_persona_scope_renders_canonical_value", () => {
  // Legacy-only Goose persona (scope="definition") — the legacy fallback must
  // apply at this scope and render the normalized canonical value.
  // Exercises the legacy read path for the persona editor surface specifically.
  const model = deriveAgentConfigFieldModel({
    config: {
      ...config,
      env_vars: { BUZZ_AGENT_THINKING_EFFORT: "high" },
    },
    runtime: runtime("goose", {
      thinkingEnvVar: "GOOSE_THINKING_EFFORT",
      acceptedEffortValues: ["off", "low", "medium", "high", "max"],
    }),
    scope: "definition",
  });
  assert.equal(
    field(model, "effort").value,
    "high",
    "valid legacy value at persona scope must be shown as canonical effort value",
  );
  assert.equal(
    field(model, "effort").legacyConsumedKey,
    "BUZZ_AGENT_THINKING_EFFORT",
    "legacyConsumedKey set at persona scope so the duplicate env row is hidden",
  );
});

test("goose_alias_none_normalizes_to_off_in_display_value", () => {
  // Pre-migration Goose record with BUZZ_AGENT_THINKING_EFFORT=none.
  // The display value must be normalized to "off" (canonical) not "none" (alias).
  // Uses scope: "instance" — legacy is consumed only at record/persona tiers.
  const model = deriveAgentConfigFieldModel({
    config: {
      ...config,
      env_vars: { BUZZ_AGENT_THINKING_EFFORT: "none" },
    },
    runtime: runtime("goose", {
      thinkingEnvVar: "GOOSE_THINKING_EFFORT",
      acceptedEffortValues: ["off", "low", "medium", "high", "max"],
      effortAliases: GOOSE_ALIASES,
    }),
    scope: "instance",
  });
  assert.equal(
    field(model, "effort").value,
    "off",
    "none (Buzz alias) must normalize to off (Goose canonical) for display",
  );
});

test("goose_alias_xhigh_normalizes_to_max_in_display_value", () => {
  // Pre-migration Goose record with BUZZ_AGENT_THINKING_EFFORT=xhigh.
  // The display value must be normalized to "max" (canonical).
  // Uses scope: "instance" — legacy is consumed only at record/persona tiers.
  const model = deriveAgentConfigFieldModel({
    config: {
      ...config,
      env_vars: { BUZZ_AGENT_THINKING_EFFORT: "xhigh" },
    },
    runtime: runtime("goose", {
      thinkingEnvVar: "GOOSE_THINKING_EFFORT",
      acceptedEffortValues: ["off", "low", "medium", "high", "max"],
      effortAliases: GOOSE_ALIASES,
    }),
    scope: "instance",
  });
  assert.equal(
    field(model, "effort").value,
    "max",
    "xhigh (Buzz alias) must normalize to max (Goose canonical) for display",
  );
});

test("goose_invalid_legacy_effort_shows_null_value_not_alias", () => {
  // BUZZ_AGENT_THINKING_EFFORT=minimal is invalid for Goose (no alias). Display
  // value must be null (skip-as-absent), not the raw invalid string.
  // Uses scope: "instance" — legacy is consumed only at record/persona tiers.
  const model = deriveAgentConfigFieldModel({
    config: {
      ...config,
      env_vars: { BUZZ_AGENT_THINKING_EFFORT: "minimal" },
    },
    runtime: runtime("goose", {
      thinkingEnvVar: "GOOSE_THINKING_EFFORT",
      acceptedEffortValues: ["off", "low", "medium", "high", "max"],
    }),
    scope: "instance",
  });
  assert.equal(
    field(model, "effort").value,
    null,
    "minimal (invalid for Goose) must yield null display value",
  );
});

test("goose_legacy_consumed_as_effort_reports_legacyConsumedKey", () => {
  // When a legacy value is consumed as Goose effort at record/persona tier
  // (scope "instance"), the descriptor must set legacyConsumedKey so callers
  // can suppress the dup row. Legacy is NOT consumed at global scope.
  const model = deriveAgentConfigFieldModel({
    config: {
      ...config,
      env_vars: { BUZZ_AGENT_THINKING_EFFORT: "high" },
    },
    runtime: runtime("goose", {
      thinkingEnvVar: "GOOSE_THINKING_EFFORT",
      acceptedEffortValues: ["off", "low", "medium", "high", "max"],
    }),
    scope: "instance",
  });
  assert.equal(
    field(model, "effort").legacyConsumedKey,
    "BUZZ_AGENT_THINKING_EFFORT",
    "legacyConsumedKey must be set when legacy key value is shown via fallback",
  );
});

test("goose_global_scope_legacy_not_consumed_effort_empty_and_row_visible", () => {
  // Plan v3 Delta 4 pin: at global scope, BUZZ_AGENT_THINKING_EFFORT is
  // buzz-agent's native key and must NEVER be consumed as Goose effort.
  // The effort control must show empty; the legacy key must NOT appear in
  // structuredEnvKeys (so it remains a visible advanced row).
  const model = deriveAgentConfigFieldModel({
    config: {
      ...config,
      env_vars: { BUZZ_AGENT_THINKING_EFFORT: "high" },
    },
    runtime: runtime("goose", {
      thinkingEnvVar: "GOOSE_THINKING_EFFORT",
      acceptedEffortValues: ["off", "low", "medium", "high", "max"],
    }),
    scope: "global",
  });
  assert.equal(
    field(model, "effort").value,
    null,
    "Goose effort control must be empty at global scope with only legacy key",
  );
  assert.equal(
    field(model, "effort").legacyConsumedKey,
    undefined,
    "legacyConsumedKey must be undefined at global scope — legacy stays a visible row",
  );
  // Legacy key must NOT be hidden (not in structuredEnvKeys).
  const renderedDescriptors = model.fields.filter(
    (f) => f.render === "control",
  );
  const keys = structuredEnvKeys(renderedDescriptors);
  assert.equal(
    keys.includes("BUZZ_AGENT_THINKING_EFFORT"),
    false,
    "legacy key must remain a visible advanced row at global scope for Goose",
  );
});

test("goose_native_key_set_does_not_report_legacyConsumedKey", () => {
  // When native key is present, legacyConsumedKey must be undefined (no dup row).
  const model = deriveAgentConfigFieldModel({
    config: {
      ...config,
      env_vars: { GOOSE_THINKING_EFFORT: "medium" },
    },
    runtime: runtime("goose", {
      thinkingEnvVar: "GOOSE_THINKING_EFFORT",
      acceptedEffortValues: ["off", "low", "medium", "high", "max"],
    }),
    scope: "global",
  });
  assert.equal(
    field(model, "effort").legacyConsumedKey,
    undefined,
    "legacyConsumedKey must be undefined when native key is set",
  );
});
