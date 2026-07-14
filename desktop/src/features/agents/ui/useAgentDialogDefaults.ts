import * as React from "react";

import { useBakedBuildEnvQuery } from "../hooks";
import { useGlobalAgentConfig } from "../useGlobalAgentConfig";
import { BUZZ_AGENT_THINKING_EFFORT } from "./buzzAgentConfig";
import {
  countNonSecretInheritedEnvVars,
  getAdvancedInheritedSummary,
  getInheritedAgentDefaults,
} from "./bakedEnvHelpers";

export function useAgentDialogDefaults({
  inheritedEnvVars = {},
  open,
}: {
  inheritedEnvVars?: Record<string, string>;
  open: boolean;
}) {
  const { globalConfig } = useGlobalAgentConfig();
  const { data: bakedEnv } = useBakedBuildEnvQuery({ enabled: open });
  const inheritedDefaults = getInheritedAgentDefaults(globalConfig, bakedEnv);
  const effectiveInheritedEnvVars = React.useMemo(
    () => ({
      ...globalConfig.env_vars,
      ...inheritedEnvVars,
      ...(inheritedDefaults.effort.value
        ? { [BUZZ_AGENT_THINKING_EFFORT]: inheritedDefaults.effort.value }
        : {}),
    }),
    [globalConfig.env_vars, inheritedDefaults.effort.value, inheritedEnvVars],
  );
  const advancedInheritedSummary = getAdvancedInheritedSummary(
    inheritedDefaults.effort,
    countNonSecretInheritedEnvVars(
      globalConfig.env_vars,
      BUZZ_AGENT_THINKING_EFFORT,
    ),
  );

  return {
    advancedInheritedSummary,
    globalConfig,
    inheritedDefaults,
    inheritedEnvVars: effectiveInheritedEnvVars,
  };
}
