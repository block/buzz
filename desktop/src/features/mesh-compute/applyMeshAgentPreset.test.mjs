import assert from "node:assert/strict";
import test from "node:test";

import {
  detectMeshPresetOverrides,
  meshAgentPresetPatch,
} from "./applyMeshAgentPreset.ts";

const PRESET = {
  providerId: "relay-mesh",
  label: "Relay mesh",
  acpCommand: "buzz-acp",
  agentCommand: "buzz-agent",
  agentArgs: [],
  mcpCommand: "buzz-dev-mcp",
  model: "Qwen3-8B-Q4_K_M",
  envVars: {
    BUZZ_AGENT_PROVIDER: "openai",
    OPENAI_COMPAT_BASE_URL: "http://127.0.0.1:9337/v1",
    OPENAI_COMPAT_MODEL: "Qwen3-8B-Q4_K_M",
    OPENAI_COMPAT_API_KEY: "buzz-mesh-local",
    OPENAI_COMPAT_API: "chat",
  },
};

test("patch carries the fields a managed-agent draft needs", () => {
  const patch = meshAgentPresetPatch(PRESET);
  assert.equal(patch.acpCommand, "buzz-acp");
  assert.equal(patch.agentCommand, "buzz-agent");
  assert.deepEqual(patch.agentArgs, []);
  assert.equal(patch.mcpCommand, "buzz-dev-mcp");
  assert.equal(patch.model, "Qwen3-8B-Q4_K_M");
  assert.equal(patch.envVars.OPENAI_COMPAT_MODEL, "Qwen3-8B-Q4_K_M");
});

test("patch returns owned copies — caller cannot mutate the preset", () => {
  const patch = meshAgentPresetPatch(PRESET);
  patch.agentArgs.push("dirty");
  patch.envVars.DIRTY = "1";
  assert.deepEqual(PRESET.agentArgs, []);
  assert.equal(PRESET.envVars.DIRTY, undefined);
});

test("empty draft has no overrides", () => {
  const overrides = detectMeshPresetOverrides(
    {
      acpCommand: "",
      agentCommand: "",
      agentArgs: [],
      mcpCommand: "",
      model: null,
      envVars: {},
    },
    PRESET,
  );
  assert.deepEqual(overrides, []);
});

test("matching draft has no overrides", () => {
  const overrides = detectMeshPresetOverrides(
    {
      acpCommand: "buzz-acp",
      agentCommand: "buzz-agent",
      agentArgs: [],
      mcpCommand: "buzz-dev-mcp",
      model: "Qwen3-8B-Q4_K_M",
      envVars: {
        BUZZ_AGENT_PROVIDER: "openai",
        OPENAI_COMPAT_BASE_URL: "http://127.0.0.1:9337/v1",
      },
    },
    PRESET,
  );
  assert.deepEqual(overrides, []);
});

test("differing model is reported as override", () => {
  const overrides = detectMeshPresetOverrides(
    {
      acpCommand: "buzz-acp",
      agentCommand: "buzz-agent",
      agentArgs: [],
      mcpCommand: "buzz-dev-mcp",
      model: "llama-3.2-3b-instruct",
      envVars: {},
    },
    PRESET,
  );
  assert.deepEqual(overrides, ["model"]);
});

test("non-buzz-agent runtime + non-mesh model both reported", () => {
  const overrides = detectMeshPresetOverrides(
    {
      acpCommand: "buzz-acp",
      agentCommand: "goose",
      agentArgs: ["acp"],
      mcpCommand: "buzz-dev-mcp",
      model: "gpt-4o",
      envVars: {},
    },
    PRESET,
  );
  assert.deepEqual(overrides, ["agent runtime", "model"]);
});

test("overlapping env-var with differing value is reported", () => {
  const overrides = detectMeshPresetOverrides(
    {
      acpCommand: "buzz-acp",
      agentCommand: "buzz-agent",
      agentArgs: [],
      mcpCommand: "buzz-dev-mcp",
      model: "Qwen3-8B-Q4_K_M",
      envVars: {
        BUZZ_AGENT_PROVIDER: "anthropic",
      },
    },
    PRESET,
  );
  assert.deepEqual(overrides, ["environment variables"]);
});

test("overlapping env-var with same value is NOT reported", () => {
  const overrides = detectMeshPresetOverrides(
    {
      acpCommand: "buzz-acp",
      agentCommand: "buzz-agent",
      agentArgs: [],
      mcpCommand: "buzz-dev-mcp",
      model: "Qwen3-8B-Q4_K_M",
      envVars: {
        BUZZ_AGENT_PROVIDER: "openai",
      },
    },
    PRESET,
  );
  assert.deepEqual(overrides, []);
});

test("additive env-var (new key) is not an override", () => {
  const overrides = detectMeshPresetOverrides(
    {
      acpCommand: "buzz-acp",
      agentCommand: "buzz-agent",
      agentArgs: [],
      mcpCommand: "buzz-dev-mcp",
      model: "Qwen3-8B-Q4_K_M",
      envVars: {
        SOME_USER_VAR: "kept",
      },
    },
    PRESET,
  );
  assert.deepEqual(overrides, []);
});

test("empty model string treated like null (no override)", () => {
  // ManagedAgent.model is `string | null` but a fresh draft sometimes carries
  // "" instead of null. Either should be treated as "user hasn't picked yet."
  const overrides = detectMeshPresetOverrides(
    {
      acpCommand: "buzz-acp",
      agentCommand: "buzz-agent",
      agentArgs: [],
      mcpCommand: "buzz-dev-mcp",
      model: "",
      envVars: {},
    },
    PRESET,
  );
  assert.deepEqual(overrides, []);
});
