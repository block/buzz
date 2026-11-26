import type {
  AgentModelsResponse,
  AgentProvidersResponse,
} from "@/shared/api/types";
import { invokeTauri } from "@/shared/api/tauri";
export type DiscoverAgentModelsInput = {
  acpCommand?: string;
  agentCommand: string;
  agentArgs?: string[];
  provider?: string;
  envVars?: Record<string, string>;
  /** Definition-level env from the harness definition (custom/preset). Merged below user envVars. */
  definitionEnv?: Record<string, string>;
};

export async function discoverAgentModels(input: DiscoverAgentModelsInput) {
  return invokeTauri<AgentModelsResponse>("discover_agent_models", { input });
}

export type DiscoverAgentProvidersInput = {
  acpCommand?: string;
  agentCommand: string;
  agentArgs?: string[];
  envVars?: Record<string, string>;
  /** Definition-level env from the harness definition (custom/preset). Merged below user envVars. */
  definitionEnv?: Record<string, string>;
};

/**
 * Ask the harness for its own LLM provider inventory (goose publishes one).
 * Harnesses without the extension resolve with an empty `providers` list, so
 * callers can fall back to the built-in catalog.
 */
export async function discoverAgentProviders(
  input: DiscoverAgentProvidersInput,
) {
  return invokeTauri<AgentProvidersResponse>("discover_agent_providers", {
    input,
  });
}
