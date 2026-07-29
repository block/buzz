import assert from "node:assert/strict";
import test from "node:test";

import { buildUnifiedGroups, pickProfileAgent } from "./unifiedAgentGroups.ts";

function agent(overrides = {}) {
  return {
    pubkey: "a".repeat(64),
    name: "agent",
    personaId: "persona-1",
    status: "stopped",
    backend: { type: "local" },
    ...overrides,
  };
}

const provider = { type: "provider", id: "ssh", config: {} };

// ── pickProfileAgent ────────────────────────────────────────────────────────

test("an active agent outranks an idle one", () => {
  const running = agent({ name: "zulu", status: "running" });
  const picked = pickProfileAgent([agent({ name: "alpha" }), running]);
  assert.equal(picked, running);
});

// The defect: with both records idle the tiebreak was name order alone, so a
// local record could win the card for a group whose real agent runs on a host.
// The two disagree about where the agent runs — the local record's harness
// comes from this computer's catalog — so the card and its edit dialog would
// describe a machine the user is not looking at.
test("a provider-backed record wins the card over a local one", () => {
  const remote = agent({ name: "zulu", backend: provider });
  const picked = pickProfileAgent([agent({ name: "alpha" }), remote]);
  assert.equal(picked, remote, "the remote record owns the group's identity");
});

test("active still outranks provider-backed", () => {
  const runningLocal = agent({ name: "zulu", status: "running" });
  const picked = pickProfileAgent([
    agent({ name: "alpha", backend: provider }),
    runningLocal,
  ]);
  assert.equal(picked, runningLocal);
});

test("name breaks a tie between two records of the same rank", () => {
  const picked = pickProfileAgent([
    agent({ name: "zulu", backend: provider }),
    agent({ name: "alpha", backend: provider }),
  ]);
  assert.equal(picked.name, "alpha");
});

// ── buildUnifiedGroups ──────────────────────────────────────────────────────

test("agents group by persona, keyless and unknown-persona ones split off", () => {
  const linked = agent({ name: "linked" });
  const loose = agent({ name: "loose", personaId: null });
  const orphan = agent({ name: "orphan", personaId: "gone" });
  const { groups, ungrouped, unknown } = buildUnifiedGroups(
    [{ id: "persona-1" }],
    [linked, loose, orphan],
  );

  assert.deepEqual(
    groups.map((group) => group.agents),
    [[linked]],
  );
  assert.deepEqual(ungrouped, [loose]);
  assert.deepEqual(unknown, [orphan]);
});
