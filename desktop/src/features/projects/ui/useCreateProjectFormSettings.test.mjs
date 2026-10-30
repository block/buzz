import assert from "node:assert/strict";
import { test } from "node:test";

import { buildCreateProjectAgents } from "./useCreateProjectFormSettings.ts";

const runtime = {
  id: "buzz-agent",
  label: "Buzz Agent",
  availability: "available",
  command: "buzz-agent",
  binaryPath: "/bin/buzz-agent",
};

function persona(id, displayName) {
  return {
    id,
    displayName,
    avatarUrl: null,
    systemPrompt: `${displayName} instructions`,
    runtime: null,
    model: null,
  };
}

test("project creation expands a team and deduplicates the separately selected persona", () => {
  const alpha = persona("alpha", "Alpha");
  const beta = persona("beta", "Beta");
  const agents = buildCreateProjectAgents({
    agentPersonaId: "beta",
    personas: [alpha, beta],
    runtimes: [runtime],
    teamId: "builders",
    teams: [{ id: "builders", personaIds: ["alpha", "beta"] }],
  });

  assert.deepEqual(
    agents.map(({ personaId, teamId }) => ({ personaId, teamId })),
    [
      { personaId: "alpha", teamId: "builders" },
      { personaId: "beta", teamId: "builders" },
    ],
  );
  assert.equal(agents[0].runtime, runtime);
});
