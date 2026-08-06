import type { ObserverEvent } from "./agentSessionTypes";
import {
  extractPromptText,
  extractToolResult,
  parsePromptText,
} from "./agentSessionTranscriptHelpers";
import { asRecord, asString } from "./agentSessionUtils";

export type CausalFinding = {
  findingId: string;
  detectedAt: string;
  title: string;
  ruleId: string;
  evidenceCount: number;
  sessionId: string | null;
  turnId: string | null;
};

export type CausalEventClass = "fact" | "decision" | "inference" | "gap";
export type CausalEventConfidence = "direct" | "correlated" | "unknown";

export type CausalTimelineEvent = {
  id: string;
  timestamp: string;
  class: CausalEventClass;
  title: string;
  detail: string;
  sourceSystem: string;
  sourceLayer: string;
  observerRole: "observed" | "decided" | "enforced" | "inferred" | "missing";
  confidence: CausalEventConfidence;
  sessionId: string | null;
  turnId: string | null;
  evidenceIds: string[];
};

export type SessionOutcome = "succeeded" | "failed" | "in_progress" | "unknown";

export type RemediationVerification = {
  remedy: string;
  controlledChange: string;
  status: "validated" | "rejected" | "inconclusive" | "not_tested";
  result: string;
  evidenceIds: string[];
};

export type SessionExplanation = {
  task: string;
  outcome: SessionOutcome;
  why: string;
  confidence: "high" | "medium" | "low";
  evidence: CausalTimelineEvent[];
  unknowns: CausalTimelineEvent[];
  nextAction: string;
  remediation: RemediationVerification | null;
};

const REQUIRED_SOURCE_LAYERS = [
  { layer: "host_workspace", system: "Host workspace tools" },
  { layer: "os_sandbox", system: "Operating-system sandbox" },
] as const;

function objectPayload(event: ObserverEvent): Record<string, unknown> {
  return event.payload && typeof event.payload === "object"
    ? (event.payload as Record<string, unknown>)
    : {};
}

function stringValue(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function stringValues(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((entry): entry is string => typeof entry === "string")
    : [];
}

function observerEventId(event: ObserverEvent): string {
  return `observer:${event.sessionId ?? "unknown"}:${event.seq}`;
}

function latestTurnEvents(
  observerEvents: readonly ObserverEvent[],
): ObserverEvent[] {
  const latestTurn = [...observerEvents]
    .reverse()
    .find((event) => event.turnId);
  if (!latestTurn?.turnId) return observerEvents as ObserverEvent[];
  return observerEvents.filter(
    (event) =>
      event.turnId === latestTurn.turnId &&
      (!latestTurn.sessionId || event.sessionId === latestTurn.sessionId),
  );
}

function acpMethod(event: ObserverEvent): string | null {
  return asString(objectPayload(event).method);
}

function acpUpdate(event: ObserverEvent): Record<string, unknown> {
  const payload = objectPayload(event);
  return asRecord(asRecord(payload.params).update);
}

function permissionDecisionFromRawEvent(
  event: ObserverEvent,
): { decision: string; denied: boolean } | null {
  if (event.kind !== "acp_write" || acpMethod(event)) return null;
  const result = asRecord(asRecord(objectPayload(event).result).outcome);
  const outcome = asString(result.outcome);
  if (!outcome) return null;
  const optionId = asString(result.optionId);
  const decision = optionId ?? outcome;
  if (outcome !== "cancelled" && !/allow|reject|deny|cancel/.test(decision)) {
    return null;
  }
  return {
    decision,
    denied:
      outcome === "cancelled" ||
      decision.includes("reject") ||
      decision.includes("deny") ||
      decision.includes("cancel"),
  };
}

function toolResultFromRawEvent(
  event: ObserverEvent,
  observerEvents: readonly ObserverEvent[],
): { detail: string; failed: boolean } | null {
  if (event.kind !== "acp_read" || acpMethod(event) !== "session/update") {
    return null;
  }
  const update = acpUpdate(event);
  if (asString(update.sessionUpdate) !== "tool_call_update") return null;
  const status = asString(update.status);
  if (status !== "completed" && status !== "failed") return null;
  const toolCallId = asString(update.toolCallId);
  const earlierUpdate = toolCallId
    ? [...observerEvents]
        .reverse()
        .map((candidate) => acpUpdate(candidate))
        .find(
          (candidateUpdate) =>
            asString(candidateUpdate.sessionUpdate) === "tool_call_update" &&
            asString(candidateUpdate.toolCallId) === toolCallId &&
            Boolean(extractToolResult(candidateUpdate)),
        )
    : null;
  const detail =
    extractToolResult(update) ||
    (earlierUpdate ? extractToolResult(earlierUpdate) : null) ||
    `Tool ${status}.`;
  return {
    detail,
    failed:
      status === "failed" ||
      /permission denied|operation not permitted/i.test(detail),
  };
}

function taskFromEvents(events: readonly ObserverEvent[]): string | null {
  const taskEvent = [...events]
    .reverse()
    .find((event) => event.kind === "task_captured");
  if (taskEvent) {
    const taskPayload = objectPayload(taskEvent);
    return (
      stringValue(taskPayload.description) ??
      (stringValue(taskPayload.sourceMessageId)
        ? `Task captured from message ${stringValue(taskPayload.sourceMessageId)}`
        : null)
    );
  }

  const promptEvent = [...events]
    .reverse()
    .find(
      (event) =>
        event.kind === "acp_write" && acpMethod(event) === "session/prompt",
    );
  if (!promptEvent) return null;
  const prompt = extractPromptText(objectPayload(promptEvent));
  if (!prompt) return null;
  return parsePromptText(prompt).userText || prompt.trim() || null;
}

function projectObserverEvent(
  event: ObserverEvent,
  observerEvents: readonly ObserverEvent[],
): CausalTimelineEvent {
  const payload = objectPayload(event);
  const eventId = observerEventId(event);

  const rawPermissionDecision = permissionDecisionFromRawEvent(event);
  if (rawPermissionDecision) {
    return {
      id: eventId,
      timestamp: event.timestamp,
      class: "decision",
      title: rawPermissionDecision.denied
        ? "Tool request denied"
        : "Tool request allowed",
      detail: `Buzz ACP answered ${rawPermissionDecision.decision}.`,
      sourceSystem: "Buzz ACP",
      sourceLayer: "acp_permission_gate",
      observerRole: "enforced",
      confidence: "direct",
      sessionId: event.sessionId,
      turnId: event.turnId,
      evidenceIds: [eventId],
    };
  }

  const rawToolResult = toolResultFromRawEvent(event, observerEvents);
  if (rawToolResult) {
    const permissionFailure =
      rawToolResult.failed &&
      /permission denied|operation not permitted/i.test(rawToolResult.detail);
    return {
      id: eventId,
      timestamp: event.timestamp,
      class: "fact",
      title: rawToolResult.failed ? "Tool failed" : "Tool completed",
      detail: rawToolResult.detail,
      sourceSystem: permissionFailure ? "Operating-system sandbox" : "Buzz ACP",
      sourceLayer: permissionFailure ? "os_sandbox" : "acp_tool_result",
      observerRole: "observed",
      confidence: "direct",
      sessionId: event.sessionId,
      turnId: event.turnId,
      evidenceIds: [eventId],
    };
  }

  if (event.kind === "host_operation") {
    const operation = stringValue(payload.operation) ?? "Host operation";
    const target = stringValue(payload.target) ?? "unknown target";
    const path = stringValue(payload.executionPath) ?? "host runtime";
    return {
      id: eventId,
      timestamp: event.timestamp,
      class: "fact",
      title: `${operation} ${target}`,
      detail: `The ${path} performed this operation.`,
      sourceSystem: stringValue(payload.observedBy) ?? "Host runtime",
      sourceLayer: "host_workspace",
      observerRole: "observed",
      confidence: "direct",
      sessionId: event.sessionId,
      turnId: event.turnId,
      evidenceIds: [eventId],
    };
  }

  if (event.kind === "causal_hypothesis") {
    return {
      id: eventId,
      timestamp: event.timestamp,
      class: "inference",
      title: stringValue(payload.title) ?? "Causal hypothesis",
      detail:
        stringValue(payload.summary) ??
        "Buzz does not have a summary for this hypothesis.",
      sourceSystem: "Buzz causal graph",
      sourceLayer: "causal_correlation",
      observerRole: "inferred",
      confidence:
        stringValue(payload.confidence) === "high" ? "correlated" : "unknown",
      sessionId: event.sessionId,
      turnId: event.turnId,
      evidenceIds: stringValues(payload.evidenceIds),
    };
  }

  if (event.kind === "permission_decision") {
    const decision = stringValue(payload.decision) ?? "unknown";
    const mode = stringValue(payload.mode) ?? "unspecified policy";
    const denied = decision.includes("reject") || decision.includes("cancel");
    return {
      id: eventId,
      timestamp: event.timestamp,
      class: "decision",
      title: denied ? "Tool request denied" : "Tool request allowed",
      detail: `Buzz ACP answered ${decision} under ${mode}.`,
      sourceSystem: "Buzz ACP",
      sourceLayer: "acp_permission_gate",
      observerRole: "enforced",
      confidence: "direct",
      sessionId: event.sessionId,
      turnId: event.turnId,
      evidenceIds: [eventId],
    };
  }

  if (event.kind === "turn_error") {
    const message =
      stringValue(payload.message) ??
      stringValue(payload.error) ??
      "No diagnostic was emitted.";
    return {
      id: eventId,
      timestamp: event.timestamp,
      class: "fact",
      title: "Turn failed",
      detail: message,
      sourceSystem: "Buzz ACP",
      sourceLayer: "acp_runtime",
      observerRole: "observed",
      confidence: "direct",
      sessionId: event.sessionId,
      turnId: event.turnId,
      evidenceIds: [eventId],
    };
  }

  return {
    id: eventId,
    timestamp: event.timestamp,
    class: "fact",
    title: event.kind.replaceAll("_", " "),
    detail: "Captured by the Buzz ACP observer.",
    sourceSystem: "Buzz ACP",
    sourceLayer: "acp_observer",
    observerRole: "observed",
    confidence: "direct",
    sessionId: event.sessionId,
    turnId: event.turnId,
    evidenceIds: [eventId],
  };
}

function projectFinding(finding: CausalFinding): CausalTimelineEvent {
  return {
    id: `numbat:${finding.findingId}`,
    timestamp: finding.detectedAt,
    class: "inference",
    title: finding.title,
    detail: `${finding.ruleId} correlated ${finding.evidenceCount} evidence event${finding.evidenceCount === 1 ? "" : "s"}.`,
    sourceSystem: "Numbat",
    sourceLayer: "guardian_detection",
    observerRole: "inferred",
    confidence: finding.evidenceCount > 0 ? "correlated" : "unknown",
    sessionId: finding.sessionId,
    turnId: finding.turnId,
    evidenceIds: [],
  };
}

export function buildTrustworthySessionTimeline(
  observerEvents: readonly ObserverEvent[],
  findings: readonly CausalFinding[],
): CausalTimelineEvent[] {
  if (observerEvents.length === 0 && findings.length === 0) return [];

  const projected = [
    ...observerEvents.map((event) =>
      projectObserverEvent(event, observerEvents),
    ),
    ...findings.map(projectFinding),
  ];
  const seenLayers = new Set(projected.map((event) => event.sourceLayer));
  const sessionId =
    projected.find((event) => event.sessionId)?.sessionId ?? null;
  const turnId = projected.find((event) => event.turnId)?.turnId ?? null;
  const gapTimestamp =
    projected
      .map((event) => event.timestamp)
      .filter(Boolean)
      .sort()[0] ?? new Date(0).toISOString();

  for (const source of REQUIRED_SOURCE_LAYERS) {
    if (seenLayers.has(source.layer)) continue;
    projected.push({
      id: `gap:${source.layer}:${sessionId ?? "unknown"}`,
      timestamp: gapTimestamp,
      class: "gap",
      title: `${source.system} telemetry unavailable`,
      detail:
        "This execution layer is not connected to the Buzz session timeline. Actions may have occurred without appearing here.",
      sourceSystem: source.system,
      sourceLayer: source.layer,
      observerRole: "missing",
      confidence: "unknown",
      sessionId,
      turnId,
      evidenceIds: [],
    });
  }

  return projected.sort((left, right) => {
    const byTime = Date.parse(left.timestamp) - Date.parse(right.timestamp);
    return byTime || left.id.localeCompare(right.id);
  });
}

export function explainSession(
  observerEvents: readonly ObserverEvent[],
  findings: readonly NumbatFinding[],
): SessionExplanation | null {
  const scopedEvents = latestTurnEvents(observerEvents);
  const scopedTurnId = scopedEvents.find((event) => event.turnId)?.turnId;
  const scopedFindings = scopedTurnId
    ? findings.filter(
        (finding) => !finding.turnId || finding.turnId === scopedTurnId,
      )
    : findings;
  const timeline = buildTrustworthySessionTimeline(
    scopedEvents,
    scopedFindings,
  );
  if (timeline.length === 0) return null;

  const latestError = [...timeline]
    .reverse()
    .find((event) => event.title === "Turn failed");
  const latestCompleted = [...scopedEvents]
    .reverse()
    .find((event) => event.kind === "turn_completed");
  const latestStarted = [...scopedEvents]
    .reverse()
    .find((event) => event.kind === "turn_started");
  const strongestFinding = [...timeline]
    .reverse()
    .find(
      (event) =>
        event.class === "inference" && event.confidence === "correlated",
    );
  const causalHypothesis = [...timeline]
    .reverse()
    .find((event) => event.sourceLayer === "causal_correlation");
  const deniedDecision = [...timeline]
    .reverse()
    .find(
      (event) =>
        event.class === "decision" && event.title === "Tool request denied",
    );
  const gaps = timeline.filter((event) => event.class === "gap");
  const facts = timeline.filter((event) => event.class !== "gap");
  const latestToolFailure = [...timeline]
    .reverse()
    .find((event) => event.title === "Tool failed");
  const latestToolSuccess = [...timeline]
    .reverse()
    .find((event) => event.title === "Tool completed");

  const outcome: SessionOutcome =
    latestError || latestToolFailure || deniedDecision
      ? "failed"
      : latestCompleted && latestToolSuccess
        ? "succeeded"
        : latestCompleted
          ? "unknown"
          : latestStarted
            ? "in_progress"
            : "unknown";
  const cause =
    causalHypothesis ??
    strongestFinding ??
    latestError ??
    latestToolFailure ??
    deniedDecision ??
    null;
  const why = cause
    ? cause.detail === "Captured by the Buzz ACP observer."
      ? cause.title
      : cause.detail
    : outcome === "in_progress"
      ? "The task is still running. Buzz does not have a final cause yet."
      : "Buzz does not have enough evidence to explain the outcome yet.";
  const confidence =
    cause?.confidence === "direct" && gaps.length === 0
      ? "high"
      : cause && cause.confidence !== "unknown"
        ? "medium"
        : "low";

  const task =
    taskFromEvents(scopedEvents) ??
    "Task description unavailable from current turn evidence";
  const remedyEvent = [...observerEvents]
    .reverse()
    .find((event) => event.kind === "remediation_proposed");
  const replayEvent = [...observerEvents]
    .reverse()
    .find((event) => event.kind === "replay_result");
  const remedyPayload = remedyEvent ? objectPayload(remedyEvent) : {};
  const replayPayload = replayEvent ? objectPayload(replayEvent) : {};
  const replayOutcome = stringValue(replayPayload.outcome);
  const expectedCauseObserved = replayPayload.expectedCauseObserved === true;
  const remedyEventId = remedyEvent ? observerEventId(remedyEvent) : null;
  const replayValidatesRemedy =
    remedyEventId !== null &&
    stringValue(replayPayload.validatesRemedyId) === remedyEventId;
  const replayRequestedPermission = replayEvent
    ? observerEvents.some(
        (event) =>
          event.kind === "permission_decision" &&
          event.sessionId === replayEvent.sessionId &&
          event.turnId === replayEvent.turnId &&
          event.timestamp <= replayEvent.timestamp,
      )
    : false;
  const remediation = remedyEvent
    ? {
        remedy:
          stringValue(remedyPayload.summary) ??
          "Proposed remediation unavailable",
        controlledChange:
          stringValue(remedyPayload.controlledChange) ??
          "Controlled change unavailable",
        status:
          replayEvent && replayValidatesRemedy
            ? replayOutcome === "succeeded" &&
              expectedCauseObserved &&
              replayRequestedPermission
              ? ("validated" as const)
              : replayOutcome === "failed"
                ? ("rejected" as const)
                : ("inconclusive" as const)
            : ("not_tested" as const),
        result: replayEvent
          ? (stringValue(replayPayload.summary) ?? "Replay result unavailable")
          : "No controlled replay has tested this remedy yet.",
        evidenceIds: replayEvent ? [observerEventId(replayEvent)] : [],
      }
    : null;
  const causalEvidence = facts.filter(
    (event) =>
      event.class === "decision" ||
      event.title === "Tool failed" ||
      event.title === "Tool completed" ||
      event.title === "Turn failed",
  );

  return {
    task,
    outcome,
    why,
    confidence,
    evidence:
      causalEvidence.length > 0 ? causalEvidence.slice(-5) : facts.slice(-5),
    unknowns: gaps,
    nextAction:
      remediation?.status === "validated"
        ? "Use the validated governed execution path for the next run."
        : remediation?.status === "not_tested"
          ? `Replay with one controlled change: ${remediation.controlledChange}`
          : deniedDecision
            ? "Review the denied tool request and grant only the access the task requires."
            : latestError
              ? "Open the failed event below and address its diagnostic before retrying."
              : strongestFinding
                ? "Review the highest-confidence finding and its linked evidence before retrying."
                : "Collect the missing outcome evidence before choosing a fix.",
    remediation,
  };
}
