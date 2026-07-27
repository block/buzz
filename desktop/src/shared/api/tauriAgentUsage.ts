import { invokeTauri } from "@/shared/api/tauri";
import type { AgentUsageSummary } from "@/shared/api/agentUsageTypes";

/**
 * Read bounded local usage and effective runtime metadata for every managed
 * agent.
 *
 * Prompt measurements are derived from harness logs and are estimates rather
 * than provider billing data. No prompt or message content crosses this
 * boundary.
 */
export async function getAgentUsageDashboard(): Promise<AgentUsageSummary[]> {
  return invokeTauri<AgentUsageSummary[]>("get_agent_usage_dashboard");
}
