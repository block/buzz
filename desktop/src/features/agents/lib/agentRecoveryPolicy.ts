export const AGENT_RECOVERY_BACKOFF_MS = [5_000, 30_000, 120_000] as const;

export type AgentRecoveryState = {
  attempts: number;
  firstFailedAt: number;
  nextAttemptAt: number;
  lastError: string | null;
};

export function beginAgentRecovery(
  now: number,
  error: string | null,
): AgentRecoveryState {
  return {
    attempts: 0,
    firstFailedAt: now,
    nextAttemptAt: now + AGENT_RECOVERY_BACKOFF_MS[0],
    lastError: error,
  };
}

export function recoveryAttemptDue(
  state: AgentRecoveryState,
  now: number,
  agentIsWorking: boolean,
): boolean {
  return (
    !agentIsWorking &&
    state.attempts < AGENT_RECOVERY_BACKOFF_MS.length &&
    now >= state.nextAttemptAt
  );
}

export function recordFailedRecoveryAttempt(
  state: AgentRecoveryState,
  now: number,
  error: string | null,
): AgentRecoveryState {
  const attempts = state.attempts + 1;
  const nextDelay = AGENT_RECOVERY_BACKOFF_MS[attempts];
  return {
    ...state,
    attempts,
    nextAttemptAt:
      nextDelay === undefined ? Number.POSITIVE_INFINITY : now + nextDelay,
    lastError: error,
  };
}

export function recoveryExhausted(state: AgentRecoveryState): boolean {
  return state.attempts >= AGENT_RECOVERY_BACKOFF_MS.length;
}

export function recoveryLifecycleHealthy(lifecycle: string): boolean {
  return (
    lifecycle === "listening" || lifecycle === "waking" || lifecycle === "ready"
  );
}
