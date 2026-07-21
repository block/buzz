import assert from "node:assert/strict";
import test from "node:test";

import {
  formatModelDiscoveryErrorStatus,
  formatModelDiscoveryLoadingMessage,
  isModelDiscoveryTimeoutError,
  MODEL_DISCOVERY_SLOW_MS,
} from "./personaModelDiscoveryStatus.ts";

test("model discovery status names missing Anthropic credentials", () => {
  const status = formatModelDiscoveryErrorStatus(
    new Error("config: ANTHROPIC_API_KEY required"),
    "anthropic",
  );

  assert.equal(status?.tone, "warning");
  assert.match(status?.message ?? "", /Anthropic API key/);
  assert.match(status?.message ?? "", /Anthropic models/);
  // Credential gaps need user input — not a Retry-only fix.
  assert.equal(status?.retryable, undefined);
});

test("model discovery status names missing OpenAI-compatible credentials", () => {
  const status = formatModelDiscoveryErrorStatus(
    new Error("config: OPENAI_COMPAT_API_KEY required"),
    "openai-compat",
  );

  assert.equal(status?.tone, "warning");
  assert.match(status?.message ?? "", /OpenAI API key/);
  assert.match(status?.message ?? "", /OpenAI models/);
  assert.equal(status?.retryable, undefined);
});

test("Buzz shared compute names the empty state and next action", () => {
  const status = formatModelDiscoveryErrorStatus(
    new Error("no Buzz shared compute serving members are available"),
    "relay-mesh",
  );

  assert.equal(status?.tone, "warning");
  assert.match(status?.message ?? "", /No members are sharing compute/);
  assert.match(status?.message ?? "", /Settings > Compute/);
  assert.equal(status?.retryable, true);
});

test("Buzz shared compute distinguishes relay lookup failures", () => {
  const status = formatModelDiscoveryErrorStatus(
    new Error("Buzz shared compute model discovery failed: relay offline"),
    "relay-mesh",
  );

  assert.equal(status?.tone, "warning");
  assert.match(status?.message ?? "", /couldn't check shared compute/);
  assert.match(status?.message ?? "", /relay connection/);
  assert.equal(status?.retryable, true);
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
  assert.equal(status?.retryable, true);
});

test("model discovery status stays quiet for missing Databricks defaults", () => {
  const status = formatModelDiscoveryErrorStatus(
    new Error("config: DATABRICKS_HOST required"),
    "databricks",
  );

  assert.equal(status, null);
});

// ── #2261 timeout / PATH / progressive loading ────────────────────────────────

test("isModelDiscoveryTimeoutError matches buzz-acp probe timeout text", () => {
  assert.equal(
    isModelDiscoveryTimeoutError("error: agent timed out (10s)"),
    true,
  );
  assert.equal(
    isModelDiscoveryTimeoutError(
      "buzz-acp models failed (exit 1): error: agent timed out (45s)",
    ),
    true,
  );
  assert.equal(
    isModelDiscoveryTimeoutError("config: ANTHROPIC_API_KEY required"),
    false,
  );
});

test("formatModelDiscoveryErrorStatus_timeout_isRetryableWithClearCopy", () => {
  const status = formatModelDiscoveryErrorStatus(
    new Error("error: agent timed out (10s)"),
    "",
  );
  assert.equal(status?.tone, "warning");
  assert.equal(status?.retryable, true);
  assert.match(status?.message ?? "", /timed out/i);
  assert.match(status?.message ?? "", /retry/i);
  assert.match(status?.message ?? "", /Codex|20–60|20-60/i);
});

test("formatModelDiscoveryErrorStatus_programNotFound_isRetryable", () => {
  const status = formatModelDiscoveryErrorStatus(
    new Error("failed to spawn agent: program not found"),
    "codex",
  );
  assert.equal(status?.tone, "warning");
  assert.equal(status?.retryable, true);
  assert.match(status?.message ?? "", /PATH/i);
});

test("formatModelDiscoveryLoadingMessage is null until slow phase", () => {
  assert.equal(formatModelDiscoveryLoadingMessage(false), null);
});

test("formatModelDiscoveryLoadingMessage returns long note only when slow", () => {
  const message = formatModelDiscoveryLoadingMessage(true);
  assert.match(message ?? "", /Still loading/i);
  assert.match(message ?? "", /Codex/i);
});

test("MODEL_DISCOVERY_SLOW_MS is 10s before under-field note appears", () => {
  assert.equal(MODEL_DISCOVERY_SLOW_MS, 10_000);
});
