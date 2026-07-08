import assert from "node:assert/strict";
import test from "node:test";

import {
  envVarsClearingManagedApiKey,
  envVarsWithProviderApiKey,
  envVarsWithoutKey,
} from "./providerEnvVarUpdates.ts";

test("envVarsWithProviderApiKey sets a non-empty value", () => {
  const current = { OTHER: "x" };
  const next = envVarsWithProviderApiKey(current, "ANTHROPIC_API_KEY", "sk-1");
  assert.deepEqual(next, { OTHER: "x", ANTHROPIC_API_KEY: "sk-1" });
  assert.notEqual(next, current);
});

test("envVarsWithProviderApiKey removes the key on empty value", () => {
  const next = envVarsWithProviderApiKey(
    { ANTHROPIC_API_KEY: "sk-1", OTHER: "x" },
    "ANTHROPIC_API_KEY",
    "",
  );
  assert.deepEqual(next, { OTHER: "x" });
});

test("envVarsWithProviderApiKey returns the same reference on no-op", () => {
  const current = { ANTHROPIC_API_KEY: "sk-1" };
  assert.equal(
    envVarsWithProviderApiKey(current, "ANTHROPIC_API_KEY", "sk-1"),
    current,
  );
  const empty = {};
  assert.equal(
    envVarsWithProviderApiKey(empty, "ANTHROPIC_API_KEY", ""),
    empty,
  );
});

test("envVarsWithoutKey removes a present key", () => {
  assert.deepEqual(envVarsWithoutKey({ A: "1", B: "2" }, "A"), { B: "2" });
});

test("envVarsWithoutKey returns the same reference when the key is absent", () => {
  const current = { A: "1" };
  assert.equal(envVarsWithoutKey(current, "B"), current);
});

test("envVarsClearingManagedApiKey clears the previous provider's key on switch", () => {
  const next = envVarsClearingManagedApiKey(
    { ANTHROPIC_API_KEY: "sk-1", KEEP: "x" },
    "anthropic",
    "openai",
  );
  assert.deepEqual(next, { KEEP: "x" });
});

test("envVarsClearingManagedApiKey clears when leaving to a custom/empty provider", () => {
  // The dialogs' CUSTOM-provider paths delete unconditionally; empty next
  // provider has no managed key, so the inequality always holds — same rule.
  const next = envVarsClearingManagedApiKey(
    { ANTHROPIC_API_KEY: "sk-1" },
    "anthropic",
    "",
  );
  assert.deepEqual(next, {});
});

test("envVarsClearingManagedApiKey is a no-op when the managed key is shared or absent", () => {
  const current = { ANTHROPIC_API_KEY: "sk-1" };
  assert.equal(
    envVarsClearingManagedApiKey(current, "anthropic", "anthropic"),
    current,
  );
  const noManaged = { X: "1" };
  assert.equal(
    envVarsClearingManagedApiKey(noManaged, "", "openai"),
    noManaged,
  );
});
