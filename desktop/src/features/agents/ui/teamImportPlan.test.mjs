import assert from "node:assert/strict";
import test from "node:test";

import { buildTeamImportPlan } from "./teamImportPlan.ts";

function createPersona(
  id,
  displayName,
  systemPrompt = "Prompt",
  avatarUrl = null,
) {
  return {
    id,
    displayName,
    avatarUrl,
    systemPrompt,
    runtime: null,
    model: null,
    namePool: [],
    isBuiltIn: false,
    isActive: true,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
  };
}

function createTeam(personaIds) {
  return {
    id: "team-1",
    name: "Alpha Team",
    description: "Original description",
    personaIds,
    createdAt: "2026-01-01T00:00:00Z",
    updatedAt: "2026-01-01T00:00:00Z",
  };
}

test("buildTeamImportPlan categorizes updated, added, and missing members", () => {
  const personas = [
    createPersona("persona-1", "Alice", "Old prompt"),
    createPersona("persona-2", "Bob", "Bob prompt"),
  ];
  const team = createTeam(["persona-1", "persona-2"]);
  const preview = {
    name: "Alpha Team v2",
    description: "Imported description",
    personas: [
      {
        display_name: "alice",
        system_prompt: "New prompt",
        avatar_url: null,
      },
      {
        display_name: "Cara",
        system_prompt: "Cara prompt",
        avatar_url: null,
      },
    ],
  };

  const plan = buildTeamImportPlan({ team, personas, preview });

  assert.equal(plan.membersToUpdate.length, 1);
  assert.equal(plan.membersToUpdate[0]?.existing.id, "persona-1");
  assert.equal(plan.membersToUpdate[0]?.addedLines, 2);
  assert.equal(plan.membersToUpdate[0]?.removedLines, 2);
  assert.equal(plan.newMembers.length, 1);
  assert.equal(plan.newMembers[0]?.imported.display_name, "Cara");
  assert.equal(plan.newMembers[0]?.addedLines, 3);
  assert.equal(plan.missingMembers.length, 1);
  assert.equal(plan.missingMembers[0]?.existing.id, "persona-2");
  assert.equal(plan.missingMembers[0]?.removedLines, 3);
  assert.equal(plan.teamNameChanged, true);
  assert.equal(plan.teamDescriptionChanged, true);
});

test("buildTeamImportPlan pairs duplicate names in stable order", () => {
  const personas = [
    createPersona("persona-1", "Sam", "First prompt"),
    createPersona("persona-2", "Sam", "Second prompt"),
  ];
  const team = createTeam(["persona-1", "persona-2"]);
  const preview = {
    name: "Alpha Team",
    description: "Original description",
    personas: [
      {
        display_name: "Sam",
        system_prompt: "First prompt updated",
        avatar_url: null,
      },
      {
        display_name: "Sam",
        system_prompt: "Second prompt updated",
        avatar_url: null,
      },
    ],
  };

  const plan = buildTeamImportPlan({ team, personas, preview });

  assert.equal(plan.matchedMembers.length, 2);
  assert.equal(plan.matchedMembers[0]?.existing.id, "persona-1");
  assert.equal(plan.matchedMembers[1]?.existing.id, "persona-2");
  assert.equal(plan.matchedMembers[0]?.addedLines, 1);
  assert.equal(plan.matchedMembers[0]?.removedLines, 1);
  assert.equal(plan.matchedMembers[1]?.addedLines, 1);
  assert.equal(plan.matchedMembers[1]?.removedLines, 1);
  assert.equal(plan.missingMembers.length, 0);
  assert.equal(plan.newMembers.length, 0);
});

test("buildTeamImportPlan reports unresolved persona ids from stale team membership", () => {
  const personas = [createPersona("persona-1", "Alice")];
  const team = createTeam(["persona-1", "persona-missing"]);
  const preview = {
    name: "Alpha Team",
    description: "Original description",
    personas: [
      {
        display_name: "Alice",
        system_prompt: "Prompt",
        avatar_url: null,
      },
    ],
  };

  const plan = buildTeamImportPlan({ team, personas, preview });

  assert.deepEqual(plan.unresolvedPersonaIds, ["persona-missing"]);
});
