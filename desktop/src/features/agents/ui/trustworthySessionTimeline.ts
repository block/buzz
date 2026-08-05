import type { ObserverEvent } from "./agentSessionTypes";

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

export type SessionExplanation = {
  task: string;
  outcome: SessionOutcome;
  why: string;
  confidence: "high" | "medium" | "low";
  evidence: CausalTimelineEvent[];
  unknowns: CausalTimelineEvent[];
  nextAction: string;
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

function observerEventId(event: ObserverEvent): string {
  return `observer:${event.sessionId ?? "unknown"}:${event.seq}`;
}

function projectObserverEvent(event: ObserverEvent): CausalTimelineEvent {
  const payload = objectPayload(event);
  const eventId = observerEventId(event);

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
    ...observerEvents.map(projectObserverEvent),
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
  const timeline = buildTrustworthySessionTimeline(observerEvents, findings);
  if (timeline.length === 0) return null;

  const latestError = [...timeline]
    .reverse()
    .find((event) => event.title === "Turn failed");
  const latestCompleted = [...observerEvents]
    .reverse()
    .find((event) => event.kind === "turn_completed");
  const latestStarted = [...observerEvents]
    .reverse()
    .find((event) => event.kind === "turn_started");
  const strongestFinding = [...timeline]
    .reverse()
    .find(
      (event) =>
        event.class === "inference" && event.confidence === "correlated",
    );
  const deniedDecision = [...timeline]
    .reverse()
    .find(
      (event) =>
        event.class === "decision" && event.title === "Tool request denied",
    );
  const gaps = timeline.filter((event) => event.class === "gap");
  const facts = timeline.filter((event) => event.class !== "gap");

  const outcome: SessionOutcome = latestError
    ? "failed"
    : latestCompleted
      ? "succeeded"
      : latestStarted
        ? "in_progress"
        : "unknown";
  const cause = strongestFinding ?? latestError ?? deniedDecision ?? null;
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

  return {
    task: "Task description unavailable from current session evidence",
    outcome,
    why,
    confidence,
    evidence: facts.slice(-5),
    unknowns: gaps,
    nextAction: deniedDecision
      ? "Review the denied tool request and grant only the access the task requires."
      : latestError
        ? "Open the failed event below and address its diagnostic before retrying."
        : strongestFinding
          ? "Review the highest-confidence finding and its linked evidence before retrying."
          : "Collect the missing outcome evidence before choosing a fix.",
  };
}
