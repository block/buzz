import assert from "node:assert/strict";
import test from "node:test";

import { searchPersonaChooser } from "./personaChooserSearch.ts";

function persona(id, overrides = {}) {
  return {
    id,
    displayName: id,
    description: null,
    systemPrompt: "",
    ...overrides,
  };
}

function team(id, personaIds, overrides = {}) {
  return { id, name: id, description: null, personaIds, ...overrides };
}

const personas = [
  persona("Scout", { systemPrompt: "Research the codebase." }),
  persona("Ralph", { description: "Reviews pull requests." }),
  persona("Writer", { systemPrompt: "Draft release notes." }),
];
const teams = [
  team("Engineering", ["Scout", "Ralph"]),
  team("Comms", ["Writer"], { description: "Announcements and docs." }),
];

const names = (items) => items.map((item) => item.id);

test("an empty or blank query returns everything, as copies", () => {
  for (const query of ["", "   "]) {
    const result = searchPersonaChooser(personas, teams, query);
    assert.deepEqual(names(result.personas), ["Scout", "Ralph", "Writer"]);
    assert.deepEqual(names(result.teams), ["Engineering", "Comms"]);
    assert.notEqual(result.personas, personas);
  }
});

test("multi-word queries must match every term, case-insensitively", () => {
  assert.deepEqual(
    names(searchPersonaChooser(personas, teams, "PULL  regressions").personas),
    [],
  );
  assert.deepEqual(
    names(searchPersonaChooser(personas, teams, "Reviews PULL").personas),
    ["Ralph"],
  );
});

test("personas match on name, description and system prompt", () => {
  assert.deepEqual(
    names(searchPersonaChooser(personas, teams, "ralph").personas),
    ["Ralph"],
  );
  assert.deepEqual(
    names(searchPersonaChooser(personas, teams, "pull requests").personas),
    ["Ralph"],
  );
  assert.deepEqual(
    names(searchPersonaChooser(personas, teams, "codebase").personas),
    ["Scout"],
  );
  assert.deepEqual(
    names(searchPersonaChooser(personas, teams, "nothing here").personas),
    [],
  );
});

test("teams match on their own text or through a member persona", () => {
  // Team text.
  assert.deepEqual(names(searchPersonaChooser(personas, teams, "docs").teams), [
    "Comms",
  ]);
  // Member persona only: no team text mentions "codebase".
  assert.deepEqual(
    names(searchPersonaChooser(personas, teams, "codebase").teams),
    ["Engineering"],
  );
  // Team name without any member match still keeps the team, drops personas.
  const byTeamName = searchPersonaChooser(personas, teams, "engineering");
  assert.deepEqual(names(byTeamName.teams), ["Engineering"]);
  assert.deepEqual(names(byTeamName.personas), []);
});

test("a team member that is not in the persona list cannot match", () => {
  const orphaned = [team("Ghosts", ["Missing"])];
  assert.deepEqual(
    names(searchPersonaChooser(personas, orphaned, "missing").teams),
    [],
  );
});
