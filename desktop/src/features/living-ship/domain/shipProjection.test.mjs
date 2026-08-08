import assert from "node:assert/strict";
import test from "node:test";

import { projectLivingShipAgents } from "./shipProjection.ts";

const managed = [
  {
    pubkey: "cos-key",
    name: "CoS",
    personaId: "builtin:command-chief-of-staff",
    status: "running",
  },
  {
    pubkey: "ops-key",
    name: "Operations",
    personaId: "builtin:command-operations",
    status: "running",
  },
  {
    pubkey: "n2-key",
    name: "N2",
    personaId: "builtin:command-intelligence",
    status: "stopped",
  },
  {
    pubkey: "other-key",
    name: "Other",
    personaId: "builtin:fizz",
    status: "running",
  },
];

test("projects only command personas into stable ship presentation records", () => {
  const result = projectLivingShipAgents({
    managedAgents: managed,
    channels: [{ id: "ops-channel", name: "operations" }],
    workingByPubkey: new Map([
      ["cos-key", { working: false, channels: [] }],
      [
        "ops-key",
        {
          working: true,
          channels: [
            { channelId: "ops-channel", anchorAt: 100, source: "observer" },
          ],
        },
      ],
      ["n2-key", { working: false, channels: [] }],
    ]),
    observerEventsByPubkey: new Map(),
  });

  assert.deepEqual(
    result.map(({ adviser, locationId, lifecycle, channelName }) => ({
      adviser,
      locationId,
      lifecycle,
      channelName,
    })),
    [
      {
        adviser: "chief_of_staff",
        locationId: "wardroom",
        lifecycle: "online",
        channelName: null,
      },
      {
        adviser: "operations",
        locationId: "cic",
        lifecycle: "online",
        channelName: "operations",
      },
      {
        adviser: "intelligence",
        locationId: "personnel-strip",
        lifecycle: "offline",
        channelName: null,
      },
    ],
  );
});

test("groups collaborators only from explicit observer collaboration IDs", () => {
  const commonFrame = {
    seq: 1,
    timestamp: "2026-08-02T02:00:00Z",
    kind: "turn_started",
    agentIndex: 0,
    channelId: "ops-channel",
    sessionId: null,
    turnId: "turn-1",
    payload: {
      collaborationId: "brief-17",
      workspace: "meeting-room",
      context: "command",
      participantPubkeys: ["cos-key", "ops-key"],
      summary: "Preparing the daily command brief",
    },
  };
  const result = projectLivingShipAgents({
    managedAgents: managed,
    channels: [{ id: "ops-channel", name: "operations" }],
    workingByPubkey: new Map([
      [
        "cos-key",
        {
          working: true,
          channels: [
            { channelId: "ops-channel", anchorAt: 100, source: "observer" },
          ],
        },
      ],
      [
        "ops-key",
        {
          working: true,
          channels: [
            { channelId: "ops-channel", anchorAt: 100, source: "observer" },
          ],
        },
      ],
    ]),
    observerEventsByPubkey: new Map([
      ["cos-key", [commonFrame]],
      ["ops-key", [{ ...commonFrame, seq: 2, turnId: "turn-2" }]],
    ]),
  });

  assert.equal(result[0].locationId, "meeting-room");
  assert.equal(result[1].locationId, "meeting-room");
  assert.deepEqual(result[0].collaboratorPubkeys, ["ops-key"]);
  assert.deepEqual(result[1].collaboratorPubkeys, ["cos-key"]);
  assert.equal(result[0].taskSummary, "Preparing the daily command brief");
});

test("does not call agents collaborators merely because they share a channel", () => {
  const result = projectLivingShipAgents({
    managedAgents: managed.slice(0, 2),
    channels: [{ id: "ops-channel", name: "operations" }],
    workingByPubkey: new Map(
      ["cos-key", "ops-key"].map((pubkey) => [
        pubkey,
        {
          working: true,
          channels: [
            { channelId: "ops-channel", anchorAt: 100, source: "observer" },
          ],
        },
      ]),
    ),
    observerEventsByPubkey: new Map(),
  });

  assert.deepEqual(
    result.map((agent) => agent.collaboratorPubkeys),
    [[], []],
  );
});
