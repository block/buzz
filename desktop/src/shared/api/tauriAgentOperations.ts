import { invokeTauri } from "@/shared/api/tauri";
import type {
  AgentOperationsConfig,
  AgentOperationsStatus,
} from "@/shared/api/types";

export async function getAgentOperationsStatus(): Promise<AgentOperationsStatus> {
  return invokeTauri<AgentOperationsStatus>("get_agent_operations_status");
}

export async function saveAgentOperationsConfig(
  input: AgentOperationsConfig,
): Promise<AgentOperationsStatus> {
  return invokeTauri<AgentOperationsStatus>("save_agent_operations_config", {
    input,
  });
}
