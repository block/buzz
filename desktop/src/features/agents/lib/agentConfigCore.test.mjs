import assert from "node:assert/strict";
import test from "node:test";

import {
  deriveAgentConfigFieldModel,
  deriveEffortEnvDescriptor,
  deriveNumericDescriptors,
  implicitEffortProvider,
  readEffortEnvValue,
  readStructuredJsonEnvValue,
  structuredEnvKeys,
  updateEffortEnvValue,
  updateStructuredJsonEnvValue,
} from "./agentConfigCore.ts";
import { NUMERIC_KIND_MIN } from "../ui/buzzAgentModelTuningFields.tsx";

const config = {
  env_vars: { BUZZ_AGENT_THINKING_EFFORT: "high" },
  model: "test-model",
  preferred_runtime: null,
  provider: "anthropic",
};

/**
 * Effort env targets as declared by the real Rust catalog
 * (`src-tauri/src/managed_agents/discovery/known_runtimes.rs`). The fixture
 * applies them by runtime id so a test never asserts against a metadata shape
 * the shipped catalog does not produce.
 */
const CATALOG_EFFORT_TARGETS = {
  "buzz-agent": { thinkingEnvVar: "BUZZ_AGENT_THINKING_EFFORT" },
  goose: { thinkingEnvVar: "GOOSE_THINKING_EFFORT" },
  claude: { thinkingEnvVar: "CLAUDE_CODE_EFFORT_LEVEL" },
  codex: {
    thinkingConfigJsonEnvVar: "CODEX_CONFIG",
    thinkingConfigJsonKey: "model_reasoning_effort",
  },
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
    thinkingConfigJsonEnvVar: null,
    thinkingConfigJsonKey: null,
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
    ...CATALOG_EFFORT_TARGETS[id],
    ...metadata,
  };
}

function field(model, kind) {
  return model.fields.find((candidate) => candidate.kind === kind);
}

test("structured JSON effort preserves unrelated runtime configuration", () => {
  const descriptor = deriveEffortEnvDescriptor(runtime("codex"));
  assert.ok(descriptor);

  const initial = {
    CODEX_CONFIG: JSON.stringify({
      other_setting: "keep",
      model_reasoning_effort: "medium",
    }),
  };
  assert.equal(
    readStructuredJsonEnvValue(initial, descriptor.currentPersistence),
    "medium",
  );
  const updated = updateStructuredJsonEnvValue(
    initial,
    descriptor.currentPersistence,
    "high",
  );
  assert.deepEqual(JSON.parse(updated.CODEX_CONFIG), {
    other_setting: "keep",
    model_reasoning_effort: "high",
  });
  const cleared = updateStructuredJsonEnvValue(
    updated,
    descriptor.currentPersistence,
    "",
  );
  assert.deepEqual(JSON.parse(cleared.CODEX_CONFIG), { other_setting: "keep" });
});

test("structured JSON effort replaces malformed JSON only when edited", () => {
  const descriptor = deriveEffortEnvDescriptor(runtime("codex"));
  assert.ok(descriptor);
  const initial = { CODEX_CONFIG: "not-json" };
  assert.equal(
    readStructuredJsonEnvValue(initial, descriptor.currentPersistence),
    "",
  );
  const updated = updateStructuredJsonEnvValue(
    initial,
    descriptor.currentPersistence,
    "high",
  );
  assert.deepEqual(JSON.parse(updated.CODEX_CONFIG), {
    model_reasoning_effort: "high",
  });
});

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
  // The value is stored exactly where the harness reads it: an earlier
  // revision pinned currentPersistence to BUZZ_AGENT_THINKING_EFFORT for every
  // runtime, so the Goose control wrote a key Goose ignores.
  assert.deepEqual(field(model, "effort").currentPersistence, {
    kind: "envVar",
    key: "GOOSE_THINKING_EFFORT",
  });
  assert.deepEqual(field(model, "effort").targetApplication, {
    kind: "envVar",
    key: "GOOSE_THINKING_EFFORT",
  });
});

test("Claude Code effort renders against its own plain env var", () => {
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
    key: "CLAUDE_CODE_EFFORT_LEVEL",
  });
  assert.deepEqual(field(model, "effort").targetApplication, {
    kind: "envVar",
    key: "CLAUDE_CODE_EFFORT_LEVEL",
  });
  assert.deepEqual(model.omissions, []);
});

test("Codex effort renders against its structured JSON env target", () => {
  const model = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("codex"),
    scope: "global",
  });

  assert.deepEqual(
    model.fields.map((item) => item.kind),
    ["model", "effort"],
  );
  assert.equal(field(model, "effort").render, "control");
  assert.deepEqual(field(model, "effort").currentPersistence, {
    kind: "structuredJsonEnv",
    envKey: "CODEX_CONFIG",
    jsonKey: "model_reasoning_effort",
  });
  assert.deepEqual(
    field(model, "effort").targetApplication,
    field(model, "effort").currentPersistence,
    "value must be stored exactly where the harness reads it",
  );
});

test("a harness with no effort env target omits effort with a named reason", () => {
  const model = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("openclaw", {
      thinkingEnvVar: null,
      thinkingConfigJsonEnvVar: null,
      thinkingConfigJsonKey: null,
    }),
    scope: "global",
  });

  assert.equal(field(model, "effort"), undefined);
  assert.deepEqual(model.omissions, [
    { kind: "effort", reason: "unsupportedByHarness" },
  ]);
});

test("effort value is read from the shape the harness uses", () => {
  const claude = deriveAgentConfigFieldModel({
    config: {
      env_vars: { CLAUDE_CODE_EFFORT_LEVEL: "xhigh" },
      model: null,
      preferred_runtime: null,
      provider: null,
    },
    runtime: runtime("claude"),
    scope: "instance",
  });
  assert.equal(field(claude, "effort").value, "xhigh");

  const codex = deriveAgentConfigFieldModel({
    config: {
      env_vars: {
        CODEX_CONFIG: JSON.stringify({ model_reasoning_effort: "low" }),
      },
      model: null,
      preferred_runtime: null,
      provider: null,
    },
    runtime: runtime("codex"),
    scope: "instance",
  });
  assert.equal(field(codex, "effort").value, "low");

  // A stale BUZZ_AGENT_THINKING_EFFORT must not be read as a Claude value —
  // that was the defect this contract closes.
  const stale = deriveAgentConfigFieldModel({
    config: {
      env_vars: { BUZZ_AGENT_THINKING_EFFORT: "high" },
      model: null,
      preferred_runtime: null,
      provider: null,
    },
    runtime: runtime("claude"),
    scope: "instance",
  });
  assert.equal(stale.fields.find((f) => f.kind === "effort").value, null);
});

test("effort writes land on the harness's own key, not buzz-agent's", () => {
  for (const [id, expectedKey] of [
    ["goose", "GOOSE_THINKING_EFFORT"],
    ["claude", "CLAUDE_CODE_EFFORT_LEVEL"],
  ]) {
    const descriptor = deriveEffortEnvDescriptor(runtime(id));
    assert.ok(descriptor, `${id} must have an effort env target`);
    const written = updateEffortEnvValue(
      {},
      descriptor.currentPersistence,
      "high",
    );
    assert.deepEqual(written, { [expectedKey]: "high" });
    assert.equal(
      Object.hasOwn(written, "BUZZ_AGENT_THINKING_EFFORT"),
      false,
      `${id} must not write buzz-agent's key`,
    );
    assert.equal(
      readEffortEnvValue(written, descriptor.currentPersistence),
      "high",
    );
    assert.deepEqual(
      updateEffortEnvValue(written, descriptor.currentPersistence, ""),
      {},
      "clearing removes the key",
    );
  }
});

test("effort helpers are no-ops without a target", () => {
  assert.equal(readEffortEnvValue({ A: "b" }, undefined), "");
  const cleared = updateEffortEnvValue({ A: "b" }, undefined, "high");
  assert.deepEqual(cleared, { A: "b" });
});

test("deriveEffortEnvDescriptor returns undefined without a catalog target", () => {
  assert.equal(deriveEffortEnvDescriptor(undefined), undefined);
  assert.equal(
    deriveEffortEnvDescriptor(
      runtime("openclaw", {
        thinkingEnvVar: null,
        thinkingConfigJsonEnvVar: null,
        thinkingConfigJsonKey: null,
      }),
    ),
    undefined,
  );
});

test("deriveEffortEnvDescriptor prefers the structured JSON target", () => {
  // A harness declaring both must apply through the structured target: the
  // plain variable is not read by the adapter that owns the JSON blob.
  const descriptor = deriveEffortEnvDescriptor(
    runtime("codex", { thinkingEnvVar: "IGNORED_PLAIN_VAR" }),
  );
  assert.equal(descriptor.currentPersistence.kind, "structuredJsonEnv");
});

test("implicitEffortProvider names the provider a locked harness implies", () => {
  assert.equal(implicitEffortProvider("claude"), "anthropic");
  assert.equal(implicitEffortProvider("codex"), "openai");
  assert.equal(implicitEffortProvider("goose"), "");
});

test("a provider-locked harness offers only levels that harness accepts", () => {
  // Claude Code's CLI enum is ["low","medium","high","xhigh","max"];
  // CLAUDE_CODE_EFFORT_LEVEL=none resolves to "unset", so offering "none" or
  // "minimal" would let the user pick a value the harness silently drops.
  const claude = deriveEffortEnvDescriptor(runtime("claude"));
  assert.deepEqual(
    [...claude.values],
    ["low", "medium", "high", "xhigh", "max"],
  );

  // Goose's provider is user-selectable, so the accepted set is not knowable
  // from the harness alone — the full list stays, narrowed per provider on the
  // surfaces that know the provider.
  const goose = deriveEffortEnvDescriptor(runtime("goose"));
  assert.ok(goose.values.includes("none"));
  assert.ok(goose.values.includes("minimal"));
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

test("structuredEnvKeys_per_agent_goose_hides_its_own_effort_key", () => {
  // Per-agent Goose now renders an effort control against Goose's own key, so
  // that key IS owned by a first-class editor and must be hidden from the
  // generic rows. Nothing hides buzz-agent's key here: a stale value from the
  // previous modeling stays visible and deletable as a generic row.
  const gooseModel = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("goose", {
      maxTokensEnvVar: "GOOSE_MAX_TOKENS",
      contextLimitEnvVar: "GOOSE_CONTEXT_LIMIT",
    }),
    scope: "definition",
  });

  const keys = structuredEnvKeys(
    gooseModel.fields.filter((f) => f.render === "control"),
  );

  assert.ok(
    keys.includes("GOOSE_THINKING_EFFORT"),
    "Goose's own effort key must be hidden (its control renders)",
  );
  assert.equal(
    keys.includes("BUZZ_AGENT_THINKING_EFFORT"),
    false,
    "a stale buzz-agent key must stay a visible generic row — no editor owns it",
  );
  assert.ok(keys.includes("GOOSE_MAX_TOKENS"), "maxTokens key must be present");
  assert.ok(
    keys.includes("GOOSE_CONTEXT_LIMIT"),
    "contextLimit key must be present",
  );
});

test("structuredEnvKeys_codex_hides_the_structured_json_env_key", () => {
  const codexModel = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("codex"),
    scope: "definition",
  });

  const keys = structuredEnvKeys(
    codexModel.fields.filter((f) => f.render === "control"),
  );
  assert.deepEqual(keys, ["CODEX_CONFIG"]);
});

test("structuredEnvKeys_emits_nothing_when_no_effort_target_exists", () => {
  const model = deriveAgentConfigFieldModel({
    config,
    runtime: runtime("openclaw", {
      thinkingEnvVar: null,
      thinkingConfigJsonEnvVar: null,
      thinkingConfigJsonKey: null,
    }),
    scope: "global",
  });

  assert.equal(
    structuredEnvKeys(model.fields).length,
    0,
    "model descriptor alone must not contribute hidden keys",
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
