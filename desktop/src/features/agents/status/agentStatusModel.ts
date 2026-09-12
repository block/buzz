import type { AgentStatusViewModel, AgentMetricSnapshot } from "./types";

const DEFAULT_STALE_AFTER_SECONDS = 45 * 60;

function clampPercent(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(100, Math.max(0, value));
}

function normalizedPubkey(value: string | undefined): string | null {
  const candidate = value?.trim().toLowerCase();
  return candidate && /^[0-9a-f]{64}$/.test(candidate) ? candidate : null;
}

function newestSnapshot(
  snapshots: readonly AgentMetricSnapshot[],
  pubkey: string,
): AgentMetricSnapshot | null {
  let newest: AgentMetricSnapshot | null = null;
  for (const snapshot of snapshots) {
    if (snapshot.agentPubkey.trim().toLowerCase() !== pubkey) continue;
    if (!newest || snapshot.timestamp > newest.timestamp) newest = snapshot;
  }
  return newest;
}

export function deriveConfiguredAgentStatuses({
  agents,
  snapshots,
  isLoading = false,
  error = null,
  nowSeconds = Math.floor(Date.now() / 1_000),
  staleAfterSeconds = DEFAULT_STALE_AFTER_SECONDS,
}: {
  agents: readonly { agentPubkey: string; id: string; label: string }[];
  snapshots: readonly AgentMetricSnapshot[];
  isLoading?: boolean;
  error?: string | null;
  nowSeconds?: number;
  staleAfterSeconds?: number;
}): AgentStatusViewModel[] {
  return agents.map((agent) => {
    const pubkey = normalizedPubkey(agent.agentPubkey);
    const base = {
      id: agent.id,
      label: agent.label,
      agentPubkey: pubkey ?? undefined,
      model: null,
      harness: null,
      contextUsedTokens: null,
      contextLimitTokens: null,
      contextPercent: null,
      contextAgeSeconds: null,
      usageWindows: [],
      usageWindowsAgeSeconds: null,
      timestamp: null,
      ageSeconds: null,
      errorMessage: null,
    } satisfies Omit<AgentStatusViewModel, "state">;

    if (!pubkey) return { ...base, state: "unconfigured" as const };

    const snapshot = newestSnapshot(snapshots, pubkey);
    if (!snapshot) {
      if (error) {
        return { ...base, state: "error" as const, errorMessage: error };
      }
      return {
        ...base,
        state: isLoading ? ("loading" as const) : ("empty" as const),
      };
    }

    const hasContext =
      snapshot.contextUsedTokens !== null &&
      snapshot.contextLimitTokens !== null &&
      snapshot.contextLimitTokens > 0n;
    const contextPercent = hasContext
      ? clampPercent(
          Number(
            ((snapshot.contextUsedTokens ?? 0n) * 10_000n) /
              (snapshot.contextLimitTokens ?? 1n),
          ) / 100,
        )
      : null;
    const usageWindows = snapshot.accountUsageWindows.map((window) => ({
      ...window,
      usedPercent: clampPercent(window.usedPercent),
    }));
    const contextTimestamp = snapshot.contextTimestamp ?? snapshot.timestamp;
    const usageWindowsTimestamp =
      snapshot.accountUsageWindowsTimestamp ?? snapshot.timestamp;
    const contextAgeSeconds =
      contextPercent === null
        ? null
        : Math.max(0, nowSeconds - contextTimestamp);
    const usageWindowsAgeSeconds =
      usageWindows.length === 0
        ? null
        : Math.max(0, nowSeconds - usageWindowsTimestamp);
    const primaryCandidates = [
      ...(contextPercent === null
        ? []
        : [{ percent: contextPercent, timestamp: contextTimestamp }]),
      ...usageWindows.map((window) => ({
        percent: window.usedPercent,
        timestamp: usageWindowsTimestamp,
      })),
    ];
    const primaryMetric = primaryCandidates.reduce<
      { percent: number; timestamp: number } | undefined
    >(
      (highest, candidate) =>
        !highest || candidate.percent > highest.percent ? candidate : highest,
      undefined,
    );
    const ageSeconds = Math.max(
      0,
      nowSeconds - (primaryMetric?.timestamp ?? snapshot.timestamp),
    );

    return {
      ...base,
      state: ageSeconds > staleAfterSeconds ? "stale" : "ready",
      model: snapshot.model,
      harness: snapshot.harness,
      contextUsedTokens: snapshot.contextUsedTokens,
      contextLimitTokens: snapshot.contextLimitTokens,
      contextPercent,
      contextAgeSeconds,
      usageWindows,
      usageWindowsAgeSeconds,
      timestamp: snapshot.timestamp,
      ageSeconds,
    };
  });
}
