import assert from "node:assert/strict";
import fs from "node:fs";
import test from "node:test";
import vm from "node:vm";

import ts from "typescript";

function loadUseApplyTemplate({ createChannelManagedAgents }) {
  const source = fs.readFileSync(
    new URL("./useApplyTemplate.ts", import.meta.url),
    "utf8",
  );
  const template = {
    id: "provider-template",
    agents: {
      personas: [
        {
          personaId: "persona-1",
          runtime: null,
          model: null,
          role: "bot",
          backend: { type: "provider", id: "legacy-provider" },
        },
      ],
      teams: [],
    },
  };
  const stubs = {
    "@tanstack/react-query": {
      useQueryClient: () => ({ invalidateQueries: async () => {} }),
    },
    "@/features/agents/channelAgents": { createChannelManagedAgents },
    "@/features/agents/hooks": {
      useAvailableAcpRuntimes: () => ({
        data: [{ id: "runtime-1", command: "agent", label: "Agent" }],
      }),
      usePersonasQuery: () => ({
        data: [
          {
            id: "persona-1",
            displayName: "Template Agent",
            systemPrompt: "Help",
          },
        ],
      }),
      useTeamsQuery: () => ({ data: [] }),
    },
    "@/features/agents/lib/resolvePersonaRuntime": {
      resolvePersonaRuntime: (_saved, runtimes) => ({ runtime: runtimes[0] }),
    },
    "@/features/agents/lib/teamPersonas": {
      resolveTeamPersonas: () => ({ resolvedPersonas: [] }),
    },
    "@/features/agents/lib/useLastRuntime": {
      useLastRuntime: () => ({ lastRuntimeId: null }),
    },
    "@/features/channel-templates/hooks": {
      useChannelTemplatesQuery: () => ({ data: [template] }),
    },
    "@/shared/api/tauri": { setCanvas: async () => {} },
    "@/shared/api/types": {},
  };
  const exports = {};
  vm.runInNewContext(
    ts.transpileModule(source, {
      compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        target: ts.ScriptTarget.ES2022,
      },
    }).outputText,
    {
      exports,
      require: (key) => {
        assert.ok(key in stubs, `unmocked dependency: ${key}`);
        return stubs[key];
      },
    },
  );
  return exports.useApplyTemplate;
}

test("saved provider templates preserve their explicit local-custody contract", async () => {
  let captured;
  const useApplyTemplate = loadUseApplyTemplate({
    createChannelManagedAgents: async (channelId, inputs) => {
      captured = { channelId, inputs };
      return { failures: [], successes: [] };
    },
  });

  const { applyAgents } = useApplyTemplate();
  await applyAgents("provider-template", "channel-1");

  assert.equal(captured.channelId, "channel-1");
  assert.equal(captured.inputs.length, 1);
  assert.equal(captured.inputs[0].backend.type, "provider");
  assert.equal(captured.inputs[0].backend.id, "legacy-provider");
  assert.equal(Object.keys(captured.inputs[0].backend.config).length, 0);
  assert.equal(captured.inputs[0].expectedKeyCustody, "local");
});
