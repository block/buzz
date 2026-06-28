import assert from "node:assert/strict";
import test from "node:test";

import {
  formatModelDiscoveryErrorStatus,
  formatModelDiscoveryFallbackStatus,
} from "./personaModelDiscoveryStatus.ts";

test("model discovery status names missing Anthropic credentials", () => {
  const status = formatModelDiscoveryErrorStatus(
    new Error("config: ANTHROPIC_API_KEY required"),
    "anthropic",
  );

  assert.equal(status.tone, "warning");
  assert.match(status.message, /ANTHROPIC_API_KEY/);
  assert.match(status.message, /Anthropic models/);
});

test("model discovery status names missing OpenAI-compatible credentials", () => {
  const status = formatModelDiscoveryErrorStatus(
    new Error("config: OPENAI_COMPAT_API_KEY required"),
    "openai-compat",
  );

  assert.equal(status.tone, "warning");
  assert.match(status.message, /OPENAI_COMPAT_API_KEY/);
  assert.match(status.message, /OpenAI models/);
});

test("model discovery status names missing Databricks defaults", () => {
  const status = formatModelDiscoveryErrorStatus(
    new Error("config: DATABRICKS_HOST required"),
    "databricks",
  );

  assert.equal(status.tone, "warning");
  assert.match(status.message, /DATABRICKS_HOST/);
  assert.match(status.message, /DATABRICKS_MODEL/);
});

test("model discovery status explains a runtime without live model listing", () => {
  const status = formatModelDiscoveryFallbackStatus({
    provider: "databricks",
    response: {
      agentName: "buzz-agent",
      agentVersion: "0.0.0",
      models: [],
      agentDefaultModel: null,
      selectedModel: null,
      supportsSwitching: false,
    },
  });

  assert.equal(status?.tone, "muted");
  assert.match(status?.message ?? "", /Databricks/);
  assert.match(status?.message ?? "", /does not expose a live model list/);
});

test("model discovery status stays quiet when live models are available", () => {
  const status = formatModelDiscoveryFallbackStatus({
    provider: "databricks",
    response: {
      agentName: "buzz-agent",
      agentVersion: "0.0.0",
      models: [{ id: "goose-gpt-5-5", name: "GPT 5.5", description: null }],
      agentDefaultModel: null,
      selectedModel: null,
      supportsSwitching: true,
    },
  });

  assert.equal(status, null);
});
