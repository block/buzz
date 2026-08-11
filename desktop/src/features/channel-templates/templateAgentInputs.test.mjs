import assert from "node:assert/strict";
import test from "node:test";

import {
  buildTemplateAgentInputs,
  dataAfterInitialLoad,
} from "./templateAgentInputs.ts";

function createPersona(id, overrides = {}) {
  return {
    id,
    displayName: `Persona ${id}`,
    avatarUrl: null,
    systemPrompt: `Prompt ${id}`,
    runtime: null,
    model: null,
    provider: null,
    namePool: [],
    isBuiltIn: false,
    isActive: true,
    shared: false,
    envVars: {},
    respondTo: null,
    respondToAllowlist: [],
    parallelism: null,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
    ...overrides,
  };
}

function createTeam(id, personaIds) {
  return {
    id,
    name: `Team ${id}`,
    description: null,
    instructions: `Instructions ${id}`,
    personaIds,
    isBuiltin: false,
    sourceDir: null,
    isSymlink: false,
    symlinkTarget: null,
    version: null,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
  };
}

function createTemplate(agents) {
  return {
    id: "template-1",
    name: "Template",
    description: null,
    channelType: "stream",
    visibility: "open",
    canvasTemplate: null,
    projectFolders: [],
    projectFolder: null,
    worktree: null,
    agents,
    isBuiltin: false,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
  };
}

const runtime = {
  id: "runtime-1",
  label: "Runtime",
  avatarUrl: "",
  availability: "available",
  command: "runtime-command",
  binaryPath: "/runtime-command",
  defaultArgs: [],
  mcpCommand: null,
  modelEnvVar: null,
  providerEnvVar: null,
  thinkingEnvVar: null,
  maxTokensEnvVar: null,
  contextLimitEnvVar: null,
  maxRoundsEnvVar: null,
  installHint: "",
  installInstructionsUrl: "",
  canAutoInstall: false,
  requiresExternalCli: false,
  underlyingCliPath: null,
  nodeRequired: false,
  authStatus: { status: "not_applicable" },
  loginHint: null,
  source: "builtin",
};

test("team template entries preserve team context and force a fresh instance", () => {
  const persona = createPersona("persona-1");
  const inputs = buildTemplateAgentInputs({
    template: createTemplate({
      personas: [],
      teams: [
        {
          teamId: "team-1",
          runtime: null,
          model: null,
          backend: null,
        },
      ],
    }),
    personas: [persona],
    teams: [createTeam("team-1", [persona.id])],
    runtimes: [runtime],
    lastRuntimeId: null,
  });

  assert.equal(inputs.length, 1);
  assert.equal(inputs[0].personaId, persona.id);
  assert.equal(inputs[0].teamId, "team-1");
  assert.equal(inputs[0].forceNewInstance, true);
});

test("the same persona can retain distinct direct and per-team contexts", () => {
  const persona = createPersona("persona-1");
  const inputs = buildTemplateAgentInputs({
    template: createTemplate({
      personas: [
        {
          personaId: persona.id,
          runtime: null,
          model: null,
          role: null,
          backend: null,
        },
      ],
      teams: [
        {
          teamId: "team-1",
          runtime: null,
          model: null,
          backend: null,
        },
        {
          teamId: "team-2",
          runtime: null,
          model: null,
          backend: null,
        },
      ],
    }),
    personas: [persona],
    teams: [
      createTeam("team-1", [persona.id]),
      createTeam("team-2", [persona.id]),
    ],
    runtimes: [runtime],
    lastRuntimeId: null,
  });

  assert.deepEqual(
    inputs.map((input) => input.teamId ?? null),
    [null, "team-1", "team-2"],
  );
});

test("dataAfterInitialLoad waits for missing query data", async () => {
  const expected = [createPersona("persona-1")];
  let refetched = false;

  const result = await dataAfterInitialLoad({
    data: undefined,
    async refetch() {
      refetched = true;
      return { data: expected, error: null };
    },
  });

  assert.equal(refetched, true);
  assert.equal(result, expected);
});

test("dataAfterInitialLoad reuses cached query data without refetching", async () => {
  const expected = [createPersona("persona-1")];

  const result = await dataAfterInitialLoad({
    data: expected,
    async refetch() {
      assert.fail("cached data should not refetch");
    },
  });

  assert.equal(result, expected);
});
