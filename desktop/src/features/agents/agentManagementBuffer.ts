import type { ManagedAgent } from "@/shared/api/types";

/**
 * Defers an ownership decision until managed-agent data has initialized.
 * Once loaded, only requests from an owned agent are accepted.
 */
export function classifyAgentManagementSender(
  agents: readonly Pick<ManagedAgent, "pubkey">[] | undefined,
  agentPubkey: string,
): "buffer" | "accept" | "reject" {
  if (agents === undefined) return "buffer";
  return agents.some(
    (agent) => agent.pubkey.toLowerCase() === agentPubkey.toLowerCase(),
  )
    ? "accept"
    : "reject";
}
