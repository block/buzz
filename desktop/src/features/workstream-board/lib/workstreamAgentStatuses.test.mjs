import assert from "node:assert/strict";
import test from "node:test";

import { classifyWorkstreamAgents } from "./workstreamAgentStatuses.ts";

const A = "a".repeat(64);
const B = "b".repeat(64);
const C = "c".repeat(64);
const assignees = [
  { pubkey: A, name: "Active" },
  { pubkey: B, name: "Blocked" },
  { pubkey: C, name: "Unknown" },
];

test("classifies only evidence-backed working and waiting states", () => {
  assert.deepEqual(
    classifyWorkstreamAgents(
      assignees,
      { channelId: "channel", agentCount: 1, agentPubkeys: [A], anchorAt: 1 },
      [
        {
          key: "blocked",
          actor: { kind: "agent", pubkey: B.toUpperCase(), name: "Blocked" },
          reason: "Needs a decision",
          source: "manual",
        },
      ],
    ),
    { working: [assignees[0]], waiting: [assignees[1]] },
  );
});

test("working wins when the same assigned agent also has a blocker", () => {
  const result = classifyWorkstreamAgents(
    assignees,
    { channelId: "channel", agentCount: 1, agentPubkeys: [B], anchorAt: 1 },
    [
      {
        key: "blocked",
        actor: { kind: "agent", pubkey: B, name: "Blocked" },
        reason: "Needs a decision",
        source: "manual",
      },
    ],
  );
  assert.deepEqual(result.working, [assignees[1]]);
  assert.deepEqual(result.waiting, []);
});

test("does not assert idle or waiting for non-active assignees without named blockers", () => {
  assert.deepEqual(classifyWorkstreamAgents(assignees, undefined, []), {
    working: [],
    waiting: [],
  });
});
