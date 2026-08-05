import assert from "node:assert/strict";
import test from "node:test";

import { buildTrustworthySessionTimeline } from "./trustworthySessionTimeline.ts";

const base = {
  agentIndex: 0,
  channelId: "channel-1",
  sessionId: "session-1",
  turnId: "turn-1",
};

test("projects enforcement provenance and explicit uncovered layers", () => {
  const events = buildTrustworthySessionTimeline(
    [
      {
        ...base,
        seq: 1,
        timestamp: "2026-08-05T12:00:00Z",
        kind: "permission_decision",
        payload: { mode: "lockdown", decision: "reject_once" },
      },
    ],
    [],
  );

  const decision = events.find((event) => event.class === "decision");
  assert.equal(decision.title, "Tool request denied");
  assert.equal(decision.sourceSystem, "Buzz ACP");
  assert.equal(decision.observerRole, "enforced");
  assert.equal(decision.confidence, "direct");

  assert.deepEqual(
    events
      .filter((event) => event.class === "gap")
      .map((event) => event.sourceLayer),
    ["host_workspace", "os_sandbox"],
  );
});

test("keeps Numbat conclusions distinct from directly observed facts", () => {
  const events = buildTrustworthySessionTimeline(
    [],
    [
      {
        findingId: "finding-1",
        ruleId: "guardian.suspicious-write",
        title: "Suspicious write",
        severity: "high",
        detectedAt: "2026-08-05T12:00:01Z",
        sourceAgent: "agent-1",
        sessionId: "session-1",
        channelId: "channel-1",
        turnId: "turn-1",
        evidenceCount: 2,
      },
    ],
  );

  const finding = events.find((event) => event.sourceSystem === "Numbat");
  assert.equal(finding.class, "inference");
  assert.equal(finding.observerRole, "inferred");
  assert.equal(finding.confidence, "correlated");
});

test("does not invent gaps before a session has any evidence", () => {
  assert.deepEqual(buildTrustworthySessionTimeline([], []), []);
});
