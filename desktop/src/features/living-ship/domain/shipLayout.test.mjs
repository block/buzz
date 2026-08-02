import assert from "node:assert/strict";
import test from "node:test";

import {
  LIVING_SHIP_ADVISERS,
  SHIP_ROOMS,
  resolveAgentLocation,
} from "./shipLayout.ts";

test("pins the approved room geometry and all eight adviser home rooms", () => {
  assert.deepEqual(
    SHIP_ROOMS.map(({ id, zone, row, column }) => ({ id, zone, row, column })),
    [
      { id: "dse", zone: "aft", row: 0, column: 0 },
      { id: "plans", zone: "aft", row: 1, column: 0 },
      { id: "cic", zone: "forward", row: 0, column: 0 },
      { id: "chart-house", zone: "forward", row: 0, column: 1 },
      { id: "wardroom", zone: "forward", row: 1, column: 0 },
      { id: "meeting-room", zone: "forward", row: 1, column: 1 },
      { id: "ships-office", zone: "forward", row: 2, column: 0 },
      { id: "supply-office", zone: "forward", row: 2, column: 1 },
    ],
  );

  assert.deepEqual(
    Object.fromEntries(
      LIVING_SHIP_ADVISERS.map(({ adviser, homeRoom }) => [adviser, homeRoom]),
    ),
    {
      chief_of_staff: "meeting-room",
      operations: "cic",
      intelligence: "dse",
      logistics: "supply-office",
      navigation: "chart-house",
      daily_routine: "ships-office",
      reporting: "ships-office",
      plans: "plans",
    },
  );
});

test("places unavailable agents off ship and confirmed idle agents in the Wardroom", () => {
  assert.equal(
    resolveAgentLocation({
      adviser: "operations",
      lifecycle: "offline",
      working: false,
    }).locationId,
    "personnel-strip",
  );
  assert.equal(
    resolveAgentLocation({
      adviser: "operations",
      lifecycle: "online",
      working: false,
    }).locationId,
    "wardroom",
  );
  assert.equal(
    resolveAgentLocation({
      adviser: "operations",
      lifecycle: "online",
      working: true,
    }).locationId,
    "cic",
  );
});

test("explicit collaboration workspace wins over deterministic context", () => {
  const result = resolveAgentLocation({
    adviser: "intelligence",
    lifecycle: "online",
    working: true,
    collaboration: {
      id: "collab-1",
      workspace: "chart-house",
      context: "operations",
    },
  });
  assert.equal(result.locationId, "chart-house");
  assert.equal(result.reason, "collaboration-explicit");
});

test("collaboration contexts map deterministically without channel inference", () => {
  const cases = {
    operations: "cic",
    intelligence: "cic",
    navigation: "chart-house",
    command: "meeting-room",
    planning: "meeting-room",
    reporting: "ships-office",
    routine: "ships-office",
    logistics: "supply-office",
  };

  for (const [context, expected] of Object.entries(cases)) {
    assert.equal(
      resolveAgentLocation({
        adviser: "chief_of_staff",
        lifecycle: "online",
        working: true,
        collaboration: { id: `collab-${context}`, context },
      }).locationId,
      expected,
      context,
    );
  }

  assert.equal(
    resolveAgentLocation({
      adviser: "chief_of_staff",
      lifecycle: "online",
      working: true,
    }).reason,
    "working-home",
  );
});
