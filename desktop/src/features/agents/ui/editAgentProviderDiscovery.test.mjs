import assert from "node:assert/strict";
import test from "node:test";

import {
  runtimeSupportsLlmProviderSelection,
  getPersonaProviderOptions,
} from "./personaDialogPickers.tsx";

// ── LLM provider field visibility (EditAgentDialog) ─────────────────────────
//
// The edit dialog shows the provider picker when the current runtime supports
// LLM provider selection. Changing the provider in that picker re-fires
// usePersonaModelDiscovery (keyed on provider), so the model dropdown updates
// without saving. These tests guard the visibility predicate.

test("editAgent_providerFieldVisible_forBuzzAgent", () => {
  assert.equal(
    runtimeSupportsLlmProviderSelection("buzz-agent"),
    true,
    "buzz-agent runtime must expose the provider picker",
  );
});

test("editAgent_providerFieldVisible_forGoose", () => {
  assert.equal(
    runtimeSupportsLlmProviderSelection("goose"),
    true,
    "goose runtime must expose the provider picker",
  );
});

test("editAgent_providerFieldHidden_forClaude", () => {
  assert.equal(
    runtimeSupportsLlmProviderSelection("claude"),
    false,
    "claude runtime locks the provider; picker must be hidden",
  );
});

test("editAgent_providerFieldHidden_forBlankRuntime", () => {
  assert.equal(
    runtimeSupportsLlmProviderSelection(""),
    false,
    "blank runtime (catalog miss) must not show the provider picker",
  );
});

// ── Provider dropdown options for EditAgentProviderField ────────────────────
//
// The provider dropdown must always contain the well-known providers
// (databricks, databricks_v2, anthropic, openai, openai-compat) plus a
// default-provider fallback entry so users can clear a saved provider.

test("editAgent_providerOptions_includesDatabricksProviders", () => {
  const options = getPersonaProviderOptions("", "buzz-agent");
  const ids = options.map((o) => o.id);
  assert.ok(ids.includes("databricks"), "databricks must be a provider option");
  assert.ok(
    ids.includes("databricks_v2"),
    "databricks_v2 must be a provider option",
  );
});

test("editAgent_providerOptions_includesDefaultEntry", () => {
  const options = getPersonaProviderOptions("", "buzz-agent");
  // The first entry is the default (empty id) — clearing back to runtime default.
  assert.equal(
    options[0].id,
    "",
    "first provider option must be the default (empty id)",
  );
});

test("editAgent_providerOptions_includesCurrentIfCustom", () => {
  const options = getPersonaProviderOptions("my-custom-llm", "buzz-agent");
  const ids = options.map((o) => o.id);
  assert.ok(
    ids.includes("my-custom-llm"),
    "a currently-saved custom provider must appear in the dropdown",
  );
});
