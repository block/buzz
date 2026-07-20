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

test("model discovery status names missing Gemini credentials", () => {
  const status = formatModelDiscoveryErrorStatus(
    new Error("config: GEMINI_API_KEY required"),
    "gemini",
  );

  assert.equal(status?.tone, "warning");
  assert.match(status?.message ?? "", /Gemini API key/);
  assert.match(status?.message ?? "", /Gemini models/);
});

test("model discovery status labels Gemini in the generic fallback", () => {
  const status = formatModelDiscoveryErrorStatus(
    new Error("gemini discovery hit an unexpected transport error"),
    "gemini",
  );

  assert.equal(status?.tone, "warning");
  assert.match(status?.message ?? "", /Gemini/);
});

test("Buzz shared compute names the empty state and next action", () => {
  const status = formatModelDiscoveryErrorStatus(
    new Error("no Buzz shared compute serving members are available"),
    "relay-mesh",
  );

  assert.equal(status?.tone, "warning");
  assert.match(status?.message ?? "", /No members are sharing compute/);
  assert.match(status?.message ?? "", /Settings > Compute/);
});

test("Buzz shared compute distinguishes relay lookup failures", () => {
  const status = formatModelDiscoveryErrorStatus(
    new Error("Buzz shared compute model discovery failed: relay offline"),
    "relay-mesh",
  );

  assert.equal(status?.tone, "warning");
  assert.match(status?.message ?? "", /couldn't check shared compute/);
  assert.match(status?.message ?? "", /relay connection/);
});

test("Buzz shared compute names a missing relay member roster", () => {
  const status = formatModelDiscoveryErrorStatus(
    new Error("Buzz shared compute is waiting for the current member roster"),
    "relay-mesh",
  );

  assert.equal(status?.tone, "warning");
  assert.match(status?.message ?? "", /waiting for the relay's member roster/);
  assert.match(status?.message ?? "", /membership configuration/);
  assert.doesNotMatch(status?.message ?? "", /relay connection/);
});

test("model discovery status stays quiet for missing Databricks defaults", () => {
  const status = formatModelDiscoveryErrorStatus(
    new Error("config: DATABRICKS_HOST required"),
    "databricks",
  );

  assert.equal(status, null);
});
