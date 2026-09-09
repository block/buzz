import * as React from "react";

import type {
  CreatePersonaInput,
  UpdatePersonaInput,
} from "@/shared/api/types";
import { computeLocalModeGate } from "./agentConfigOptions";
import {
  agentAiConfigurationModeSatisfied,
  type AgentAiConfigurationMode,
} from "./agentAiConfigurationPolicy";
import {
  definitionEditPreservesExecutionConfiguration,
  definitionExecutionReadinessSatisfied,
} from "./agentDefinitionExecutionReadiness";

type LocalModeGateInput = Parameters<typeof computeLocalModeGate>[0];

export function useAgentDefinitionExecutionReadiness({
  aiConfigurationMode,
  initialValues,
  isRuntimeAutoSeeded,
  runtimeCanChooseLlmProvider,
  ...localModeInput
}: Omit<LocalModeGateInput, "isProviderMode"> & {
  aiConfigurationMode: AgentAiConfigurationMode;
  initialValues: CreatePersonaInput | UpdatePersonaInput | null;
  isRuntimeAutoSeeded: boolean;
  runtimeCanChooseLlmProvider: boolean;
}) {
  const {
    bakedEnvKeys,
    envVars,
    globalEnvVars,
    globalModel,
    globalProvider,
    model,
    provider,
    runtimeFileConfig,
    runtimeId,
  } = localModeInput;
  const localModeGate = React.useMemo(
    () =>
      computeLocalModeGate({
        bakedEnvKeys,
        envVars,
        globalEnvVars,
        globalModel,
        globalProvider,
        isProviderMode: false,
        model,
        provider,
        runtimeFileConfig,
        runtimeId,
      }),
    [
      bakedEnvKeys,
      envVars,
      globalEnvVars,
      globalModel,
      globalProvider,
      model,
      provider,
      runtimeFileConfig,
      runtimeId,
    ],
  );
  const customAiPairSatisfied = agentAiConfigurationModeSatisfied(
    aiConfigurationMode,
    { provider, model },
    runtimeCanChooseLlmProvider,
  );
  const isEditMode = Boolean(initialValues && "id" in initialValues);
  const preservesExecutionConfiguration = Boolean(
    initialValues &&
      "id" in initialValues &&
      definitionEditPreservesExecutionConfiguration({
        initial: initialValues,
        runtime: runtimeId,
        model: aiConfigurationMode === "defaults" ? "" : model,
        provider: aiConfigurationMode === "defaults" ? "" : provider,
        envVars,
        isRuntimeAutoSeeded,
      }),
  );

  return {
    localModeGate,
    executionReadinessSatisfied: definitionExecutionReadinessSatisfied({
      isEditMode,
      preservesExecutionConfiguration,
      localModeSatisfied: localModeGate.satisfied,
      customAiPairSatisfied,
    }),
  };
}
