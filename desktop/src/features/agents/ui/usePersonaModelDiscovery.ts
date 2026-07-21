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
import type { PersonaModelOption } from "./agentConfigOptions";
import { providerRequiresExplicitModel } from "./agentConfigOptions";

export const MODEL_DISCOVERY_LOADING_VALUE = "__model_discovery_loading__";

const MODEL_DISCOVERY_CREDENTIAL_DEBOUNCE_MS = 250;

function stableModelDiscoveryEnvKey(envVars: EnvVarsValue): string {
  return JSON.stringify(
    Object.entries(envVars).sort(([left], [right]) =>
      left.localeCompare(right),
    ),
  );
}

/**
 * True when a harness catalog entry is the harness's own "use my default"
 * row (e.g. Claude Code ships a literal `default` model id). Such entries
 * mean the same thing as leaving the model unset, so the UI merges them
 * into the single canonical default row instead of showing two rows for
 * one idea.
 */
function isHarnessDefaultModelEntry(model: { id: string }) {
  return model.id.trim().toLowerCase() === "default";
}

export function getDiscoveredPersonaModelOptions(
  response: AgentModelsResponse | null,
  provider: string,
): readonly PersonaModelOption[] | null {
  if (!response?.supportsSwitching || response.models.length === 0) {
    return null;
  }

  // One row per idea: the harness's own default catalog entry (if any) is
  // absorbed into the canonical default row. Selecting it keeps the stored
  // model unset — behaviorally identical, and it avoids two saved states
  // ("default" vs unset) that mean the same thing.
  const explicitModels = response.models.filter(
    (model) => !isHarnessDefaultModelEntry(model),
  );
  const harnessDefaultEntry = response.models.find(isHarnessDefaultModelEntry);
  const agentDefaultModel = response.agentDefaultModel?.trim();

  const defaultModelOption =
    providerRequiresExplicitModel(provider) && harnessDefaultEntry === undefined
      ? []
      : [
          {
            id: "",
            label:
              provider === "relay-mesh"
                ? "Default (auto)"
                : agentDefaultModel
                  ? `Default model (${agentDefaultModel})`
                  : "Default model",
          },
        ];

  if (explicitModels.length === 0 && defaultModelOption.length === 0) {
    return null;
  }

  return [
    ...defaultModelOption,
    ...explicitModels.map((model) => ({
      id: model.id,
      label: model.name?.trim() || model.id,
    })),
  ];
}

/**
 * Returns a warning status when a discovery response resolves but contains no
 * usable model options (either because the harness does not support model
 * switching, or because it returned an empty model list). When options ARE
 * available, returns null so callers can clear any prior status.
 */
export function synthesizeEmptyDiscoveryStatus(
  response: AgentModelsResponse,
  provider: string,
): PersonaModelDiscoveryStatus | null {
  if (getDiscoveredPersonaModelOptions(response, provider) !== null) {
    return null;
  }
  const agentLabel = response.agentName.trim() || "This agent";
  return {
    message: `${agentLabel} reported no models. Check that the CLI is installed and signed in, then reopen this screen.`,
    tone: "warning",
  };
}

/**
 * True when a discovery response is worth caching.  Responses that yielded no
 * usable model options are intentionally excluded so that close → reopen
 * re-runs discovery, letting the user's CLI install or sign-in be reflected
 * without a hard refresh.
 */
export function isCacheableDiscoveryResponse(
  response: AgentModelsResponse,
  provider: string,
): boolean {
  return getDiscoveredPersonaModelOptions(response, provider) !== null;
}

/**
 * Pure derivation of the "discovery is still pending" flag exposed by the
 * hook.  Extracted so tests can verify resolved-but-empty responses do not
 * count as pending.
 */
export function deriveModelDiscoveryPending({
  modelDiscoveryLoading,
  modelDiscoveryKey,
  activeModelDiscoveryData,
  activeModelDiscoveryStatus,
}: {
  modelDiscoveryLoading: boolean;
  modelDiscoveryKey: string | null;
  activeModelDiscoveryData: AgentModelsResponse | null;
  activeModelDiscoveryStatus: PersonaModelDiscoveryStatus | null;
}): boolean {
  return (
    modelDiscoveryLoading ||
    (modelDiscoveryKey !== null &&
      activeModelDiscoveryData === null &&
      activeModelDiscoveryStatus === null)
  );
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
  const [modelDiscoveryDataKey, setModelDiscoveryDataKey] = React.useState<
    string | null
  >(null);
  const [modelDiscoveryStatus, setModelDiscoveryStatus] =
    React.useState<PersonaModelDiscoveryStatus | null>(null);
  const [modelDiscoveryStatusKey, setModelDiscoveryStatusKey] = React.useState<
    string | null
  >(null);
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
  // Narrow to the individual fields the effect consumes so a new object
  // reference from a React Query refetch (same data, unstable ref) does not
  // abandon and re-issue an in-flight discovery IPC call.
  const selectedRuntimeAvailability = selectedRuntime?.availability;
  const selectedRuntimeDefaultArgs = selectedRuntime?.defaultArgs;
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
      setModelDiscoveryDataKey(null);
      // When the runtime exists but is not available, surface a status message
      // so the model dropdown explains why no live models can be loaded.
      if (
        selectedRuntimeAvailability != null &&
        selectedRuntimeAvailability !== "available"
      ) {
        setModelDiscoveryStatus(
          formatModelDiscoveryErrorStatus(
            new Error(`Runtime not available: ${selectedRuntimeAvailability}`),
            trimmedProvider,
          ),
        );
        setModelDiscoveryStatusKey(null);
      } else {
        setModelDiscoveryStatus(null);
        setModelDiscoveryStatusKey(null);
      }
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
      setModelDiscoveryDataKey(activeModelDiscoveryKey);
      setModelDiscoveryStatus(
        synthesizeEmptyDiscoveryStatus(cached, trimmedProvider),
      );
      setModelDiscoveryStatusKey(activeModelDiscoveryKey);
      setModelDiscoveryLoading(false);
      return;
    }

    setModelDiscoveryData(null);
    setModelDiscoveryDataKey(null);
    setModelDiscoveryStatus(null);
    setModelDiscoveryStatusKey(activeModelDiscoveryKey);
    setModelDiscoveryLoading(true);
    function runModelDiscovery() {
      void discoverAgentModels({
        agentCommand: activeAgentCommand,
        agentArgs: selectedRuntimeDefaultArgs ?? [],
        provider: trimmedProvider || undefined,
        envVars,
      })
        .then((response) => {
          if (modelDiscoveryRequestRef.current !== requestId) {
            return;
          }
          // Only cache responses that yielded usable model options.  An
          // empty/no-switching result gets the "reopen this screen" warning,
          // and closing → reopening the dialog must re-run discovery so the
          // user's CLI-install/sign-in is actually reflected.
          if (isCacheableDiscoveryResponse(response, trimmedProvider)) {
            modelDiscoveryCacheRef.current.set(
              activeModelDiscoveryKey,
              response,
            );
          }
          setModelDiscoveryData(response);
          setModelDiscoveryDataKey(activeModelDiscoveryKey);
          setModelDiscoveryStatus(
            synthesizeEmptyDiscoveryStatus(response, trimmedProvider),
          );
          setModelDiscoveryStatusKey(activeModelDiscoveryKey);
        })
        .catch((error) => {
          if (modelDiscoveryRequestRef.current !== requestId) {
            return;
          }
          setModelDiscoveryData(null);
          setModelDiscoveryDataKey(null);
          setModelDiscoveryStatus(
            formatModelDiscoveryErrorStatus(error, trimmedProvider),
          );
          setModelDiscoveryStatusKey(activeModelDiscoveryKey);
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
    selectedRuntimeAvailability,
    selectedRuntimeDefaultArgs,
    shouldDebounceModelDiscovery,
    trimmedProvider,
  ]);

  const activeModelDiscoveryData =
    modelDiscoveryKey !== null && modelDiscoveryDataKey === modelDiscoveryKey
      ? modelDiscoveryData
      : null;
  const activeModelDiscoveryStatus =
    modelDiscoveryKey === null
      ? modelDiscoveryStatus
      : modelDiscoveryStatusKey === modelDiscoveryKey
        ? modelDiscoveryStatus
        : null;
  const discoveredModelOptions = React.useMemo(
    () =>
      getDiscoveredPersonaModelOptions(
        activeModelDiscoveryData,
        trimmedProvider,
      ),
    [activeModelDiscoveryData, trimmedProvider],
  );
  const modelDiscoveryPending = deriveModelDiscoveryPending({
    modelDiscoveryLoading,
    modelDiscoveryKey,
    activeModelDiscoveryData,
    activeModelDiscoveryStatus,
  });

  return {
    discoveredModelOptions,
    modelDiscoveryLoading: modelDiscoveryPending,
    modelDiscoveryStatus:
      modelDiscoveryPending || discoveredModelOptions !== null
        ? null
        : activeModelDiscoveryStatus,
  };
}
