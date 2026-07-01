/**
 * React hook: load the global agent configuration defaults.
 *
 * Fetches once on mount and exposes the config. Error is swallowed and
 * treated as no global config (safe — the absence of a global config is
 * not an error state for callers).
 */
import * as React from "react";

import { getGlobalAgentConfig } from "@/shared/api/tauriGlobalAgentConfig";
import type { GlobalAgentConfig } from "@/shared/api/types";

const EMPTY_CONFIG: GlobalAgentConfig = {
  env_vars: {},
  provider: null,
  model: null,
};

export function useGlobalAgentConfig(): {
  globalConfig: GlobalAgentConfig;
  isLoading: boolean;
} {
  const [globalConfig, setGlobalConfig] =
    React.useState<GlobalAgentConfig>(EMPTY_CONFIG);
  const [isLoading, setIsLoading] = React.useState(true);

  React.useEffect(() => {
    let cancelled = false;
    getGlobalAgentConfig()
      .then((config) => {
        if (!cancelled) {
          setGlobalConfig(config);
          setIsLoading(false);
        }
      })
      .catch(() => {
        // Treat load failure as no global config — never block the dialog.
        if (!cancelled) setIsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return { globalConfig, isLoading };
}
