import assert from "node:assert/strict";
import test from "node:test";

import {
  envVarsClearingManagedApiKey,
  envVarsWithoutKey,
  envVarsWithoutKeyCaseInsensitive,
} from "./providerEnvVarUpdates.ts";

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

test("envVarsWithoutKeyCaseInsensitive removes every case-colliding alias in one pass", () => {
  // Windows Command case-folds env names, so a persisted state can hold both
  // the canonical key and a mixed-case duplicate; a runtime switch must clear
  // all of them or a survivor keeps shadowing the projected value on launch.
  const next = envVarsWithoutKeyCaseInsensitive(
    {
      GOOSE_THINKING_EFFORT: "high",
      goose_thinking_effort: "low",
      KEEP: "x",
    },
    "GOOSE_THINKING_EFFORT",
  );
  assert.deepEqual(next, { KEEP: "x" });
});

test("envVarsWithoutKeyCaseInsensitive returns the same reference when no alias matches", () => {
  const current = { KEEP: "x" };
  assert.equal(
    envVarsWithoutKeyCaseInsensitive(current, "GOOSE_THINKING_EFFORT"),
    current,
  );
});
