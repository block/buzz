import type { ManagedAgent } from "@/shared/api/types";

export type AgentCardStatus = "working" | "available" | "error" | "off";

export function deriveAgentCardStatus({
  hasError,
  isWorking,
  status,
}: {
  hasError: boolean;
  isWorking: boolean;
  status: ManagedAgent["status"] | null;
}): AgentCardStatus {
  const isRuntimeActive = status === "running" || status === "deployed";
  if (!isRuntimeActive) {
    return hasError ? "error" : "off";
  }
  return isWorking ? "working" : "available";
}

export function formatAgentCardActivityChannel(
  channelName: string | null | undefined,
) {
  const visibleName = channelName?.trim();
  return visibleName ? `#${visibleName}` : "activiteit";
}
