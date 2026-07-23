import { invokeTauri } from "@/shared/api/tauri";
import type {
  GlobalAgentConfig,
  GlobalAgentConfigSaveResult,
} from "@/shared/api/types";

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
 * shape and reserved-key rules, restarts running local agents whose effective
 * env changed, and returns the saved config with a restart count.
 *
 * Throws a string error message on validation failure.
 */
export async function setGlobalAgentConfig(
  config: GlobalAgentConfig,
): Promise<GlobalAgentConfigSaveResult> {
  return invokeTauri<GlobalAgentConfigSaveResult>("set_global_agent_config", {
    config,
  });
}
