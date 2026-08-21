import type { AcpRuntimeCatalogEntry, ManagedAgent } from "@/shared/api/types";
import type {
  ProviderUsageId,
  ProviderUsageWindow,
} from "@/shared/api/tauriProviderUsage";

export type AgentProviderUsageResolution = {
  providerUsageId: ProviderUsageId | null;
  runtimeLabel: string;
};

export type ProviderAllowanceLevel =
  | "healthy"
  | "low"
  | "critical"
  | "exhausted";

/**
 * Resolve the effective runtime from the backend catalog rather than teaching
 * React about specific harness ids. Older managed-agent records can lack a
 * runtime id, so the resolved command remains the compatibility fallback.
 */
export function resolveAgentProviderUsage(
  agent: Pick<ManagedAgent, "agentCommand" | "runtime">,
  runtimes: AcpRuntimeCatalogEntry[],
): AgentProviderUsageResolution {
  const runtimeId = agent.runtime?.trim();
  const command = agent.agentCommand.trim();
  const runtime =
    (runtimeId
      ? runtimes.find((candidate) => candidate.id === runtimeId)
      : undefined) ??
    runtimes.find((candidate) => candidate.command?.trim() === command) ??
    runtimes.find((candidate) => candidate.id === command);

  return {
    providerUsageId: runtime?.providerUsageId ?? null,
    runtimeLabel: runtime?.label ?? (command || "Runtime unavailable"),
  };
}

/** 80%, 90%, and 100% consumed thresholds expressed as allowance remaining. */
export function providerAllowanceLevel(
  remainingPercent: number,
): ProviderAllowanceLevel {
  if (remainingPercent <= 0) return "exhausted";
  if (remainingPercent <= 10) return "critical";
  if (remainingPercent <= 20) return "low";
  return "healthy";
}

export function constrainingProviderWindow(
  windows: ProviderUsageWindow[],
): ProviderUsageWindow | null {
  return (
    windows.reduce<ProviderUsageWindow | null>(
      (lowest, window) =>
        lowest === null || window.remainingPercent < lowest.remainingPercent
          ? window
          : lowest,
      null,
    ) ?? null
  );
}
