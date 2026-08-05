import type { EnvVarsValue } from "./EnvVarsEditor";
import {
  AUTO_MODEL_DROPDOWN_VALUE,
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
import { BUZZ_AGENT_THINKING_EFFORT } from "./buzzAgentConfig";

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
    /**
     * Native thinking-effort key for the PREVIOUS runtime, or `null` when the
     * previous runtime has no effort key or it equals the legacy key.
     * When provided, this key is deleted from envVars on runtime change.
     */
    previousRuntimeNativeEffortKey?: string | null;
    /**
     * Native thinking-effort key for the NEXT runtime, or `null` when the
     * next runtime has no effort key or it equals the legacy key.
     * When provided, this key is deleted from envVars on runtime change
     * (any stale leftover from a prior session is removed).
     */
    nextRuntimeNativeEffortKey?: string | null;
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

  // Effort cleanup at record/persona scope: clear prev native, next native,
  // and legacy keys when the runtime changes. Global scope is exempt (Delta-5
  // rule: global switches preserve both runtimes' native keys — that is handled
  // by resetConfigForHarnessChange's no-op policy).
  //
  // When BOTH native keys are non-legacy (e.g. claude↔goose), only the
  // previous side was cleared before — leaving the destination-native key as
  // a stale record entry the new runtime would silently inherit at spawn.
  // Fix: clone once and independently delete all three keys whose harness
  // now differs. Each deletion is conditional only on whether the key is a
  // non-legacy native key (buzz-agent: native === legacy, so buzz→buzz keeps
  // effort); deleting the same key twice from one clone is a no-op.
  {
    const prevKey = params.previousRuntimeNativeEffortKey;
    const nextKey = params.nextRuntimeNativeEffortKey;
    const hasNonLegacyKey =
      (prevKey && prevKey !== BUZZ_AGENT_THINKING_EFFORT) ||
      (nextKey && nextKey !== BUZZ_AGENT_THINKING_EFFORT);

    if (hasNonLegacyKey) {
      const ev = { ...next.envVars };
      if (prevKey && prevKey !== BUZZ_AGENT_THINKING_EFFORT) {
        delete ev[prevKey];
      }
      if (nextKey && nextKey !== BUZZ_AGENT_THINKING_EFFORT) {
        delete ev[nextKey];
      }
      delete ev[BUZZ_AGENT_THINKING_EFFORT];
      next.envVars = ev;
    }
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

  next.isCustomModelEditing = false;
  next.model =
    params.nextValue === AUTO_MODEL_DROPDOWN_VALUE ? "" : params.nextValue;
  return next;
}
