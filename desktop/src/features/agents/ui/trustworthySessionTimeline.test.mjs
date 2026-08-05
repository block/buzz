import assert from "node:assert/strict";
import test from "node:test";

import {
  buildTrustworthySessionTimeline,
  explainSession,
} from "./trustworthySessionTimeline.ts";

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

test("explains a failed session before exposing raw telemetry", () => {
  const explanation = explainSession(
    [
      {
        ...base,
        seq: 1,
        timestamp: "2026-08-05T12:00:00Z",
        kind: "turn_started",
        payload: {},
      },
      {
        ...base,
        seq: 2,
        timestamp: "2026-08-05T12:00:02Z",
        kind: "turn_error",
        payload: { error: "cargo build exited 101" },
      },
    ],
    [],
  );

  assert.equal(explanation.outcome, "failed");
  assert.equal(explanation.why, "cargo build exited 101");
  assert.equal(explanation.confidence, "medium");
  assert.match(explanation.nextAction, /failed event/i);
  assert.equal(explanation.unknowns.length, 2);
});

test("does not invent a cause or fix when evidence is incomplete", () => {
  const explanation = explainSession(
    [
      {
        ...base,
        seq: 1,
        timestamp: "2026-08-05T12:00:00Z",
        kind: "turn_started",
        payload: {},
      },
    ],
    [],
  );

  assert.equal(explanation.outcome, "in_progress");
  assert.match(explanation.why, /still running/i);
  assert.equal(explanation.confidence, "low");
  assert.match(explanation.nextAction, /missing outcome evidence/i);
});

test("models the real host-write incident and validates one controlled replay", () => {
  const failedRun = [
    {
      ...base,
      seq: 1,
      timestamp: "2026-08-05T12:00:00Z",
      kind: "task_captured",
      payload: {
        description:
          "Write the requested file and ask before changing the workspace",
        sourceMessageId: "host-write-request",
      },
    },
    {
      ...base,
      seq: 2,
      timestamp: "2026-08-05T12:00:01Z",
      kind: "host_operation",
      payload: {
        operation: "Wrote",
        target: "workspace file",
        executionPath: "host tool path outside ACP",
        observedBy: "Codex host",
      },
    },
    {
      ...base,
      seq: 3,
      timestamp: "2026-08-05T12:00:02Z",
      kind: "causal_hypothesis",
      payload: {
        title: "Host write bypassed the ACP permission gate",
        summary:
          "No permission appeared because the host tool performed the write outside the ACP execution path.",
        confidence: "high",
        evidenceIds: ["observer:session-1:2"],
      },
    },
    {
      ...base,
      seq: 4,
      timestamp: "2026-08-05T12:00:03Z",
      kind: "turn_completed",
      payload: {},
    },
    {
      ...base,
      seq: 5,
      timestamp: "2026-08-05T12:00:04Z",
      kind: "remediation_proposed",
      payload: {
        summary: "Route workspace writes through the governed ACP tool path",
        controlledChange: "execution path: host tool → ACP workspace tool",
      },
    },
  ];

  const beforeReplay = explainSession(failedRun, []);
  assert.equal(
    beforeReplay.task,
    "Write the requested file and ask before changing the workspace",
  );
  assert.match(beforeReplay.why, /outside the ACP execution path/i);
  assert.equal(beforeReplay.remediation.status, "not_tested");
  assert.match(beforeReplay.nextAction, /host tool → ACP workspace tool/i);
  assert.equal(
    beforeReplay.unknowns.some((gap) => gap.sourceLayer === "host_workspace"),
    false,
  );
  assert.equal(
    beforeReplay.unknowns.some((gap) => gap.sourceLayer === "os_sandbox"),
    true,
  );

  const afterReplay = explainSession(
    [
      ...failedRun,
      {
        ...base,
        sessionId: "session-replay-1",
        turnId: "turn-replay-1",
        seq: 6,
        timestamp: "2026-08-05T12:01:00Z",
        kind: "permission_decision",
        payload: { mode: "ask", decision: "allow_once" },
      },
      {
        ...base,
        sessionId: "session-replay-1",
        turnId: "turn-replay-1",
        seq: 7,
        timestamp: "2026-08-05T12:01:01Z",
        kind: "replay_result",
        payload: {
          outcome: "succeeded",
          expectedCauseObserved: true,
          validatesRemedyId: "observer:session-1:5",
          summary:
            "The governed path requested permission before the write and the task succeeded.",
        },
      },
    ],
    [],
  );

  assert.equal(afterReplay.remediation.status, "validated");
  assert.match(afterReplay.remediation.result, /requested permission/i);
  assert.match(afterReplay.nextAction, /validated governed execution path/i);
});

test("does not validate a remedy from an unrelated or permission-free replay", () => {
  const failedRun = [
    {
      ...base,
      seq: 1,
      kind: "remediation_proposed",
      payload: {
        summary: "Route writes through ACP",
        controlledChange: "host tool → ACP workspace tool",
      },
    },
  ];
  const successfulButUnlinkedReplay = {
    ...base,
    sessionId: "session-replay-2",
    turnId: "turn-replay-2",
    seq: 2,
    kind: "replay_result",
    payload: {
      outcome: "succeeded",
      expectedCauseObserved: true,
      validatesRemedyId: "observer:some-other-session:1",
      summary: "An unrelated replay succeeded.",
    },
  };

  const unlinked = explainSession(
    [...failedRun, successfulButUnlinkedReplay],
    [],
  );
  assert.equal(unlinked.remediation.status, "not_tested");

  const linkedWithoutPermission = explainSession(
    [
      ...failedRun,
      {
        ...successfulButUnlinkedReplay,
        payload: {
          ...successfulButUnlinkedReplay.payload,
          validatesRemedyId: "observer:session-1:1",
        },
      },
    ],
    [],
  );
  assert.equal(linkedWithoutPermission.remediation.status, "inconclusive");
});
