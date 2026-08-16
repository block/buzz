import assert from "node:assert/strict";
import test from "node:test";

import { coalesceAgentAutocompleteCandidates } from "../lib/agentAutocompleteEligibility.ts";
import { agentIdentityKey } from "../lib/agentIdentity.ts";
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

/** Positional builders for the rename fixtures, where name is the subject. */
function namedAgent(pubkey, name, personaId, status = "stopped") {
  return { pubkey, name, personaId, status };
}

function namedPersona(id, displayName) {
  return { id, displayName };
}

/**
 * The owner's real shape: one builtin persona whose instances were renamed, so
 * the group carries two distinct names across four distinct pubkeys.
 */
const FIZZ_PERSONA = namedPersona("builtin:fizz", "Fizz");
const PUBKEYS = {
  claudeOne: "1".repeat(64),
  claudeTwo: "2".repeat(64),
  fizzOne: "3".repeat(64),
  fizzTwo: "4".repeat(64),
};
const FIZZ_AGENTS = [
  namedAgent(PUBKEYS.claudeOne, "Claude", "builtin:fizz"),
  namedAgent(PUBKEYS.claudeTwo, "Claude", "builtin:fizz"),
  namedAgent(PUBKEYS.fizzOne, "Fizz", "builtin:fizz"),
  namedAgent(PUBKEYS.fizzTwo, "Fizz", "builtin:fizz"),
];

function cardsOf(result) {
  return result.groups.flatMap((group) => group.cards);
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

test("a renamed instance still gets a card in the agents library", () => {
  const { groups } = buildUnifiedGroups(
    [FIZZ_PERSONA],
    FIZZ_AGENTS,
    NONE_ARCHIVED,
  );

  assert.equal(groups.length, 1);
  assert.deepEqual(
    groups[0].cards.map((card) => card.label),
    ["Claude", "Fizz"],
  );
  assert.deepEqual(
    groups[0].cards.map((card) => card.agent.pubkey),
    [PUBKEYS.claudeOne, PUBKEYS.fizzOne],
  );
});

test("a fully archived name gets no card of its own", () => {
  // Archiving every "Claude" leaves only "Fizz" as a clickable card — an
  // archived identity must never become a library card in its own right.
  const isArchived = (pubkey) =>
    pubkey === PUBKEYS.claudeOne || pubkey === PUBKEYS.claudeTwo;

  const { groups } = buildUnifiedGroups(
    [FIZZ_PERSONA],
    FIZZ_AGENTS,
    isArchived,
  );

  assert.deepEqual(
    groups[0].cards.map((card) => card.label),
    ["Fizz"],
  );
  assert.equal(groups[0].cards[0].ownsPersonaActions, true);
});

test("a split persona with every instance archived keeps one persona-only card", () => {
  const { groups } = buildUnifiedGroups([FIZZ_PERSONA], FIZZ_AGENTS, () => true);

  assert.equal(groups[0].cards.length, 1);
  assert.equal(groups[0].cards[0].key, FIZZ_PERSONA.id);
  assert.equal(groups[0].cards[0].label, FIZZ_PERSONA.displayName);
  assert.equal(groups[0].cards[0].agent, undefined);
  assert.equal(groups[0].cards[0].ownsPersonaActions, true);
});

test("no managed agent is dropped by persona grouping", () => {
  const { groups } = buildUnifiedGroups(
    [FIZZ_PERSONA],
    FIZZ_AGENTS,
    NONE_ARCHIVED,
  );
  const reachable = cardsOf({ groups })
    .flatMap((card) => card.agents)
    .map(agentIdentityKey);

  assert.deepEqual(
    new Set(reachable),
    new Set(FIZZ_AGENTS.map(agentIdentityKey)),
  );
  assert.equal(reachable.length, FIZZ_AGENTS.length);
});

test("the agents library and @-mention autocomplete agree on which agents exist", () => {
  const records = [
    ...FIZZ_AGENTS,
    namedAgent("5".repeat(64), "Solo", "custom:solo", "running"),
  ];
  const { groups, ungrouped, unknown } = buildUnifiedGroups(
    [FIZZ_PERSONA, namedPersona("custom:solo", "Solo")],
    records,
    NONE_ARCHIVED,
  );

  const libraryIdentities = new Set(
    [
      ...cardsOf({ groups }).flatMap((card) => card.agents),
      ...ungrouped,
      ...unknown,
    ].map(agentIdentityKey),
  );
  const autocompleteIdentities = new Set(
    coalesceAgentAutocompleteCandidates(
      records.map((record) => ({ ...record, isAgent: true })),
      { getLabel: (candidate) => candidate.name },
    ).map(agentIdentityKey),
  );

  assert.deepEqual(libraryIdentities, autocompleteIdentities);
});

test("same-named instances of one persona stay on the persona's single card", () => {
  const duplicates = [
    namedAgent(
      PUBKEYS.claudeOne,
      "Duplicate Auditor",
      "custom:duplicate",
      "running",
    ),
    namedAgent(PUBKEYS.claudeTwo, "Duplicate Auditor", "custom:duplicate"),
  ];

  const { groups } = buildUnifiedGroups(
    [namedPersona("custom:duplicate", "Duplicate Auditor")],
    duplicates,
    NONE_ARCHIVED,
  );

  assert.equal(groups[0].cards.length, 1);
  assert.equal(groups[0].cards[0].key, "custom:duplicate");
  assert.equal(groups[0].cards[0].label, "Duplicate Auditor");
  assert.equal(groups[0].cards[0].agents.length, 2);
  assert.equal(groups[0].cards[0].ownsPersonaActions, true);
});

test("persona-level actions live on exactly one card per persona", () => {
  const { groups } = buildUnifiedGroups(
    [FIZZ_PERSONA],
    FIZZ_AGENTS,
    NONE_ARCHIVED,
  );

  assert.equal(
    groups[0].cards.filter((card) => card.ownsPersonaActions).length,
    1,
  );
});

test("card keys stay unique so cards cannot overwrite each other", () => {
  const { groups } = buildUnifiedGroups(
    [FIZZ_PERSONA],
    FIZZ_AGENTS,
    NONE_ARCHIVED,
  );
  const keys = groups[0].cards.map((card) => card.key);

  assert.equal(new Set(keys).size, keys.length);
});

test("a persona with no instances keeps its single unchanged card", () => {
  const { groups } = buildUnifiedGroups(
    [namedPersona("custom:idle", "Idle")],
    [],
    NONE_ARCHIVED,
  );

  assert.equal(groups[0].cards.length, 1);
  assert.equal(groups[0].cards[0].key, "custom:idle");
  assert.equal(groups[0].cards[0].label, "Idle");
  assert.equal(groups[0].cards[0].agent, undefined);
});

test("agents without a persona, and personas that vanished, are unchanged", () => {
  const orphan = namedAgent("6".repeat(64), "Orphan", "custom:gone");
  const custom = namedAgent("7".repeat(64), "Custom", null);

  const { ungrouped, unknown } = buildUnifiedGroups(
    [],
    [orphan, custom],
    NONE_ARCHIVED,
  );

  assert.deepEqual(ungrouped, [custom]);
  assert.deepEqual(unknown, [orphan]);
});
