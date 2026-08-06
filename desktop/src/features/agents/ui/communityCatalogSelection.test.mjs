import assert from "node:assert/strict";
import test from "node:test";

import { nextCatalogSelection } from "./communityCatalogSelection.ts";

// ── Stubs ─────────────────────────────────────────────────────────────────────

const PERSONA_KEY = "p:persona-abc";
const TEAM_KEY = "t:alice-pubkey:release";

// ── Rule 1: user-made selection preserved when item still valid ───────────────

test("user selection is preserved while preferred section is loading", () => {
  // User clicked a team while agents were still loading.
  assert.equal(
    nextCatalogSelection(
      TEAM_KEY,
      /*userHasSelected*/ true,
      /*currentIsValid*/ true,
      "agents",
      /*personasLoading*/ true,
      /*teamsLoading*/ false,
      PERSONA_KEY,
      TEAM_KEY,
    ),
    TEAM_KEY,
  );
});

test("user selection is preserved after all sections have settled", () => {
  assert.equal(
    nextCatalogSelection(
      TEAM_KEY,
      /*userHasSelected*/ true,
      /*currentIsValid*/ true,
      "agents",
      /*personasLoading*/ false,
      /*teamsLoading*/ false,
      PERSONA_KEY,
      TEAM_KEY,
    ),
    TEAM_KEY,
  );
});

// ── Rule 1 (vanish): a retracted user selection falls through to auto-init ────

test("manually selected agent that vanishes falls back to first available item", () => {
  // User had selected an agent (agents section was their preference).
  // A live refresh retracts the agent — currentIsValid is now false.
  // Both sections are settled; personas has a different first item, teams has items.
  // Expected: fall through to rule 3 (preferred section nonempty), pick first agent.
  const OTHER_PERSONA_KEY = "p:persona-xyz";
  assert.equal(
    nextCatalogSelection(
      PERSONA_KEY, // the retracted agent
      /*userHasSelected*/ true,
      /*currentIsValid*/ false, // item vanished
      "agents",
      /*personasLoading*/ false,
      /*teamsLoading*/ false,
      OTHER_PERSONA_KEY, // first item now in the personas list
      TEAM_KEY,
    ),
    OTHER_PERSONA_KEY,
  );
});

test("manually selected team that vanishes falls back to first available item", () => {
  // User had selected a team (teams section was their preference).
  // A live refresh retracts the team — currentIsValid is now false.
  const OTHER_TEAM_KEY = "t:bob-pubkey:security";
  assert.equal(
    nextCatalogSelection(
      TEAM_KEY, // the retracted team
      /*userHasSelected*/ true,
      /*currentIsValid*/ false, // item vanished
      "teams",
      /*personasLoading*/ false,
      /*teamsLoading*/ false,
      PERSONA_KEY,
      OTHER_TEAM_KEY, // first item now in the teams list
    ),
    OTHER_TEAM_KEY,
  );
});

test("vanish while preferred section is loading does not reintroduce premature cross-section fallback", () => {
  // User selected a team (agents was the launch preference) while agents were loading.
  // Live refresh then retracts that team. Agents are still loading.
  // Rule 1 fails (currentIsValid=false), Rule 2 fires (agents still loading) → null.
  // The repair must NOT commit a cross-section fallback from the teams list.
  const OTHER_TEAM_KEY = "t:bob-pubkey:security";
  assert.equal(
    nextCatalogSelection(
      TEAM_KEY, // retracted team
      /*userHasSelected*/ true,
      /*currentIsValid*/ false, // item vanished
      "agents",
      /*personasLoading*/ true, // preferred section still loading
      /*teamsLoading*/ false,
      null, // no agents available yet
      OTHER_TEAM_KEY, // a team item is available, but must not be auto-committed
    ),
    null, // wait for agents to settle
  );
});

// ── Rule 2: no auto-commit while preferred section is loading ─────────────────

test("agents launch: no selection while agents are loading, even when teams are ready", () => {
  // Thufir's race scenario — teams settle first.
  assert.equal(
    nextCatalogSelection(
      null,
      /*userHasSelected*/ false,
      /*currentIsValid*/ false,
      "agents",
      /*personasLoading*/ true,
      /*teamsLoading*/ false,
      null, // firstPersonaKey not yet available
      TEAM_KEY,
    ),
    null,
  );
});

test("teams launch: no selection while teams are loading, even when agents are ready", () => {
  // Symmetric race — agents settle first.
  assert.equal(
    nextCatalogSelection(
      null,
      /*userHasSelected*/ false,
      /*currentIsValid*/ false,
      "teams",
      /*personasLoading*/ false,
      /*teamsLoading*/ true,
      PERSONA_KEY,
      null, // firstTeamKey not yet available
    ),
    null,
  );
});

// ── Rule 3: preferred section settled nonempty → select its first item ────────

test("agents launch: selects first agent once agents settle", () => {
  assert.equal(
    nextCatalogSelection(
      null,
      /*userHasSelected*/ false,
      /*currentIsValid*/ false,
      "agents",
      /*personasLoading*/ false,
      /*teamsLoading*/ false,
      PERSONA_KEY,
      TEAM_KEY,
    ),
    PERSONA_KEY,
  );
});

test("teams launch: selects first team once teams settle", () => {
  assert.equal(
    nextCatalogSelection(
      null,
      /*userHasSelected*/ false,
      /*currentIsValid*/ false,
      "teams",
      /*personasLoading*/ false,
      /*teamsLoading*/ false,
      PERSONA_KEY,
      TEAM_KEY,
    ),
    TEAM_KEY,
  );
});

// ── Rule 4: preferred section settled empty → fall back to other section ──────

test("agents launch: falls back to first team when agents section is empty", () => {
  assert.equal(
    nextCatalogSelection(
      null,
      /*userHasSelected*/ false,
      /*currentIsValid*/ false,
      "agents",
      /*personasLoading*/ false,
      /*teamsLoading*/ false,
      null, // no personas
      TEAM_KEY,
    ),
    TEAM_KEY,
  );
});

test("teams launch: falls back to first agent when teams section is empty", () => {
  assert.equal(
    nextCatalogSelection(
      null,
      /*userHasSelected*/ false,
      /*currentIsValid*/ false,
      "teams",
      /*personasLoading*/ false,
      /*teamsLoading*/ false,
      PERSONA_KEY,
      null, // no teams
    ),
    PERSONA_KEY,
  );
});

// ── Rule 5: both settled and empty → null ────────────────────────────────────

test("both sections settled and empty yields null", () => {
  assert.equal(
    nextCatalogSelection(
      null,
      /*userHasSelected*/ false,
      /*currentIsValid*/ false,
      "agents",
      /*personasLoading*/ false,
      /*teamsLoading*/ false,
      null,
      null,
    ),
    null,
  );
});

// ── Staggered-settlement regressions (Thufir's required scenarios) ────────────

test("agents launch staggered: teams settle first then agents settle — selects agent not team", () => {
  // Step 1: teams settle while agents still loading → auto-init stays null.
  const afterTeamsSettle = nextCatalogSelection(
    null,
    false,
    /*currentIsValid*/ false,
    "agents",
    /*personasLoading*/ true,
    /*teamsLoading*/ false,
    null, // agents not ready
    TEAM_KEY,
  );
  assert.equal(afterTeamsSettle, null);

  // Step 2: agents now settle → selects first agent, not the team.
  const afterAgentsSettle = nextCatalogSelection(
    afterTeamsSettle,
    false,
    /*currentIsValid*/ false,
    "agents",
    /*personasLoading*/ false,
    /*teamsLoading*/ false,
    PERSONA_KEY,
    TEAM_KEY,
  );
  assert.equal(afterAgentsSettle, PERSONA_KEY);
});

test("teams launch staggered: agents settle first then teams settle — selects team not agent", () => {
  // Step 1: agents settle while teams still loading → auto-init stays null.
  const afterAgentsSettle = nextCatalogSelection(
    null,
    false,
    /*currentIsValid*/ false,
    "teams",
    /*personasLoading*/ false,
    /*teamsLoading*/ true,
    PERSONA_KEY,
    null, // teams not ready
  );
  assert.equal(afterAgentsSettle, null);

  // Step 2: teams now settle → selects first team, not the agent.
  const afterTeamsSettle = nextCatalogSelection(
    afterAgentsSettle,
    false,
    /*currentIsValid*/ false,
    "teams",
    /*personasLoading*/ false,
    /*teamsLoading*/ false,
    PERSONA_KEY,
    TEAM_KEY,
  );
  assert.equal(afterTeamsSettle, TEAM_KEY);
});

test("user selects from ready section while preferred section is loading — preserved after preferred settles", () => {
  // Teams are ready; agents are loading. User clicks a team while waiting.
  // (userHasSelected = true because the user clicked)
  const afterUserClick = nextCatalogSelection(
    TEAM_KEY,
    /*userHasSelected*/ true,
    /*currentIsValid*/ true, // team exists in current data
    "agents",
    /*personasLoading*/ true,
    /*teamsLoading*/ false,
    null, // agents not ready yet
    TEAM_KEY,
  );
  assert.equal(afterUserClick, TEAM_KEY);

  // Agents now settle — user's team selection must survive.
  const afterAgentsSettle = nextCatalogSelection(
    TEAM_KEY,
    /*userHasSelected*/ true, // still true — user clicked
    /*currentIsValid*/ true, // team still present
    "agents",
    /*personasLoading*/ false,
    /*teamsLoading*/ false,
    PERSONA_KEY,
    TEAM_KEY,
  );
  assert.equal(afterAgentsSettle, TEAM_KEY);
});
