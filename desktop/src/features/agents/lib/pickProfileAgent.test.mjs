import assert from "node:assert/strict";
import test from "node:test";

import {
  pickDirectProfileAgent,
  pickProfileAgent,
} from "./pickProfileAgent.ts";

const NONE_ARCHIVED = () => false;

function agent(overrides = {}) {
  return {
    name: "Instance",
    pubkey: "a".repeat(64),
    status: "stopped",
    ...overrides,
  };
}

test("the shared profile target prefers the active persona instance", () => {
  const stopped = agent({
    name: "Earlier instance",
    pubkey: "a".repeat(64),
    status: "stopped",
  });
  const running = agent({
    name: "Current instance",
    pubkey: "b".repeat(64),
    status: "running",
  });

  assert.equal(pickProfileAgent([stopped, running], NONE_ARCHIVED), running);
  assert.equal(pickProfileAgent([running, stopped], NONE_ARCHIVED), running);
});

test("an archived instance early in file order cannot hijack the target", () => {
  const archived = agent({
    name: "Archived instance",
    pubkey: "a".repeat(64),
    status: "running",
  });
  const live = agent({
    name: "Live instance",
    pubkey: "b".repeat(64),
    status: "stopped",
  });
  const isArchived = (pubkey) => pubkey === archived.pubkey;

  // Archived is active AND first — without the filter it would win the sort.
  assert.equal(pickProfileAgent([archived, live], isArchived), live);
  assert.equal(pickProfileAgent([live, archived], isArchived), live);
});

test("all instances archived yields undefined for persona-only mode", () => {
  const first = agent({ pubkey: "a".repeat(64) });
  const second = agent({ pubkey: "b".repeat(64) });

  assert.equal(
    pickProfileAgent([first, second], () => true),
    undefined,
  );
});

test("a fail-open predicate keeps every instance eligible while loading", () => {
  const stopped = agent({ pubkey: "a".repeat(64), status: "stopped" });
  const running = agent({ pubkey: "b".repeat(64), status: "running" });

  // Fail-open (all false) during the archive-snapshot window: normal ranking.
  assert.equal(pickProfileAgent([stopped, running], NONE_ARCHIVED), running);
});

test("a direct-opened active instance is never redirected to a sibling", () => {
  // "Alpha Sibling" sorts before "Tyler Agent"; without the direct guard an
  // access edit on Tyler would target the sibling.
  const sibling = {
    name: "Alpha Sibling",
    pubkey: "a".repeat(64),
    status: "running",
  };
  const clicked = {
    name: "Tyler Agent",
    pubkey: "b".repeat(64),
    status: "running",
  };

  assert.equal(
    pickDirectProfileAgent(clicked, [sibling, clicked], NONE_ARCHIVED),
    clicked,
  );
});

test("a direct-opened inactive instance redirects to the active sibling", () => {
  const historical = {
    name: "Earlier Parity Agent",
    pubkey: "a".repeat(64),
    status: "stopped",
  };
  const current = {
    name: "Current Parity Agent",
    pubkey: "b".repeat(64),
    status: "running",
  };

  assert.equal(
    pickDirectProfileAgent(historical, [historical, current], NONE_ARCHIVED),
    current,
  );
});

test("a direct-opened inactive instance with no active sibling stays put", () => {
  const clicked = {
    name: "Only Instance",
    pubkey: "a".repeat(64),
    status: "stopped",
  };
  const otherStopped = {
    name: "Another Stopped",
    pubkey: "b".repeat(64),
    status: "stopped",
  };

  assert.equal(
    pickDirectProfileAgent(clicked, [clicked, otherStopped], NONE_ARCHIVED),
    clicked,
  );
  assert.equal(pickDirectProfileAgent(clicked, [], NONE_ARCHIVED), clicked);
});
