import assert from "node:assert/strict";
import test from "node:test";

import { fromRawPersona } from "./tauriPersonas.ts";

function rawPersona(overrides = {}) {
  return {
    id: "persona-1",
    display_name: "Team Analyst",
    avatar_url: null,
    system_prompt: "You are Team Analyst.",
    runtime: null,
    model: null,
    provider: null,
    name_pool: [],
    is_builtin: false,
    is_active: true,
    source_team: null,
    env_vars: {},
    created_at: "2026-01-01T00:00:00.000Z",
    updated_at: "2026-01-01T00:00:00.000Z",
    ...overrides,
  };
}

test("fromRawPersona maps source_team to sourceTeam", () => {
  const persona = fromRawPersona(rawPersona({ source_team: "team-research" }));

  assert.equal(persona.sourceTeam, "team-research");
});

test("fromRawPersona maps authored description and defaults absence to null", () => {
  assert.equal(
    fromRawPersona(rawPersona({ description: "A careful analyst." }))
      .description,
    "A careful analyst.",
  );
  assert.equal(fromRawPersona(rawPersona()).description, null);
});

test("persona wire mapping preserves native references and structured MCP settings", () => {
  const capabilities = {
    skills: ["skills/日本語"],
    mcp_servers: [
      {
        name: "probe",
        command: "node",
        args: ["two words"],
        env: { ROLE: "reviewer" },
      },
    ],
    hooks: { on_message: "observe.sh" },
    working_directory: "/tmp/persona",
  };
  assert.deepEqual(
    fromRawPersona(rawPersona({ capabilities })).capabilities,
    capabilities,
  );
});
