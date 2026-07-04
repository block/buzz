import type { AgentModelsResponse } from "@/shared/api/types";
import { invokeTauri } from "@/shared/api/tauri";

export type DiscoverAgentModelsInput = {
  acpCommand?: string;
  agentCommand: string;
  agentArgs?: string[];
  provider?: string;
  envVars?: Record<string, string>;
};

export async function discoverAgentModels(input: DiscoverAgentModelsInput) {
  return invokeTauri<AgentModelsResponse>("discover_agent_models", { input });
}
