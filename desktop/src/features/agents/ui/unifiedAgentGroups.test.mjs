import assert from "node:assert/strict";
import test from "node:test";

import {
  buildUnifiedGroups,
  profileAgentsForGroup,
} from "./unifiedAgentGroups.ts";

function agent(pubkey, overrides = {}) {
  return {
    pubkey,
    name: overrides.name ?? "Fizz",
    personaId: overrides.personaId ?? "builtin:fizz",
    status: overrides.status ?? "stopped",
    ...overrides,
  };
}

const fizz = { id: "builtin:fizz", displayName: "Fizz" };
const NONE_ARCHIVED = () => false;

test("buildUnifiedGroups retains every managed instance for one persona", () => {
  const first = agent("a".repeat(64));
  const second = agent("b".repeat(64));

  const { groups, ungrouped, unknown } = buildUnifiedGroups(
    [fizz],
    [first, second],
    NONE_ARCHIVED,
  );

  assert.deepEqual(groups, [{ persona: fizz, agents: [first, second] }]);
  assert.deepEqual(ungrouped, []);
  assert.deepEqual(unknown, []);
});

test("profileAgentsForGroup returns every instance in stable order without mutating input", () => {
  const stopped = agent("a".repeat(64), { name: "Zulu" });
  const runningLater = agent("c".repeat(64), {
    name: "Alpha",
    status: "running",
  });
  const runningEarlier = agent("b".repeat(64), {
    name: "Alpha",
    status: "running",
  });
  const input = [stopped, runningLater, runningEarlier];

  assert.deepEqual(profileAgentsForGroup(input, NONE_ARCHIVED), [
    runningEarlier,
    runningLater,
    stopped,
  ]);
  assert.deepEqual(input, [stopped, runningLater, runningEarlier]);
});

test("a relay-restored persona instance follows the same visible group path", () => {
  const relayRestored = agent("c".repeat(64), {
    name: "Recovered Fizz",
    status: "stopped",
  });

  const { groups } = buildUnifiedGroups([fizz], [relayRestored], NONE_ARCHIVED);

  assert.deepEqual(profileAgentsForGroup(groups[0].agents, NONE_ARCHIVED), [
    relayRestored,
  ]);
});

test("a persona with no managed instance remains an empty group", () => {
  const { groups } = buildUnifiedGroups([fizz], [], NONE_ARCHIVED);

  assert.deepEqual(groups, [{ persona: fizz, agents: [] }]);
  assert.deepEqual(profileAgentsForGroup(groups[0].agents, NONE_ARCHIVED), []);
});

test("profileAgentsForGroup omits archived instances while preserving visible peers", () => {
  const archived = agent("a".repeat(64), { status: "running" });
  const visible = agent("b".repeat(64), { status: "stopped" });

  assert.deepEqual(
    profileAgentsForGroup(
      [archived, visible],
      (pubkey) => pubkey === archived.pubkey,
    ),
    [visible],
  );
});

test("archived standalone custom agents are omitted while live peers remain", () => {
  const archived = agent("a".repeat(64), { personaId: null });
  const live = agent("b".repeat(64), { personaId: null });
  const isArchived = (pubkey) => pubkey === archived.pubkey;

  const { ungrouped } = buildUnifiedGroups([], [archived, live], isArchived);

  assert.deepEqual(
    ungrouped.map((candidate) => candidate.pubkey),
    [live.pubkey],
  );
});

test("archived unknown-persona agents are omitted while live peers remain", () => {
  const archived = agent("a".repeat(64), { personaId: "orphan" });
  const live = agent("b".repeat(64), { personaId: "orphan" });
  const isArchived = (pubkey) => pubkey === archived.pubkey;

  const { unknown } = buildUnifiedGroups([], [archived, live], isArchived);

  assert.deepEqual(
    unknown.map((candidate) => candidate.pubkey),
    [live.pubkey],
  );
});

test("matched persona groups retain archived instances for profile history", () => {
  const archived = agent("a".repeat(64));
  const live = agent("b".repeat(64));
  const isArchived = (pubkey) => pubkey === archived.pubkey;

  const { groups } = buildUnifiedGroups([fizz], [archived, live], isArchived);

  assert.deepEqual(groups, [{ persona: fizz, agents: [archived, live] }]);
  assert.deepEqual(profileAgentsForGroup(groups[0].agents, isArchived), [live]);
});

test("a fail-open predicate keeps every standalone agent discoverable", () => {
  const first = agent("a".repeat(64), { personaId: null });
  const second = agent("b".repeat(64), { personaId: null });

  const { ungrouped } = buildUnifiedGroups([], [first, second], NONE_ARCHIVED);

  assert.equal(ungrouped.length, 2);
});
