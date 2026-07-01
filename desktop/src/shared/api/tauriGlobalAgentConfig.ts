import { invokeTauri } from "@/shared/api/tauri";
import type { GlobalAgentConfig } from "@/shared/api/types";

/**
 * Read the current global agent configuration defaults.
 *
 * Returns an empty default if the file has not been written yet.
 */
export async function getGlobalAgentConfig(): Promise<GlobalAgentConfig> {
  return invokeTauri<GlobalAgentConfig>("get_global_agent_config");
}

/**
 * Validate and persist a new global agent configuration.
 *
 * The backend strips empty env values (empty = "inherit"), validates key
 * shape and reserved-key rules, and returns the saved config.
 *
 * Throws a string error message on validation failure.
 */
export async function setGlobalAgentConfig(
  config: GlobalAgentConfig,
): Promise<GlobalAgentConfig> {
  return invokeTauri<GlobalAgentConfig>("set_global_agent_config", { config });
}
