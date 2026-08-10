import type { EnvVarsValue } from "./EnvVarsEditor";
import {
  AUTO_MODEL_DROPDOWN_VALUE,
  RELAY_MESH_AUTO_MODEL,
  RELAY_MESH_PROVIDER,
  AUTO_PROVIDER_DROPDOWN_VALUE,
  CUSTOM_MODEL_DROPDOWN_VALUE,
  CUSTOM_PROVIDER_DROPDOWN_VALUE,
  getProviderApiKeyEnvVar,
  shouldClearKnownModelForSelectionScope,
} from "./agentConfigOptions";
import { shouldClearModelForRuntimeChange } from "./personaRuntimeModel";
import {
  envVarsClearingManagedApiKey,
  envVarsWithoutKey,
} from "./providerEnvVarUpdates";

/**
 * Pure transition functions for the runtime -> LLM provider -> model dropdown
 * state machine shared by the persona / create-agent / edit-agent dialogs.
 * Each dialog applies the returned state to its own setters and layers its
 * dialog-specific side effects (inherit pins, command sync, catalog memory)
 * at the call site. Divergent behaviors are parameterized, never merged.
 */
export type RuntimeModelProviderSelection = {
  provider: string;
  model: string;
  isCustomProviderEditing: boolean;
  isCustomModelEditing: boolean;
  envVars: EnvVarsValue;
};

export function selectionOnRuntimeChange(
  current: RuntimeModelProviderSelection,
  params: {
    previousRuntime: string;
    nextRuntime: string;
    /** Caller-computed: whether the next runtime supports provider selection. */
    nextRuntimeCanChooseProvider: boolean;
    /**
     * Persona/Edit clear the managed API key and custom-model editing flag
     * when switching to a provider-locked runtime ("full"); Create clears
     * only the provider selection ("provider-only").
     */
    lockedRuntimeReset: "full" | "provider-only";
  },
): RuntimeModelProviderSelection {
  const next = { ...current };

  if (
    shouldClearModelForRuntimeChange(
      params.previousRuntime,
      params.nextRuntime,
    ) ||
    shouldClearKnownModelForSelectionScope({
      model: current.model,
      provider: current.provider,
      runtime: params.nextRuntime,
    })
  ) {
    next.model = "";
    next.isCustomModelEditing = false;
  }

  if (!params.nextRuntimeCanChooseProvider) {
    if (params.lockedRuntimeReset === "full") {
      next.envVars = envVarsClearingManagedApiKey(
        next.envVars,
        current.provider,
        "",
      );
      next.isCustomModelEditing = false;
    }
    next.isCustomProviderEditing = false;
    next.provider = "";
  }

  return next;
}

export function selectionOnProviderDropdownChange(
  current: RuntimeModelProviderSelection,
  params: {
    /** Runtime id used for the model-scope clearing rule. */
    runtime: string;
    nextValue: string;
    /**
     * Persona-only: clear the model when the newly selected provider's API
     * key is not yet filled (model discovery cannot run without it).
     */
    clearModelWhenApiKeyMissing: boolean;
  },
): RuntimeModelProviderSelection {
  const next = { ...current };

  if (params.nextValue === CUSTOM_PROVIDER_DROPDOWN_VALUE) {
    const previousEnvVar = getProviderApiKeyEnvVar(current.provider);
    if (previousEnvVar) {
      next.envVars = envVarsWithoutKey(next.envVars, previousEnvVar);
    }
    next.isCustomProviderEditing = true;
    next.provider = "";
    return next;
  }

  const nextProvider =
    params.nextValue === AUTO_PROVIDER_DROPDOWN_VALUE ? "" : params.nextValue;
  next.envVars = envVarsClearingManagedApiKey(
    next.envVars,
    current.provider,
    nextProvider,
  );
  next.isCustomProviderEditing = false;
  next.provider = nextProvider;

  if (params.clearModelWhenApiKeyMissing) {
    const requiredEnvVar = getProviderApiKeyEnvVar(nextProvider);
    if (requiredEnvVar && !next.envVars[requiredEnvVar]?.trim()) {
      next.model = "";
      next.isCustomModelEditing = false;
    }
  }

  // Guard on the PRE-transition editing flag, matching all three dialogs
  // (their handlers read the render-scope value).
  if (
    !current.isCustomModelEditing &&
    shouldClearKnownModelForSelectionScope({
      model: current.model,
      provider: nextProvider,
      runtime: params.runtime,
    })
  ) {
    next.model = "";
    next.isCustomModelEditing = false;
  }

  return next;
}

export function selectionOnModelDropdownChange(
  current: RuntimeModelProviderSelection,
  params: {
    nextValue: string;
    /**
     * Persona clears a known (non-custom) model when entering custom mode;
     * Create/Edit keep it as the editable starting value.
     */
    clearKnownModelOnCustomEntry: boolean;
    /** Caller-computed: whether the current model is outside the known options. */
    isModelCustom: boolean;
    /**
     * Caller-computed: whether the effective provider is Buzz shared compute.
     *
     * Shared compute encodes Auto as the literal `"auto"`, not as blank. Blank
     * means *inherit the global model*, so a blank Auto on shared compute both
     * resolves to whatever unrelated model the global config names (e.g. a
     * Claude id sent to a local mesh node) and fails the raw-pair submit gate in
     * `agentAiConfigurationModeSatisfied`, which requires a non-empty model.
     *
     * `"auto"` is also a trigger token rather than a final choice: Buzz sets
     * `BUZZ_AGENT_PREFER_MESH_FOR_AUTO` for this provider, and buzz-agent
     * rewrites exactly `"auto"` to the virtual `mesh` model when the live
     * catalog offers enough models for Mixture-of-Agents. Any concrete model id
     * defeats that translation.
     *
     * Optional. Callers that already derive it (because the picker can be driven
     * by an *inherited* effective provider, not just the selection's own) should
     * pass it; otherwise the selection's own provider is used, which covers every
     * case where the agent names the provider itself.
     */
    isRelayMesh?: boolean;
  },
): RuntimeModelProviderSelection {
  const next = { ...current };

  if (params.nextValue === CUSTOM_MODEL_DROPDOWN_VALUE) {
    next.isCustomModelEditing = true;
    if (params.clearKnownModelOnCustomEntry && !params.isModelCustom) {
      next.model = "";
    }
    return next;
  }

  const isRelayMesh =
    params.isRelayMesh ?? current.provider.trim() === RELAY_MESH_PROVIDER;

  next.isCustomModelEditing = false;
  next.model =
    params.nextValue === AUTO_MODEL_DROPDOWN_VALUE
      ? isRelayMesh
        ? RELAY_MESH_AUTO_MODEL
        : ""
      : params.nextValue;
  return next;
}
