import assert from "node:assert/strict";
import test from "node:test";

import {
  definitionEditPreservesExecutionConfiguration,
  definitionExecutionReadinessSatisfied,
} from "./agentDefinitionExecutionReadiness.ts";

const HOSTED_DEFINITION = {
  runtime: "buzz-agent",
  model: null,
  provider: null,
  envVars: {},
};

test("unchanged hosted definition can save profile edits without local defaults", () => {
  const preservesExecutionConfiguration =
    definitionEditPreservesExecutionConfiguration({
      initial: HOSTED_DEFINITION,
      runtime: "buzz-agent",
      model: "",
      provider: "",
      envVars: {},
      isRuntimeAutoSeeded: false,
    });

  assert.equal(preservesExecutionConfiguration, true);
  assert.equal(
    definitionExecutionReadinessSatisfied({
      isEditMode: true,
      preservesExecutionConfiguration,
      localModeSatisfied: false,
      customAiPairSatisfied: true,
    }),
    true,
    "name/instructions edits must not require irrelevant local provider defaults",
  );
});

test("changing an execution field restores the readiness gate", () => {
  const cases = [
    { runtime: "codex", model: "", provider: "", envVars: {} },
    {
      runtime: "buzz-agent",
      model: "claude-opus-4-1",
      provider: "anthropic",
      envVars: {},
    },
    {
      runtime: "buzz-agent",
      model: "",
      provider: "",
      envVars: { ANTHROPIC_API_KEY: "changed" },
    },
  ];

  for (const next of cases) {
    const preservesExecutionConfiguration =
      definitionEditPreservesExecutionConfiguration({
        initial: HOSTED_DEFINITION,
        ...next,
        isRuntimeAutoSeeded: false,
      });
    assert.equal(preservesExecutionConfiguration, false);
    assert.equal(
      definitionExecutionReadinessSatisfied({
        isEditMode: true,
        preservesExecutionConfiguration,
        localModeSatisfied: false,
        customAiPairSatisfied: false,
      }),
      false,
    );
  }
});

test("create mode never bypasses execution readiness", () => {
  assert.equal(
    definitionExecutionReadinessSatisfied({
      isEditMode: false,
      preservesExecutionConfiguration: true,
      localModeSatisfied: false,
      customAiPairSatisfied: true,
    }),
    false,
  );
});

test("ready create and execution edits remain submit-ready", () => {
  assert.equal(
    definitionExecutionReadinessSatisfied({
      isEditMode: false,
      preservesExecutionConfiguration: false,
      localModeSatisfied: true,
      customAiPairSatisfied: true,
    }),
    true,
  );
  assert.equal(
    definitionExecutionReadinessSatisfied({
      isEditMode: true,
      preservesExecutionConfiguration: false,
      localModeSatisfied: true,
      customAiPairSatisfied: true,
    }),
    true,
  );
});

test("auto-seeded runtime remains unchanged when submit omits it", () => {
  assert.equal(
    definitionEditPreservesExecutionConfiguration({
      initial: { runtime: null, model: null, provider: null, envVars: {} },
      runtime: "buzz-agent",
      model: "",
      provider: "",
      envVars: {},
      isRuntimeAutoSeeded: true,
    }),
    true,
  );
});
