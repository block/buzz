import assert from "node:assert/strict";
import test from "node:test";
import { resolveGooseConfig } from "./gooseConfigDefaults.ts";
import { computeLocalModeGate } from "./agentConfigOptions.tsx";

const defaults = {
  GOOSE_PROVIDER: "databricks_v2",
  GOOSE_MODEL: "bundled-model",
};
const file = { provider: "anthropic", model: "file-model" };

for (const scenario of [
  {
    name: "fresh install",
    input: { defaults },
    expected: ["databricks_v2", "bundled-model", "build"],
  },
  {
    name: "file settings",
    input: { defaults, file },
    expected: ["anthropic", "file-model", "file"],
  },
  {
    name: "Buzz selections",
    input: { defaults, file, provider: "openai", model: "chosen-model" },
    expected: ["openai", "chosen-model", "global"],
  },
  {
    name: "environment overrides",
    input: {
      defaults,
      file,
      provider: "openai",
      model: "chosen-model",
      env: { GOOSE_PROVIDER: "anthropic", GOOSE_MODEL: "env-model" },
    },
    expected: ["anthropic", "env-model", "environment"],
  },
  {
    name: "external Goose without defaults",
    input: { file },
    expected: ["anthropic", "file-model", "file"],
  },
]) {
  test(`Goose defaults: ${scenario.name}`, () => {
    const result = resolveGooseConfig(scenario.input);
    assert.deepEqual(
      [result.provider.value, result.model.value, result.model.source],
      scenario.expected,
    );
    const gate = computeLocalModeGate({
      envVars: {
        ANTHROPIC_API_KEY: "test",
        OPENAI_API_KEY: "test",
        DATABRICKS_HOST: "https://example.com",
      },
      isProviderMode: false,
      runtimeId: "goose",
      provider: result.provider.value,
      model: result.model.value,
    });
    assert.deepEqual(gate.missingNormalizedFields, []);
  });
}

test("Goose falls back independently for missing fields", () => {
  assert.equal(
    resolveGooseConfig({ defaults, file: { model: "file-model" } }).provider
      .value,
    "databricks_v2",
  );
  assert.equal(
    resolveGooseConfig({ defaults, file: { model: "file-model" } }).model.value,
    "file-model",
  );
  assert.equal(resolveGooseConfig({}).provider.value, "");
  assert.equal(resolveGooseConfig({}).model.value, "");
});
