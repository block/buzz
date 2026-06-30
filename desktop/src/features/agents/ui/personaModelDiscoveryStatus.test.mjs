import assert from "node:assert/strict";
import test from "node:test";

import { formatModelDiscoveryErrorStatus } from "./personaModelDiscoveryStatus.ts";

test("model discovery status names missing Anthropic credentials", () => {
  const status = formatModelDiscoveryErrorStatus(
    new Error("config: ANTHROPIC_API_KEY required"),
    "anthropic",
  );

  assert.equal(status?.tone, "warning");
  assert.match(status?.message ?? "", /Anthropic API key/);
  assert.match(status?.message ?? "", /Anthropic models/);
});

test("model discovery status names missing OpenAI-compatible credentials", () => {
  const status = formatModelDiscoveryErrorStatus(
    new Error("config: OPENAI_COMPAT_API_KEY required"),
    "openai-compat",
  );

  assert.equal(status?.tone, "warning");
  assert.match(status?.message ?? "", /OpenAI API key/);
  assert.match(status?.message ?? "", /OpenAI models/);
});

test("model discovery status stays quiet for missing Databricks defaults", () => {
  const status = formatModelDiscoveryErrorStatus(
    new Error("config: DATABRICKS_HOST required"),
    "databricks",
  );

  assert.equal(status, null);
});
