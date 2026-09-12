export type AgentUsageWindowSnapshot = {
  label: string;
  usedPercent: number;
  resetAt?: number | null;
};

/** Wire-neutral NIP-AM shape expected from get_latest_agent_metric_snapshots. */
export type AgentMetricSnapshot = {
  agentPubkey: string;
  model: string | null;
  harness: string | null;
  contextUsedTokens: bigint | null;
  contextLimitTokens: bigint | null;
  contextTimestamp: number | null;
  accountUsageWindows: readonly AgentUsageWindowSnapshot[];
  accountUsageWindowsTimestamp: number | null;
  /** NIP-01 Unix timestamp in seconds. */
  timestamp: number;
};

export type AgentMetricSnapshotState = {
  snapshots: readonly AgentMetricSnapshot[];
  isLoading?: boolean;
  error?: string | null;
};

export type AgentStatusState =
  | "ready"
  | "stale"
  | "empty"
  | "loading"
  | "error"
  | "unconfigured";

export type AgentStatusViewModel = {
  id: string;
  label: string;
  state: AgentStatusState;
  agentPubkey?: string;
  model: string | null;
  harness: string | null;
  contextUsedTokens: bigint | null;
  contextLimitTokens: bigint | null;
  contextPercent: number | null;
  contextAgeSeconds: number | null;
  usageWindows: readonly AgentUsageWindowSnapshot[];
  usageWindowsAgeSeconds: number | null;
  timestamp: number | null;
  ageSeconds: number | null;
  errorMessage: string | null;
};
