import * as React from "react";

import type {
  CreatePersonaInput,
  GlobalAgentConfig,
  UpdatePersonaInput,
} from "@/shared/api/types";
import { useBakedBuildEnvQuery } from "../hooks";
import {
  maskDisabledAcpRuntimePreference,
  useDisabledAcpRuntimeIds,
} from "../lib/runtimeVisibilityPreference";
import { useGlobalAgentConfig } from "../useGlobalAgentConfig";
import { BUZZ_AGENT_THINKING_EFFORT } from "./buzzAgentConfig";
import { getInheritedAgentDefaults } from "./bakedEnvHelpers";

export type AgentDialogConfigScope = "existing" | "implicit";

export function agentDefinitionConfigScope(
  initialValues: CreatePersonaInput | UpdatePersonaInput | null,
): AgentDialogConfigScope {
  return initialValues && "id" in initialValues ? "existing" : "implicit";
}

export function resolveAgentDialogGlobalConfig(
  persistedConfig: GlobalAgentConfig,
  configScope: AgentDialogConfigScope,
  disabledRuntimeIds: readonly string[],
): GlobalAgentConfig {
  return configScope === "implicit"
    ? maskDisabledAcpRuntimePreference(persistedConfig, disabledRuntimeIds)
    : persistedConfig;
}

export function useAgentDialogDefaults({
  configScope,
  inheritedEnvVars = {},
  open,
}: {
  configScope: AgentDialogConfigScope;
  inheritedEnvVars?: Record<string, string>;
  open: boolean;
}) {
  const { globalConfig: persistedConfig } = useGlobalAgentConfig();
  const disabledRuntimeIds = useDisabledAcpRuntimeIds();
  const globalConfig = React.useMemo(
    () =>
      resolveAgentDialogGlobalConfig(
        persistedConfig,
        configScope,
        disabledRuntimeIds,
      ),
    [configScope, disabledRuntimeIds, persistedConfig],
  );
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
  return {
    globalConfig,
    inheritedDefaults,
    inheritedEnvVars: effectiveInheritedEnvVars,
  };
}

export function useDefinitionAgentDialogDefaults(
  initialValues: CreatePersonaInput | UpdatePersonaInput | null,
  open: boolean,
) {
  return useAgentDialogDefaults({
    configScope: agentDefinitionConfigScope(initialValues),
    open,
  });
}
