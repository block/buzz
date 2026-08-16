import assert from "node:assert/strict";
import test from "node:test";

import {
  pickCanonicalProfileAgent,
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

test("opening a renamed instance opens that instance, not the persona's", () => {
  const claude = {
    name: "Claude",
    personaId: "builtin:fizz",
    pubkey: "a".repeat(64),
    status: "running",
  };
  const fizz = {
    name: "Fizz",
    personaId: "builtin:fizz",
    pubkey: "b".repeat(64),
    status: "stopped",
  };
  const instances = [claude, fizz];

  assert.equal(
    pickCanonicalProfileAgent(instances, fizz, NONE_ARCHIVED),
    fizz,
  );
  assert.equal(
    pickCanonicalProfileAgent(instances, claude, NONE_ARCHIVED),
    claude,
  );
});

test("same-named instances still canonicalise onto one profile target", () => {
  const stopped = {
    name: "Bumble",
    personaId: "builtin:bumble",
    pubkey: "a".repeat(64),
    status: "stopped",
  };
  const running = {
    name: "Bumble",
    personaId: "builtin:bumble",
    pubkey: "b".repeat(64),
    status: "running",
  };
  const instances = [stopped, running];

  assert.equal(
    pickCanonicalProfileAgent(instances, stopped, NONE_ARCHIVED),
    running,
  );
  assert.equal(
    pickCanonicalProfileAgent(instances, undefined, NONE_ARCHIVED),
    running,
  );
});

test("a fully archived display group falls back to the persona's live instance", () => {
  // Scoping runs before archive filtering, so a group whose every instance is
  // archived must not resolve to nothing where the unscoped selector would
  // still have found a live sibling.
  const archivedFizz = {
    name: "Fizz",
    personaId: "builtin:fizz",
    pubkey: "a".repeat(64),
    status: "stopped",
  };
  const liveClaude = {
    name: "Claude",
    personaId: "builtin:fizz",
    pubkey: "b".repeat(64),
    status: "running",
  };
  const isArchived = (pubkey) => pubkey === archivedFizz.pubkey;

  assert.equal(
    pickCanonicalProfileAgent(
      [archivedFizz, liveClaude],
      archivedFizz,
      isArchived,
    ),
    liveClaude,
  );
});

test("a requested instance survives when every candidate is archived", () => {
  const requested = {
    name: "Fizz",
    personaId: "builtin:fizz",
    pubkey: "a".repeat(64),
    status: "stopped",
  };

  assert.equal(
    pickCanonicalProfileAgent([requested], requested, () => true),
    requested,
  );
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
  // The avatar on an old message points at a retired instance. Both carry the
  // label the Agents library shows, so the retired one is not a card of its
  // own and redirecting to the live instance matches what the library renders.
  const historical = {
    name: "Parity Agent",
    personaId: "builtin:parity",
    pubkey: "a".repeat(64),
    status: "stopped",
  };
  const current = {
    name: "Parity Agent",
    personaId: "builtin:parity",
    pubkey: "b".repeat(64),
    status: "running",
  };

  assert.equal(
    pickDirectProfileAgent(historical, [historical, current], NONE_ARCHIVED),
    current,
  );
});

test("a direct-opened inactive instance never redirects across a rename", () => {
  // The owner renamed one instance, so the library shows two cards. Redirecting
  // the retired card to the differently named live instance would reopen the
  // bug where a card refuses to open the agent it names.
  const renamed = {
    name: "Fizz",
    personaId: "builtin:fizz",
    pubkey: "a".repeat(64),
    status: "stopped",
  };
  const current = {
    name: "Claude",
    personaId: "builtin:fizz",
    pubkey: "b".repeat(64),
    status: "running",
  };

  assert.equal(
    pickDirectProfileAgent(renamed, [renamed, current], NONE_ARCHIVED),
    renamed,
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
