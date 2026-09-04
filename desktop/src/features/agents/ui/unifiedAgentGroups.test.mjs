import assert from "node:assert/strict";
import test from "node:test";

import { buildUnifiedGroups } from "./unifiedAgentGroups.ts";

const NONE_ARCHIVED = () => false;

function agent(overrides = {}) {
  return {
    name: "Agent",
    pubkey: "a".repeat(64),
    personaId: null,
    status: "stopped",
    ...overrides,
  };
}

function persona(overrides = {}) {
  return { id: "persona-1", displayName: "Persona", ...overrides };
}

test("archived standalone custom agents are omitted while live peers remain", () => {
  const archived = agent({ pubkey: "a".repeat(64), personaId: null });
  const live = agent({ pubkey: "b".repeat(64), personaId: null });
  const isArchived = (pubkey) => pubkey === archived.pubkey;

  const { ungrouped } = buildUnifiedGroups([], [archived, live], isArchived);

  assert.deepEqual(
    ungrouped.map((agent) => agent.pubkey),
    [live.pubkey],
  );
});

test("archived unknown-persona agents are omitted while live peers remain", () => {
  const archived = agent({ pubkey: "a".repeat(64), personaId: "orphan" });
  const live = agent({ pubkey: "b".repeat(64), personaId: "orphan" });
  const isArchived = (pubkey) => pubkey === archived.pubkey;

  // No persona matches "orphan", so both land in the unknown bucket.
  const { unknown } = buildUnifiedGroups([], [archived, live], isArchived);

  assert.deepEqual(
    unknown.map((agent) => agent.pubkey),
    [live.pubkey],
  );
});

test("matched persona groups keep their full instance list including archived", () => {
  const archived = agent({ pubkey: "a".repeat(64), personaId: "persona-1" });
  const live = agent({ pubkey: "b".repeat(64), personaId: "persona-1" });
  const isArchived = (pubkey) => pubkey === archived.pubkey;

  // The card resolves its own target via pickProfileAgent; the group keeps the
  // archived record so an all-archived persona still forms a card in
  // persona-only mode rather than vanishing from the library.
  const { groups } = buildUnifiedGroups(
    [persona()],
    [archived, live],
    isArchived,
  );

  assert.equal(groups.length, 1);
  assert.deepEqual(
    groups[0].agents.map((agent) => agent.pubkey).sort(),
    [archived.pubkey, live.pubkey].sort(),
  );
});

test("a fail-open predicate keeps every standalone agent discoverable", () => {
  const first = agent({ pubkey: "a".repeat(64), personaId: null });
  const second = agent({ pubkey: "b".repeat(64), personaId: null });

  const { ungrouped } = buildUnifiedGroups([], [first, second], NONE_ARCHIVED);

  assert.equal(ungrouped.length, 2);
});
