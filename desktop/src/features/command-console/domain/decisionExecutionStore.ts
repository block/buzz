export const DECISION_EXECUTION_STORAGE_KEY =
  "command-adviser:decision-executions:v1";
export const DECISION_STALL_AFTER_MS = 5 * 60_000;

export const DECISION_EXECUTION_STATUSES = [
  "queued",
  "in_progress",
  "blocked",
  "completed",
  "failed",
  "stalled",
] as const;

export type DecisionExecutionStatus =
  (typeof DECISION_EXECUTION_STATUSES)[number];
export type DecisionDirectionSource = "coa_a" | "coa_b" | "user";

export type DecisionExecution = Readonly<{
  version: 1;
  key: string;
  runId: string;
  actionId: string;
  direction: string;
  directionSource: DecisionDirectionSource;
  status: DecisionExecutionStatus;
  createdAt: number;
  updatedAt: number;
  lastActivityAt: number;
  agentPubkey?: string;
  channelId?: string;
  statusText?: string;
}>;

const TERMINAL_STATUSES = new Set<DecisionExecutionStatus>([
  "blocked",
  "completed",
  "failed",
]);

function nonEmptyText(value: unknown): value is string {
  return typeof value === "string" && value.trim().length > 0;
}

function finiteTime(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0;
}

function parseExecution(value: unknown): DecisionExecution | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const item = value as Record<string, unknown>;
  if (
    item.version !== 1 ||
    !nonEmptyText(item.key) ||
    !nonEmptyText(item.runId) ||
    !nonEmptyText(item.actionId) ||
    !nonEmptyText(item.direction) ||
    !["coa_a", "coa_b", "user"].includes(String(item.directionSource)) ||
    !DECISION_EXECUTION_STATUSES.includes(
      item.status as DecisionExecutionStatus,
    ) ||
    !finiteTime(item.createdAt) ||
    !finiteTime(item.updatedAt) ||
    !finiteTime(item.lastActivityAt) ||
    (item.agentPubkey !== undefined && !nonEmptyText(item.agentPubkey)) ||
    (item.channelId !== undefined && !nonEmptyText(item.channelId)) ||
    (item.statusText !== undefined && !nonEmptyText(item.statusText))
  ) {
    return null;
  }
  return Object.freeze({
    version: 1,
    key: item.key,
    runId: item.runId,
    actionId: item.actionId,
    direction: item.direction.trim(),
    directionSource: item.directionSource as DecisionDirectionSource,
    status: item.status as DecisionExecutionStatus,
    createdAt: item.createdAt,
    updatedAt: item.updatedAt,
    lastActivityAt: item.lastActivityAt,
    ...(item.agentPubkey ? { agentPubkey: item.agentPubkey } : {}),
    ...(item.channelId ? { channelId: item.channelId } : {}),
    ...(item.statusText ? { statusText: item.statusText } : {}),
  });
}

export function createDecisionExecution(input: {
  key: string;
  runId: string;
  actionId: string;
  direction: string;
  directionSource: DecisionDirectionSource;
  now?: number;
}): DecisionExecution {
  const now = input.now ?? Date.now();
  const parsed = parseExecution({
    version: 1,
    ...input,
    status: "queued",
    createdAt: now,
    updatedAt: now,
    lastActivityAt: now,
    now: undefined,
  });
  if (!parsed) throw new Error("Invalid command direction.");
  return parsed;
}

export function updateDecisionExecution(
  execution: DecisionExecution,
  update: Partial<
    Pick<
      DecisionExecution,
      "status" | "agentPubkey" | "channelId" | "statusText"
    >
  > & { now?: number },
): DecisionExecution {
  const now = update.now ?? Date.now();
  const parsed = parseExecution({
    ...execution,
    ...update,
    updatedAt: now,
    lastActivityAt: now,
    now: undefined,
  });
  if (!parsed) throw new Error("Invalid command direction update.");
  return parsed;
}

export function markSilentExecutionStalled(
  execution: DecisionExecution,
  now = Date.now(),
): DecisionExecution {
  if (
    TERMINAL_STATUSES.has(execution.status) ||
    execution.status === "stalled" ||
    now - execution.lastActivityAt < DECISION_STALL_AFTER_MS
  ) {
    return execution;
  }
  return updateDecisionExecution(execution, {
    status: "stalled",
    statusText: "No agent activity has been observed for five minutes.",
    now,
  });
}

export function parseDecisionExecutions(
  stored: string | null,
): readonly DecisionExecution[] {
  if (!stored) return Object.freeze([]);
  try {
    const envelope = JSON.parse(stored) as {
      version?: unknown;
      executions?: unknown;
    };
    if (envelope.version !== 1 || !Array.isArray(envelope.executions)) {
      return Object.freeze([]);
    }
    const byKey = new Map<string, DecisionExecution>();
    for (const candidate of envelope.executions) {
      const parsed = parseExecution(candidate);
      if (parsed && !byKey.has(parsed.key)) byKey.set(parsed.key, parsed);
    }
    return Object.freeze([...byKey.values()]);
  } catch {
    return Object.freeze([]);
  }
}

export function serializeDecisionExecutions(
  executions: readonly DecisionExecution[],
): string {
  return JSON.stringify({ version: 1, executions });
}
