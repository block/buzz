import assert from "node:assert/strict";
import test from "node:test";

import { resolveCanonicalManagedAgent } from "./useCanonicalManagedAgentProfile.ts";

const LIVE_PK = "b".repeat(64);
const ARCHIVED_PK = "a".repeat(64);
const HISTORICAL_PK = "c".repeat(64);
const NONE_ARCHIVED = () => false;

function agent(overrides = {}) {
  return {
    name: "Instance",
    pubkey: LIVE_PK,
    personaId: "persona-1",
    status: "stopped",
    ...overrides,
  };
}

test("a persona target with a live sibling resolves to the live instance", () => {
  const archived = agent({ pubkey: ARCHIVED_PK, status: "running" });
  const live = agent({ pubkey: LIVE_PK, status: "stopped" });

  const resolved = resolveCanonicalManagedAgent({
    directManagedAgent: undefined,
    isArchived: (pubkey) => pubkey === ARCHIVED_PK,
    personaInstances: [archived, live],
    pubkey: undefined,
  });

  assert.equal(resolved, live);
});

test("a persona target with all instances archived resolves to undefined", () => {
  const first = agent({ pubkey: ARCHIVED_PK });
  const second = agent({ pubkey: HISTORICAL_PK });

  const resolved = resolveCanonicalManagedAgent({
    directManagedAgent: undefined,
    isArchived: () => true,
    personaInstances: [first, second],
    pubkey: undefined,
  });

  assert.equal(resolved, undefined);
});

test("an explicit archived pubkey stays exact even when a live sibling exists", () => {
  const archivedDirect = agent({ pubkey: ARCHIVED_PK });
  const live = agent({ pubkey: LIVE_PK, status: "running" });

  const resolved = resolveCanonicalManagedAgent({
    directManagedAgent: archivedDirect,
    isArchived: (pubkey) => pubkey === ARCHIVED_PK,
    personaInstances: [archivedDirect, live],
    pubkey: ARCHIVED_PK,
  });

  // Without the exactness short-circuit the selector would drop the archived
  // record and return `live`, stranding the unarchive controller.
  assert.equal(resolved, archivedDirect);
});

test("an explicit archived pubkey with no managed record resolves to undefined so the panel keeps the requested key", () => {
  // A historical archived pubkey with no current managed record: directManaged
  // is undefined, and the panel falls back to the requested pubkey verbatim.
  const resolved = resolveCanonicalManagedAgent({
    directManagedAgent: undefined,
    isArchived: (pubkey) => pubkey === HISTORICAL_PK,
    personaInstances: [],
    pubkey: HISTORICAL_PK,
  });

  assert.equal(resolved, undefined);
});

test("an explicit stopped instance pins the exact record over canonicalization", () => {
  const requested = agent({ pubkey: HISTORICAL_PK, status: "stopped" });
  const live = agent({ pubkey: LIVE_PK, status: "running" });

  const resolved = resolveCanonicalManagedAgent({
    directManagedAgent: requested,
    isArchived: NONE_ARCHIVED,
    personaInstances: [requested, live],
    pubkey: HISTORICAL_PK,
  });

  assert.equal(resolved, requested);
});

test("a non-archived historical pubkey stays exact, like archived and active keys", () => {
  // History is authored by a key, not its current persona representative.
  const requested = agent({ pubkey: HISTORICAL_PK, status: "stopped" });
  const live = agent({ pubkey: LIVE_PK, status: "running" });

  const resolved = resolveCanonicalManagedAgent({
    directManagedAgent: requested,
    isArchived: NONE_ARCHIVED,
    personaInstances: [requested, live],
    pubkey: HISTORICAL_PK,
  });

  assert.equal(resolved, requested);
});

test("explicit navigation keeps a directly opened active instance exact", () => {
  // The panel's own default: an access edit must target the clicked instance,
  // not an alphabetically-earlier active sibling.
  const sibling = agent({ name: "Alpha", pubkey: LIVE_PK, status: "running" });
  const clicked = agent({
    name: "Zulu",
    pubkey: HISTORICAL_PK,
    status: "running",
  });

  const resolved = resolveCanonicalManagedAgent({
    directManagedAgent: clicked,
    isArchived: NONE_ARCHIVED,
    personaInstances: [sibling, clicked],
    pubkey: HISTORICAL_PK,
  });

  assert.equal(resolved, clicked);
});

test("an explicit archived pubkey stays exact with a running sibling", () => {
  // Archive state does not change what identity an explicit target names.
  const archivedDirect = agent({ pubkey: ARCHIVED_PK, status: "stopped" });
  const live = agent({ pubkey: LIVE_PK, status: "running" });

  const resolved = resolveCanonicalManagedAgent({
    directManagedAgent: archivedDirect,
    isArchived: (pubkey) => pubkey === ARCHIVED_PK,
    personaInstances: [archivedDirect, live],
    pubkey: ARCHIVED_PK,
  });

  assert.equal(resolved, archivedDirect);
});

for (const instances of [[], [agent({ status: "running" })]]) {
  test(`relay-only A never resolves to local sibling B (${instances.length} siblings)`, () => {
    assert.equal(
      resolveCanonicalManagedAgent({
        directManagedAgent: undefined,
        isArchived: NONE_ARCHIVED,
        personaInstances: instances,
        pubkey: HISTORICAL_PK,
      }),
      undefined,
    );
  });
}
