import * as React from "react";

import { discoverAgentModels } from "@/shared/api/agentModels";
import type {
  AcpRuntimeCatalogEntry,
  AgentModelsResponse,
} from "@/shared/api/types";
import type { EnvVarsValue } from "./EnvVarsEditor";
import {
  formatModelDiscoveryErrorStatus,
  type PersonaModelDiscoveryStatus,
} from "./personaModelDiscoveryStatus";
import type { PersonaModelOption } from "./personaDialogPickers";
import { providerRequiresExplicitModel } from "./personaDialogPickers";

export const MODEL_DISCOVERY_LOADING_VALUE = "__model_discovery_loading__";

const MODEL_DISCOVERY_CREDENTIAL_DEBOUNCE_MS = 250;

function stableModelDiscoveryEnvKey(envVars: EnvVarsValue): string {
  return JSON.stringify(
    Object.entries(envVars).sort(([left], [right]) =>
      left.localeCompare(right),
    ),
  );
}

function getDiscoveredPersonaModelOptions(
  response: AgentModelsResponse | null,
  provider: string,
): readonly PersonaModelOption[] | null {
  if (!response?.supportsSwitching || response.models.length === 0) {
    return null;
  }

  const defaultModelOption = providerRequiresExplicitModel(provider)
    ? []
    : [
        {
          id: "",
          label: response.agentDefaultModel?.trim()
            ? `Default model (${response.agentDefaultModel})`
            : "Default model",
        },
      ];

  return [
    ...defaultModelOption,
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
  const shouldDebounceModelDiscovery =
    providerRequiresExplicitModel(trimmedProvider);
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
    const activeAgentCommand = discoveryAgentCommand;
    const activeModelDiscoveryKey = modelDiscoveryKey;
    const cached = modelDiscoveryCacheRef.current.get(activeModelDiscoveryKey);
    if (cached) {
      setModelDiscoveryData(cached);
      setModelDiscoveryStatus(null);
      setModelDiscoveryLoading(false);
      return;
    }

    setModelDiscoveryData(null);
    setModelDiscoveryStatus(null);
    setModelDiscoveryLoading(true);
    function runModelDiscovery() {
      void discoverAgentModels({
        agentCommand: activeAgentCommand,
        agentArgs: selectedRuntime?.defaultArgs ?? [],
        provider: trimmedProvider || undefined,
        envVars,
      })
        .then((response) => {
          if (modelDiscoveryRequestRef.current !== requestId) {
            return;
          }
          modelDiscoveryCacheRef.current.set(activeModelDiscoveryKey, response);
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
    }

    if (!shouldDebounceModelDiscovery) {
      runModelDiscovery();
      return;
    }

    const timeout = window.setTimeout(
      runModelDiscovery,
      MODEL_DISCOVERY_CREDENTIAL_DEBOUNCE_MS,
    );

    return () => {
      window.clearTimeout(timeout);
      if (modelDiscoveryRequestRef.current === requestId) {
        modelDiscoveryRequestRef.current += 1;
        setModelDiscoveryLoading(false);
      }
    };
  }, [
    discoveryAgentCommand,
    envVars,
    modelDiscoveryKey,
    selectedRuntime?.defaultArgs,
    shouldDebounceModelDiscovery,
    trimmedProvider,
  ]);

  const discoveredModelOptions = React.useMemo(
    () => getDiscoveredPersonaModelOptions(modelDiscoveryData, trimmedProvider),
    [modelDiscoveryData, trimmedProvider],
  );

  return {
    discoveredModelOptions,
    modelDiscoveryLoading,
    modelDiscoveryStatus:
      modelDiscoveryLoading || discoveredModelOptions !== null
        ? null
        : modelDiscoveryStatus,
  };
}
