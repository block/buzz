import assert from "node:assert/strict";
import test from "node:test";

import {
  agentAiConfigurationModeSatisfied,
  agentAiConfigurationPairForMode,
  initialAgentAiConfigurationMode,
  modelFieldStatus,
  typedModelCatalogError,
} from "./agentAiConfigurationPolicy.ts";

test("existing one-sided and complete overrides open in Customize", () => {
  assert.equal(
    initialAgentAiConfigurationMode({ provider: "anthropic" }),
    "custom",
  );
  assert.equal(
    initialAgentAiConfigurationMode({ model: "claude-opus" }),
    "custom",
  );
  assert.equal(
    initialAgentAiConfigurationMode({
      provider: "anthropic",
      model: "claude-opus",
    }),
    "custom",
  );
  assert.equal(initialAgentAiConfigurationMode({}), "defaults");
});

test("Customize requires a complete explicit pair", () => {
  assert.equal(
    agentAiConfigurationModeSatisfied("custom", {
      provider: "anthropic",
      model: "",
    }),
    false,
  );
  assert.equal(
    agentAiConfigurationModeSatisfied("custom", {
      provider: "",
      model: "claude-opus",
    }),
    false,
  );
  assert.equal(
    agentAiConfigurationModeSatisfied("custom", {
      provider: "anthropic",
      model: "claude-opus",
    }),
    true,
  );
});

test("Codex/Claude Customize needs only a model, not the hidden provider", () => {
  // needsProviderSelection=false → the intentionally hidden provider must not
  // gate Save (the create/edit "Save stays disabled" regression).
  assert.equal(
    agentAiConfigurationModeSatisfied(
      "custom",
      { provider: "", model: "gpt-5-codex" },
      false,
    ),
    true,
  );
  // Still needs a model even when the provider is hidden.
  assert.equal(
    agentAiConfigurationModeSatisfied(
      "custom",
      { provider: "", model: "" },
      false,
    ),
    false,
  );
});

test("Buzz Agent/Goose Customize still requires both provider and model", () => {
  assert.equal(
    agentAiConfigurationModeSatisfied(
      "custom",
      { provider: "", model: "llama" },
      true,
    ),
    false,
  );
  assert.equal(
    agentAiConfigurationModeSatisfied(
      "custom",
      { provider: "databricks_v2", model: "llama" },
      true,
    ),
    true,
  );
});

test("runtime-less editable definition still requires the visible provider", () => {
  // A legacy/builtin definition with no runtime but a saved model exposes the
  // provider picker (runtimeCanChooseLlmProvider === true), so the dialog passes
  // needsProviderSelection=true here. An empty provider must NOT satisfy the
  // pair — otherwise Save persists `provider: undefined` despite the visible
  // picker (wesbillman's blocking review point).
  assert.equal(
    agentAiConfigurationModeSatisfied(
      "custom",
      { provider: "", model: "claude-opus-4-5" },
      true,
    ),
    false,
  );
  assert.equal(
    agentAiConfigurationModeSatisfied(
      "custom",
      { provider: "anthropic", model: "claude-opus-4-5" },
      true,
    ),
    true,
  );
});

test("Defaults clears provider and model together", () => {
  assert.deepEqual(
    agentAiConfigurationPairForMode({
      current: { provider: "anthropic", model: "claude-opus" },
      inherited: { provider: "databricks_v2", model: "llama" },
      mode: "defaults",
    }),
    { provider: "", model: "" },
  );
});

test("entering Customize pins only the harness model without a provider picker", () => {
  for (const model of ["claude-opus", "gpt-5.2-codex"]) {
    assert.deepEqual(
      agentAiConfigurationPairForMode({
        current: { provider: "", model: "" },
        inherited: { provider: "databricks_v2", model },
        mode: "custom",
        needsProviderSelection: false,
      }),
      { provider: "", model },
    );
  }
});

test("entering Customize pins unresolved fields from the inherited pair", () => {
  assert.deepEqual(
    agentAiConfigurationPairForMode({
      current: { provider: "anthropic", model: "" },
      inherited: { provider: "databricks_v2", model: "llama" },
      mode: "custom",
    }),
    { provider: "anthropic", model: "llama" },
  );
});

const CATALOG = [{ id: "" }, { id: "fable" }, { id: "opus" }];

test("a typed model the harness offers submits", () => {
  assert.equal(
    typedModelCatalogError({
      catalog: CATALOG,
      isTypedEntry: true,
      model: "fable",
    }),
    null,
  );
});

test("a typed model the harness never offered is blocked, and named", () => {
  // The runtime matches byte-exactly, so this near miss would otherwise
  // resolve to the adapter's default and run the wrong model silently.
  const error = typedModelCatalogError({
    catalog: CATALOG,
    isTypedEntry: true,
    model: "claude-fable-5",
  });
  assert.match(error, /claude-fable-5/);
  // The message names what the harness DOES offer — the empty-id "Default
  // model" row is not a model, so it is not offered as one.
  assert.match(error, /fable, opus/);
});

test("no catalog means no gate — free text still works for a BYOH harness", () => {
  for (const catalog of [null, [], [{ id: "" }]]) {
    assert.equal(
      typedModelCatalogError({
        catalog,
        isTypedEntry: true,
        model: "some-private-model",
      }),
      null,
    );
  }
});

test("the field wrapper blocks and speaks with one voice", () => {
  const discovery = { message: "Loading models...", tone: "muted" };
  // A miss owns the status line: the discovery message would otherwise explain
  // everything except why Save is dead.
  const blocked = modelFieldStatus({
    catalog: CATALOG,
    discoveryStatus: discovery,
    isTypedEntry: true,
    model: "claude-fable-5",
  });
  assert.equal(blocked.blocked, true);
  assert.match(blocked.status.message, /claude-fable-5/);
  assert.equal(blocked.status.tone, "warning");

  // Otherwise the discovery status passes through untouched.
  assert.deepEqual(
    modelFieldStatus({
      catalog: CATALOG,
      discoveryStatus: discovery,
      isTypedEntry: true,
      model: "fable",
    }),
    { blocked: false, status: discovery },
  );
});

test("an already-saved off-catalog model does not block an unrelated edit", () => {
  // Opening a dialog to rename an agent whose saved model predates the current
  // catalog must not kill Save on a field the user never touched. Callers pass
  // "is typing right now" (isCustomModelEditing), never "is outside the
  // catalog" — the latter is exactly the state every such agent opens in.
  assert.deepEqual(
    modelFieldStatus({
      catalog: CATALOG,
      discoveryStatus: null,
      isTypedEntry: false,
      model: "retired-model-id",
    }),
    { blocked: false, status: null },
  );
});

test("a long catalog is truncated rather than dumped into the message", () => {
  const catalog = Array.from({ length: 15 }, (_, index) => ({
    id: `m${index}`,
  }));
  const { status } = modelFieldStatus({
    catalog,
    discoveryStatus: null,
    isTypedEntry: true,
    model: "nope",
  });
  assert.match(status.message, /m0, m1, .*m11, and 3 more\.$/);
  assert.doesNotMatch(status.message, /m12/);
});
