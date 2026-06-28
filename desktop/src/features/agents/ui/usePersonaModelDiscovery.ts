import * as React from "react";

import { discoverAgentModels } from "@/shared/api/agentModels";
import type {
  AcpRuntimeCatalogEntry,
  AgentModelsResponse,
} from "@/shared/api/types";
import type { EnvVarsValue } from "./EnvVarsEditor";
import {
  formatModelDiscoveryErrorStatus,
  formatModelDiscoveryFallbackStatus,
  type PersonaModelDiscoveryStatus,
} from "./personaModelDiscoveryStatus";
import type { PersonaModelOption } from "./personaDialogPickers";

export const MODEL_DISCOVERY_LOADING_VALUE = "__model_discovery_loading__";

function stableModelDiscoveryEnvKey(envVars: EnvVarsValue): string {
  return JSON.stringify(
    Object.entries(envVars).sort(([left], [right]) =>
      left.localeCompare(right),
    ),
  );
}

function getDiscoveredPersonaModelOptions(
  response: AgentModelsResponse | null,
): readonly PersonaModelOption[] | null {
  if (!response?.supportsSwitching || response.models.length === 0) {
    return null;
  }

  return [
    { id: "", label: "Auto (default)" },
    ...response.models.map((model) => ({
      id: model.id,
      label: model.name?.trim() || model.id,
    })),
  ];
}

export function usePersonaModelDiscovery({
  envVars,
  isCustomProviderEditing,
  modelFieldVisible,
  open,
  provider,
  selectedRuntime,
}: {
  envVars: EnvVarsValue;
  isCustomProviderEditing: boolean;
  modelFieldVisible: boolean;
  open: boolean;
  provider: string;
  selectedRuntime: AcpRuntimeCatalogEntry | undefined;
}) {
  const [modelDiscoveryData, setModelDiscoveryData] =
    React.useState<AgentModelsResponse | null>(null);
  const [modelDiscoveryStatus, setModelDiscoveryStatus] =
    React.useState<PersonaModelDiscoveryStatus | null>(null);
  const [modelDiscoveryLoading, setModelDiscoveryLoading] =
    React.useState(false);
  const modelDiscoveryCacheRef = React.useRef(
    new Map<string, AgentModelsResponse>(),
  );
  const modelDiscoveryRequestRef = React.useRef(0);

  const trimmedProvider = provider.trim();
  const discoveryAgentCommand = selectedRuntime?.command?.trim()
    ? selectedRuntime.command
    : null;
  const canDiscoverModelOptions =
    open &&
    modelFieldVisible &&
    selectedRuntime?.availability === "available" &&
    discoveryAgentCommand !== null &&
    (!isCustomProviderEditing || trimmedProvider.length > 0);
  const modelDiscoveryEnvKey = React.useMemo(
    () => stableModelDiscoveryEnvKey(envVars),
    [envVars],
  );
  const modelDiscoveryArgsKey = JSON.stringify(
    selectedRuntime?.defaultArgs ?? [],
  );
  const modelDiscoveryKey = React.useMemo(() => {
    if (!canDiscoverModelOptions || discoveryAgentCommand === null) {
      return null;
    }

    return JSON.stringify({
      agentCommand: discoveryAgentCommand,
      agentArgs: modelDiscoveryArgsKey,
      provider: trimmedProvider,
      envVars: modelDiscoveryEnvKey,
    });
  }, [
    canDiscoverModelOptions,
    discoveryAgentCommand,
    modelDiscoveryArgsKey,
    modelDiscoveryEnvKey,
    trimmedProvider,
  ]);

  React.useEffect(() => {
    if (modelDiscoveryKey === null || discoveryAgentCommand === null) {
      modelDiscoveryRequestRef.current += 1;
      setModelDiscoveryData(null);
      setModelDiscoveryStatus(null);
      setModelDiscoveryLoading(false);
      return;
    }

    const requestId = modelDiscoveryRequestRef.current + 1;
    modelDiscoveryRequestRef.current = requestId;
    const cached = modelDiscoveryCacheRef.current.get(modelDiscoveryKey);
    if (cached) {
      setModelDiscoveryData(cached);
      setModelDiscoveryStatus(null);
      setModelDiscoveryLoading(false);
      return;
    }

    setModelDiscoveryData(null);
    setModelDiscoveryStatus(null);
    setModelDiscoveryLoading(true);
    void discoverAgentModels({
      agentCommand: discoveryAgentCommand,
      agentArgs: selectedRuntime?.defaultArgs ?? [],
      provider: trimmedProvider || undefined,
      envVars,
    })
      .then((response) => {
        if (modelDiscoveryRequestRef.current !== requestId) {
          return;
        }
        modelDiscoveryCacheRef.current.set(modelDiscoveryKey, response);
        setModelDiscoveryData(response);
        setModelDiscoveryStatus(null);
      })
      .catch((error) => {
        if (modelDiscoveryRequestRef.current !== requestId) {
          return;
        }
        setModelDiscoveryData(null);
        setModelDiscoveryStatus(
          formatModelDiscoveryErrorStatus(error, trimmedProvider),
        );
      })
      .finally(() => {
        if (modelDiscoveryRequestRef.current === requestId) {
          setModelDiscoveryLoading(false);
        }
      });
  }, [
    discoveryAgentCommand,
    envVars,
    modelDiscoveryKey,
    selectedRuntime?.defaultArgs,
    trimmedProvider,
  ]);

  const discoveredModelOptions = React.useMemo(
    () => getDiscoveredPersonaModelOptions(modelDiscoveryData),
    [modelDiscoveryData],
  );
  const modelDiscoveryFallbackStatus = React.useMemo(
    () =>
      discoveredModelOptions === null
        ? formatModelDiscoveryFallbackStatus({
            provider: trimmedProvider,
            response: modelDiscoveryData,
          })
        : null,
    [discoveredModelOptions, modelDiscoveryData, trimmedProvider],
  );

  return {
    discoveredModelOptions,
    modelDiscoveryLoading,
    modelDiscoveryStatus:
      modelDiscoveryLoading || discoveredModelOptions !== null
        ? null
        : (modelDiscoveryStatus ?? modelDiscoveryFallbackStatus),
  };
}
