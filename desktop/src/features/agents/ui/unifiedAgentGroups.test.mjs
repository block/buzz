import assert from "node:assert/strict";
import test from "node:test";

import { buildUnifiedGroups } from "./unifiedAgentGroups.ts";

const commandPersonas = [
  "chief-of-staff",
  "operations",
  "navigation",
  "daily-routine",
  "reporting",
  "plans",
].map((slug) => ({
  id: `builtin:command-${slug}`,
  displayName: slug,
}));

test("separates the six stable advisers into one ordered Command Team group", () => {
  const fizz = { id: "builtin:fizz", displayName: "Fizz" };
  const agents = [
    {
      pubkey: "a".repeat(64),
      name: "Navigator",
      personaId: "builtin:command-navigation",
      status: "running",
    },
  ];

  const result = buildUnifiedGroups([fizz, ...commandPersonas], agents);

  assert.deepEqual(
    result.commandTeamGroups.map(({ persona }) => persona.id),
    commandPersonas.map(({ id }) => id),
  );
  assert.deepEqual(
    result.groups.map(({ persona }) => persona.id),
    ["builtin:fizz"],
  );
  assert.equal(result.commandTeamGroups[2].agents[0].pubkey, "a".repeat(64));
});

test("does not duplicate a command persona or lose unknown agents", () => {
  const unknown = {
    pubkey: "b".repeat(64),
    name: "Legacy",
    personaId: "custom:missing",
    status: "stopped",
  };

  const result = buildUnifiedGroups(
    [commandPersonas[0], commandPersonas[0]],
    [unknown],
  );

  assert.deepEqual(
    result.commandTeamGroups.map(({ persona }) => persona.id),
    ["builtin:command-chief-of-staff"],
  );
  assert.deepEqual(result.unknown, [unknown]);
});
