import {
  observerEventIdentity,
  type ObserverEvent,
} from "./ui/agentSessionTypes";

export type ActivityProofState =
  | "OBSERVED"
  | "CLAIMED"
  | "RECEIPTED"
  | "VERIFIED"
  | "FAILED"
  | "UNKNOWN";

export type ActivityStatus =
  | "pending"
  | "running"
  | "completed"
  | "failed"
  | "blocked"
  | "unknown";

export type MissionJournalStatus =
  | "in_progress"
  | "completed"
  | "failed"
  | "ended_unverified"
  | "incomplete"
  | "observed";

export type ActivityCategory =
  | "turn"
  | "tool"
  | "message"
  | "thought"
  | "plan"
  | "permission"
  | "prompt"
  | "status";

export type ActivityProvenance = {
  sourceEventId: string | null;
  sourcePubkey: string | null;
  sourceKind: number | null;
  sourceCreatedAt: number | null;
  sourceSignature: string | null;
  origin: "live_observer" | "historical_backfill" | "unknown";
  observerKind: string;
  method: string | null;
  sessionUpdate: string | null;
  seq: number;
  timestamp: string;
  channelId: string | null;
  sessionId: string | null;
  turnId: string | null;
  toolCallId: string | null;
  messageId: string | null;
  triggeringEventIds: string[];
};

export type NormalizedActivityEvent = {
  id: string;
  journalKey: string;
  correlationId: string;
  category: ActivityCategory;
  title: string;
  detail: string | null;
  status: ActivityStatus;
  proofState: ActivityProofState;
  timestamp: string;
  channelId: string | null;
  sessionId: string | null;
  turnId: string | null;
  toolCallId: string | null;
  messageId: string | null;
  provenance: ActivityProvenance;
  tags: string[];
  ownerModifiedAt?: string | null;
  ownerModifiedBy?: string | null;
};

export type MissionJournal = {
  id: string;
  journalKey: string;
  correlationId: string;
  channelId: string | null;
  sessionId: string | null;
  turnId: string | null;
  startedAt: string;
  endedAt: string;
  status: MissionJournalStatus;
  proofState: ActivityProofState;
  summary: string;
  summarySource: "auto" | "owner";
  ownerModifiedAt: string | null;
  ownerModifiedBy: string | null;
  claimedCompletionWithoutEvidence: boolean;
  eventCount: number;
  events: NormalizedActivityEvent[];
};

export type MissionJournalOverride = {
  summary: string;
  modifiedAt: string;
  modifiedBy: string;
};

export type MissionJournalBuildOptions = {
  asOf?: string | Date;
  incompleteAfterMs?: number;
};

export type TodayActivityFeedInput = {
  agentPubkey: string;
  agentName: string;
  events: NormalizedActivityEvent[];
};

export type TodayActivityChannel = {
  channelId: string;
  journalIds: string[];
  agentPubkeys: string[];
  agentNames: string[];
  lastActivityAt: string;
};

export type TodayActivityJournal = MissionJournal & {
  agentPubkey: string;
  agentName: string;
};

export type TodayActivitySurface = {
  day: string;
  journals: TodayActivityJournal[];
  channels: TodayActivityChannel[];
  counts: {
    journals: number;
    failed: number;
    inProgress: number;
    claimedWithoutEvidence: number;
  };
};

const PROOF_RANK: Record<ActivityProofState, number> = {
  UNKNOWN: 0,
  CLAIMED: 1,
  OBSERVED: 2,
  RECEIPTED: 3,
  VERIFIED: 4,
  FAILED: 5,
};

function asRecord(value: unknown): Record<string, unknown> {
  return value && typeof value === "object"
    ? (value as Record<string, unknown>)
    : {};
}

function asString(value: unknown): string | null {
  return typeof value === "string" && value.trim().length > 0 ? value : null;
}

function extractContentText(value: unknown): string | null {
  if (typeof value === "string") {
    return value.trim() || null;
  }
  const record = asRecord(value);
  const text = asString(record.text);
  if (text) return text;
  if (Array.isArray(value)) {
    const parts = value
      .map((entry) => extractContentText(entry))
      .filter((entry): entry is string => Boolean(entry));
    return parts.length > 0 ? parts.join("\n") : null;
  }
  return null;
}

function toolTitle(update: Record<string, unknown>) {
  return (
    asString(update.toolName) ??
    asString(update.kind) ??
    asString(update.title) ??
    "tool"
  );
}

function toolCallId(update: Record<string, unknown>) {
  return asString(update.toolCallId) ?? asString(update.tool_call_id);
}

function messageId(update: Record<string, unknown>) {
  return asString(update.messageId) ?? asString(update.message_id);
}

function statusFromUpdate(status: string | null | undefined): ActivityStatus {
  switch (status) {
    case "pending":
      return "pending";
    case "executing":
      return "running";
    case "completed":
    case "done":
      return "completed";
    case "failed":
    case "error":
      return "failed";
    default:
      return "unknown";
  }
}

function proofStateForTool(
  status: ActivityStatus,
  output: unknown,
): ActivityProofState {
  if (status === "failed") return "FAILED";
  if (status !== "completed") return "OBSERVED";
  return output === undefined || output === null ? "OBSERVED" : "RECEIPTED";
}

function triggeringEventIds(event: ObserverEvent): string[] {
  const ids = asRecord(event.payload).triggeringEventIds;
  return Array.isArray(ids)
    ? ids.filter((id): id is string => typeof id === "string" && id.length > 0)
    : [];
}

function correlationId(
  event: ObserverEvent,
  update?: Record<string, unknown>,
  turnCorrelationId?: string | null,
) {
  const toolId = update ? toolCallId(update) : null;
  return (
    toolId ??
    triggeringEventIds(event)[0] ??
    turnCorrelationId ??
    event.journalKey ??
    event.turnId ??
    event.sessionId ??
    event.channelId ??
    `${event.kind}:${event.seq}`
  );
}

function journalKey(event: ObserverEvent) {
  return (
    event.journalKey ??
    event.turnId ??
    event.sessionId ??
    event.channelId ??
    "global"
  );
}

function buildId(
  event: ObserverEvent,
  category: ActivityCategory,
  suffix: string | null = null,
) {
  return [category, observerEventIdentity(event), suffix]
    .filter(Boolean)
    .join(":");
}

function eventTagSet(
  category: ActivityCategory,
  updateType: string | null,
  toolName: string | null,
): string[] {
  const tags: string[] = [category];
  if (updateType) tags.push(updateType);
  if (toolName) tags.push(`tool:${toolName}`);
  return tags;
}

function compareObserverEvents(left: ObserverEvent, right: ObserverEvent) {
  const leftTime = Date.parse(left.timestamp);
  const rightTime = Date.parse(right.timestamp);
  if (
    Number.isFinite(leftTime) &&
    Number.isFinite(rightTime) &&
    leftTime !== rightTime
  ) {
    return leftTime - rightTime;
  }
  return left.seq - right.seq;
}

function dedupeObserverEvents(events: readonly ObserverEvent[]) {
  const seen = new Set<string>();
  const deduped: ObserverEvent[] = [];
  for (const event of [...events].sort(compareObserverEvents)) {
    const key = observerEventIdentity(event);
    if (seen.has(key)) continue;
    seen.add(key);
    deduped.push(event);
  }
  return deduped;
}

function localDay(timestamp: string) {
  const date = new Date(timestamp);
  const year = date.getFullYear();
  const month = `${date.getMonth() + 1}`.padStart(2, "0");
  const day = `${date.getDate()}`.padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function normalizeOne(
  event: ObserverEvent,
  turnCorrelationId: string | null,
): NormalizedActivityEvent | null {
  const payload = asRecord(event.payload);
  const method = asString(payload.method);
  const update =
    method === "session/update"
      ? asRecord(asRecord(payload.params).update)
      : null;
  const updateType = update ? asString(update.sessionUpdate) : null;
  const toolName = update ? toolTitle(update) : null;
  const callId = update ? toolCallId(update) : null;
  const msgId = update ? messageId(update) : null;
  const base = {
    journalKey: journalKey(event),
    correlationId: correlationId(event, update ?? undefined, turnCorrelationId),
    timestamp: event.timestamp,
    channelId: event.channelId ?? null,
    sessionId: event.sessionId ?? null,
    turnId: event.turnId ?? null,
    toolCallId: callId,
    messageId: msgId,
    provenance: {
      sourceEventId: event.sourceEventId ?? null,
      sourcePubkey: event.sourcePubkey ?? null,
      sourceKind: event.sourceKind ?? null,
      sourceCreatedAt: event.sourceCreatedAt ?? null,
      sourceSignature: event.sourceSignature ?? null,
      origin:
        event.origin === "live_observer" ||
        event.origin === "historical_backfill"
          ? event.origin
          : "unknown",
      observerKind: event.kind,
      method: method ?? null,
      sessionUpdate: updateType ?? null,
      seq: event.seq,
      timestamp: event.timestamp,
      channelId: event.channelId ?? null,
      sessionId: event.sessionId ?? null,
      turnId: event.turnId ?? null,
      toolCallId: callId,
      messageId: msgId,
      triggeringEventIds: triggeringEventIds(event),
    } satisfies ActivityProvenance,
  };

  if (event.kind === "turn_started") {
    return {
      ...base,
      id: buildId(event, "turn"),
      category: "turn",
      title: "Turn started",
      detail: null,
      status: "running",
      proofState: "OBSERVED",
      tags: eventTagSet("turn", null, null),
    };
  }

  if (event.kind === "turn_completed") {
    return {
      ...base,
      id: buildId(event, "turn"),
      category: "turn",
      title: "Turn completed",
      detail: null,
      status: "completed",
      proofState: "OBSERVED",
      tags: eventTagSet("turn", null, null),
    };
  }

  if (event.kind === "turn_error" || event.kind === "agent_panic") {
    return {
      ...base,
      id: buildId(event, "turn"),
      category: "turn",
      title: event.kind === "agent_panic" ? "Agent crashed" : "Turn failed",
      detail: asString(payload.error) ?? extractContentText(payload) ?? null,
      status: "failed",
      proofState: "FAILED",
      tags: eventTagSet("turn", null, null),
    };
  }

  if (event.kind === "managed_agent_runtime_lifecycle") {
    const lifecycle = asRecord(event.payload);
    const phase = asString(lifecycle.lifecycle);
    const detail = asString(lifecycle.error) ?? phase;
    const status =
      phase === "failed"
        ? "failed"
        : phase === "ready"
          ? "completed"
          : "running";
    return {
      ...base,
      id: buildId(event, "status", "runtime-lifecycle"),
      category: "status",
      title:
        phase === "failed"
          ? "Runtime failed"
          : phase === "ready"
            ? "Runtime ready"
            : "Runtime lifecycle observed",
      detail,
      status,
      proofState: phase === "failed" ? "FAILED" : "OBSERVED",
      tags: eventTagSet("status", "managed_agent_runtime_lifecycle", null),
    };
  }

  if (event.kind === "turn_liveness") {
    return {
      ...base,
      id: buildId(event, "status", "turn-liveness"),
      category: "status",
      title: "Turn active",
      detail: null,
      status: "running",
      proofState: "OBSERVED",
      tags: eventTagSet("status", "turn_liveness", null),
    };
  }

  if (event.kind === "session_config_captured") {
    const config = asRecord(event.payload);
    const provider = asString(config.provider);
    const model = asString(config.model);
    const detail =
      [
        provider ? `provider ${provider}` : null,
        model ? `model ${model}` : null,
      ]
        .filter((entry): entry is string => Boolean(entry))
        .join(", ") || null;
    return {
      ...base,
      id: buildId(event, "status", "session-config"),
      category: "status",
      title: "Session config captured",
      detail,
      status: "completed",
      proofState: "OBSERVED",
      tags: eventTagSet("status", "session_config_captured", null),
    };
  }

  if (event.kind === "journal_override") {
    // Managed observer envelopes are signed by the agent, not the owner.
    // Owner edits enter through applyOwnerJournalOverride after an authenticated
    // owner action; agent-authored lookalikes are deliberately ignored.
    return null;
  }

  if (event.kind === "proof_verified" || event.kind === "proof_failed") {
    const receiptRef = asString(payload.receiptRef);
    const failed = event.kind === "proof_failed";
    return {
      ...base,
      id: buildId(event, "status", event.kind),
      category: "status",
      title: failed ? "Proof verification failed" : "Verification claimed",
      detail: receiptRef,
      status: failed ? "failed" : "completed",
      // Observer envelopes are signed by the managed agent. Fields naming a
      // verifier are still self-reported until a separate trusted signature is
      // validated, so this path must never mint VERIFIED.
      proofState: failed ? "FAILED" : "CLAIMED",
      tags: eventTagSet("status", event.kind, null),
    };
  }

  if (event.kind === "session_resolved") {
    return {
      ...base,
      id: buildId(event, "status"),
      category: "status",
      title: "Session ready",
      detail: extractContentText(payload),
      status: "running",
      proofState: "OBSERVED",
      tags: eventTagSet("status", null, null),
    };
  }

  if (method === "session/request_permission") {
    return {
      ...base,
      id: buildId(event, "permission"),
      category: "permission",
      title: "Permission requested",
      detail:
        asString(asRecord(asRecord(payload.params).title)) ??
        asString(asRecord(payload.params).message) ??
        null,
      status: "blocked",
      proofState: "OBSERVED",
      tags: eventTagSet("permission", null, null),
    };
  }

  if (event.kind === "acp_write" && !method) {
    const result = asRecord(asRecord(payload.result).outcome);
    const outcome = asString(result.outcome);
    if (outcome) {
      return {
        ...base,
        id: buildId(event, "permission"),
        category: "permission",
        title: "Permission resolved",
        detail: outcome,
        status: "completed",
        proofState: "RECEIPTED",
        tags: eventTagSet("permission", null, null),
      };
    }
  }

  if (event.kind === "acp_write" && method === "session/prompt") {
    return {
      ...base,
      id: buildId(event, "prompt"),
      category: "prompt",
      title: "Prompt issued",
      detail: extractContentText(asRecord(payload.params).prompt),
      status: "completed",
      proofState: "RECEIPTED",
      tags: eventTagSet("prompt", null, null),
    };
  }

  if (updateType === "tool_call" || updateType === "tool_call_update") {
    const status = statusFromUpdate(asString(update?.status));
    const name = toolName ?? "tool";
    const output = update?.rawOutput ?? update?.content;
    return {
      ...base,
      id: buildId(event, "tool", callId ?? name),
      category: "tool",
      title: name,
      detail:
        extractContentText(update?.rawOutput) ??
        extractContentText(update?.content) ??
        extractContentText(update?.rawInput) ??
        null,
      status,
      proofState: proofStateForTool(status, output),
      tags: eventTagSet("tool", updateType, name),
    };
  }

  if (
    updateType === "agent_message_chunk" ||
    updateType === "user_message_chunk"
  ) {
    return {
      ...base,
      id: buildId(event, "message", msgId ?? updateType),
      category: "message",
      title:
        updateType === "agent_message_chunk" ? "Agent message" : "User message",
      detail: extractContentText(update?.content),
      status: "completed",
      proofState: "CLAIMED",
      tags: eventTagSet("message", updateType, null),
    };
  }

  if (updateType === "agent_thought_chunk") {
    return {
      ...base,
      id: buildId(event, "thought"),
      category: "thought",
      title: "Thought",
      detail: extractContentText(update?.content),
      status: "completed",
      proofState: "CLAIMED",
      tags: eventTagSet("thought", updateType, null),
    };
  }

  if (updateType === "plan") {
    return {
      ...base,
      id: buildId(event, "plan"),
      category: "plan",
      title: "Plan updated",
      detail: extractContentText(update?.content),
      status: "completed",
      proofState: "CLAIMED",
      tags: eventTagSet("plan", updateType, null),
    };
  }

  const freeformText = asString(payload.text) ?? extractContentText(payload);
  if (freeformText) {
    return {
      ...base,
      id: buildId(event, "status"),
      category: "status",
      title: asString(payload.title) ?? event.kind,
      detail: freeformText,
      status: event.kind.includes("error") ? "failed" : "completed",
      proofState: event.kind.includes("error") ? "FAILED" : "OBSERVED",
      tags: eventTagSet("status", updateType, null),
    };
  }

  return null;
}

export function normalizeActivityEvents(
  events: readonly ObserverEvent[],
): NormalizedActivityEvent[] {
  const deduped = dedupeObserverEvents(events);
  const latestLiveness = new Map<string, ObserverEvent>();
  for (const event of deduped) {
    if (event.kind !== "turn_liveness") continue;
    const key = event.turnId ?? event.sessionId ?? event.channelId ?? "global";
    latestLiveness.set(key, event);
  }
  const compacted = deduped.filter((event) => {
    if (event.kind !== "turn_liveness") return true;
    const key = event.turnId ?? event.sessionId ?? event.channelId ?? "global";
    return latestLiveness.get(key) === event;
  });
  const turnCorrelations = new Map<string, string>();
  for (const event of compacted) {
    const root = triggeringEventIds(event)[0];
    if (event.turnId && root) turnCorrelations.set(event.turnId, root);
  }
  return compacted
    .map((event) =>
      normalizeOne(
        event,
        event.turnId ? (turnCorrelations.get(event.turnId) ?? null) : null,
      ),
    )
    .filter((event): event is NormalizedActivityEvent => Boolean(event));
}

function bestProofState(events: readonly NormalizedActivityEvent[]) {
  return events.reduce<ActivityProofState>((best, event) => {
    return PROOF_RANK[event.proofState] > PROOF_RANK[best]
      ? event.proofState
      : best;
  }, "UNKNOWN");
}

function buildSummary(
  events: readonly NormalizedActivityEvent[],
  status: MissionJournalStatus,
  claimedCompletionWithoutEvidence: boolean,
) {
  const ownerOverride = events.find((event) => event.ownerModifiedAt != null);
  if (ownerOverride?.detail) {
    return ownerOverride.detail;
  }

  const toolNames = [
    ...new Set(
      events
        .filter((event) => event.category === "tool")
        .map((event) => event.title),
    ),
  ];
  if (status === "failed") {
    const failed = [...events]
      .reverse()
      .find(
        (event) =>
          event.status === "failed" &&
          (event.category === "turn" ||
            event.provenance.observerKind ===
              "managed_agent_runtime_lifecycle"),
      );
    return failed?.detail
      ? `${failed.title}: ${failed.detail}`
      : `${failed?.title ?? "Turn failed"} during observed execution.`;
  }
  if (claimedCompletionWithoutEvidence) {
    return "Execution ended without supporting evidence for the requested outcome.";
  }
  if (status === "incomplete") {
    return "Execution started but no terminal event was observed before the activity became stale.";
  }
  if (status === "completed" && toolNames.length > 0) {
    return `Execution ended with receipted activity in ${toolNames.join(", ")}; outcome verification remains separate.`;
  }
  if (toolNames.length > 0) {
    return `Observed work in ${toolNames.join(", ")}.`;
  }
  const claimed = events.find(
    (event) => event.proofState === "CLAIMED" && event.detail,
  );
  if (claimed?.detail) {
    return claimed.detail;
  }
  return "Observed agent activity.";
}

export function groupMissionJournals(
  events: readonly NormalizedActivityEvent[],
  options: MissionJournalBuildOptions = {},
): MissionJournal[] {
  const grouped = new Map<string, NormalizedActivityEvent[]>();
  for (const event of [...events].sort((left, right) => {
    const leftTime = Date.parse(left.timestamp);
    const rightTime = Date.parse(right.timestamp);
    return leftTime === rightTime
      ? left.provenance.seq - right.provenance.seq
      : leftTime - rightTime;
  })) {
    const bucket = grouped.get(event.journalKey) ?? [];
    bucket.push(event);
    grouped.set(event.journalKey, bucket);
  }

  return [...grouped.entries()].map(([key, bucket]) => {
    const startedAt = bucket[0]?.timestamp ?? new Date(0).toISOString();
    const endedAt = bucket[bucket.length - 1]?.timestamp ?? startedAt;
    const latestRuntimeLifecycle = [...bucket]
      .reverse()
      .find(
        (event) =>
          event.provenance.observerKind === "managed_agent_runtime_lifecycle",
      );
    const effectiveProofEvents = latestRuntimeLifecycle
      ? bucket.filter(
          (event) =>
            event.provenance.observerKind !==
              "managed_agent_runtime_lifecycle" ||
            event === latestRuntimeLifecycle,
        )
      : bucket;
    const hasTerminalFailure =
      bucket.some(
        (event) => event.status === "failed" && event.category === "turn",
      ) || latestRuntimeLifecycle?.status === "failed";
    const hasCompletion = bucket.some(
      (event) => event.category === "turn" && event.status === "completed",
    );
    const successfulEvidence = bucket.filter(
      (event) =>
        event.status === "completed" &&
        (event.proofState === "VERIFIED" ||
          (event.proofState === "RECEIPTED" &&
            (event.category === "tool" || event.category === "status"))),
    );
    const claimedCompletionWithoutEvidence =
      hasCompletion && successfulEvidence.length === 0;
    const latestToolState = new Map<string, ActivityStatus>();
    for (const event of bucket) {
      if (event.category === "tool") {
        latestToolState.set(event.correlationId, event.status);
      }
    }
    const hasUnresolvedToolFailure = [...latestToolState.values()].some(
      (status) => status === "failed",
    );
    const hasProofFailure = bucket.some(
      (event) =>
        event.provenance.observerKind === "proof_failed" ||
        (event.category === "status" &&
          event.proofState === "FAILED" &&
          event.provenance.observerKind !== "managed_agent_runtime_lifecycle"),
    );
    const asOf =
      options.asOf instanceof Date
        ? options.asOf.getTime()
        : options.asOf
          ? Date.parse(options.asOf)
          : Number.NaN;
    const incompleteAfterMs = options.incompleteAfterMs ?? 5 * 60_000;
    const isStaleIncomplete =
      !hasCompletion &&
      !hasTerminalFailure &&
      Number.isFinite(asOf) &&
      asOf - Date.parse(endedAt) >= incompleteAfterMs &&
      bucket.some(
        (event) => event.category === "turn" && event.status === "running",
      );

    let status: MissionJournalStatus = "observed";
    if (hasTerminalFailure) {
      status = "failed";
    } else if (claimedCompletionWithoutEvidence) {
      status = "ended_unverified";
    } else if (hasCompletion) {
      status = "completed";
    } else if (isStaleIncomplete) {
      status = "incomplete";
    } else if (
      effectiveProofEvents.some((event) => event.status === "running")
    ) {
      status = "in_progress";
    }

    const ownerOverride = bucket.find((event) => event.ownerModifiedAt != null);
    const proofState: ActivityProofState =
      hasTerminalFailure || hasProofFailure || hasUnresolvedToolFailure
        ? "FAILED"
        : isStaleIncomplete
          ? "UNKNOWN"
          : successfulEvidence.some((event) => event.proofState === "VERIFIED")
            ? "VERIFIED"
            : successfulEvidence.some(
                  (event) => event.proofState === "RECEIPTED",
                )
              ? "RECEIPTED"
              : claimedCompletionWithoutEvidence
                ? "OBSERVED"
                : bestProofState(effectiveProofEvents);

    return {
      id: key,
      journalKey: key,
      correlationId: bucket[0]?.correlationId ?? key,
      channelId: bucket[0]?.channelId ?? null,
      sessionId: bucket.find((event) => event.sessionId)?.sessionId ?? null,
      turnId: bucket.find((event) => event.turnId)?.turnId ?? null,
      startedAt,
      endedAt,
      status,
      proofState,
      summary: buildSummary(bucket, status, claimedCompletionWithoutEvidence),
      summarySource: ownerOverride ? "owner" : "auto",
      ownerModifiedAt: ownerOverride?.ownerModifiedAt ?? null,
      ownerModifiedBy: ownerOverride?.ownerModifiedBy ?? null,
      claimedCompletionWithoutEvidence,
      eventCount: bucket.length,
      events: bucket,
    } satisfies MissionJournal;
  });
}

export function buildMissionJournal(
  events: readonly NormalizedActivityEvent[],
  options: MissionJournalBuildOptions = {},
): MissionJournal {
  const journals = groupMissionJournals(events, options);
  let latest: MissionJournal | undefined;
  for (const journal of journals) {
    if (
      !latest ||
      Date.parse(journal.endedAt) > Date.parse(latest.endedAt) ||
      (journal.endedAt === latest.endedAt && journal.id > latest.id)
    ) {
      latest = journal;
    }
  }
  if (latest) return latest;
  return {
    id: "empty",
    journalKey: "empty",
    correlationId: "empty",
    channelId: null,
    sessionId: null,
    turnId: null,
    startedAt: new Date(0).toISOString(),
    endedAt: new Date(0).toISOString(),
    status: "observed",
    proofState: "UNKNOWN",
    summary: "No observed activity.",
    summarySource: "auto",
    ownerModifiedAt: null,
    ownerModifiedBy: null,
    claimedCompletionWithoutEvidence: false,
    eventCount: 0,
    events: [],
  };
}

export function applyOwnerJournalOverride(
  journal: MissionJournal,
  override: MissionJournalOverride,
): MissionJournal {
  return {
    ...journal,
    summary: override.summary,
    summarySource: "owner",
    ownerModifiedAt: override.modifiedAt,
    ownerModifiedBy: override.modifiedBy,
  };
}

export function buildTodayActivitySurface(
  feeds: readonly TodayActivityFeedInput[],
  options: {
    day: string;
    asOf?: string | Date;
    incompleteAfterMs?: number;
  },
): TodayActivitySurface {
  const journals: TodayActivityJournal[] = [];
  const channels = new Map<
    string,
    {
      journalIds: string[];
      agentPubkeys: Set<string>;
      agentNames: Set<string>;
      lastActivityAt: string;
    }
  >();

  for (const feed of feeds) {
    for (const journal of groupMissionJournals(
      feed.events.filter((event) => localDay(event.timestamp) === options.day),
      {
        asOf: options.asOf ?? new Date(),
        incompleteAfterMs: options.incompleteAfterMs,
      },
    )) {
      journals.push({
        ...journal,
        agentPubkey: feed.agentPubkey,
        agentName: feed.agentName,
      });

      if (!journal.channelId) continue;
      const bucket = channels.get(journal.channelId) ?? {
        journalIds: [],
        agentPubkeys: new Set<string>(),
        agentNames: new Set<string>(),
        lastActivityAt: journal.endedAt,
      };
      bucket.journalIds.push(journal.id);
      bucket.agentPubkeys.add(feed.agentPubkey);
      bucket.agentNames.add(feed.agentName);
      if (Date.parse(journal.endedAt) > Date.parse(bucket.lastActivityAt)) {
        bucket.lastActivityAt = journal.endedAt;
      }
      channels.set(journal.channelId, bucket);
    }
  }

  journals.sort(
    (left, right) => Date.parse(left.startedAt) - Date.parse(right.startedAt),
  );

  return {
    day: options.day,
    journals,
    channels: [...channels.entries()]
      .map(([channelId, bucket]) => ({
        channelId,
        journalIds: bucket.journalIds,
        agentPubkeys: [...bucket.agentPubkeys],
        agentNames: [...bucket.agentNames],
        lastActivityAt: bucket.lastActivityAt,
      }))
      .sort((left, right) => left.channelId.localeCompare(right.channelId)),
    counts: {
      journals: journals.length,
      failed: journals.filter((journal) => journal.status === "failed").length,
      inProgress: journals.filter((journal) => journal.status === "in_progress")
        .length,
      claimedWithoutEvidence: journals.filter(
        (journal) => journal.claimedCompletionWithoutEvidence,
      ).length,
    },
  };
}
