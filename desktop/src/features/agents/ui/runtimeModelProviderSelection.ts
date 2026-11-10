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
  envVarsWithoutKeyCaseInsensitive,
} from "./providerEnvVarUpdates";

/**
 * Pure transition functions for the runtime -> LLM provider -> model dropdown
 * state machine shared by the persona / create-agent / edit-agent dialogs.
 * Each dialog applies the returned state to its own setters and layers its
 * dialog-specific side effects (inherit pins, command sync, catalog memory)
 * at the call site. Divergent behaviors are parameterized, never merged.
 */

/**
 * Every runtime-owned thinking-effort env key: the native keys of all known
 * runtimes plus the retained ACP-startup transport sentinel. Mirrors the Rust
 * `effort_suppress_keys()` full sweep (`config_bridge/effort.rs`).
 *
 * On a runtime switch these aliases become stale — they express the *previous*
 * runtime's vocabulary — so they are cleared. The canonical persisted effort
 * (`record.effort_level`) is a Save-gated column persisted by
 * `AgentInstanceEditDialog` (AGENTS.md rule 14), lives outside this env-state
 * selection, and is therefore
 * PRESERVED across the switch: the launch projection normalizes it (or skips it
 * as absent) for the destination runtime, and switching back restores it.
 */
const EFFORT_ENV_ALIASES = [
  "GOOSE_THINKING_EFFORT",
  "BUZZ_AGENT_THINKING_EFFORT",
  "BUZZ_ACP_EFFORT_LEVEL",
] as const;
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

  // F3 nondestructive switch policy: clear the previous runtime's stale
  // thinking-effort env aliases (all native keys + the ACP sentinel). The
  // canonical `record.effort_level` column is Save-gated and not part of this
  // selection state, so it is preserved — the launch projection re-expresses it
  // for the destination runtime, and switching back restores the preference.
  if (params.previousRuntime !== params.nextRuntime) {
    let envVars = next.envVars;
    for (const key of EFFORT_ENV_ALIASES) {
      // Case-insensitive: Windows Command case-folds env names, so a hand-set
      // `goose_thinking_effort` is the same variable as its canonical form and
      // must be swept too. Mirrors the Rust `effort_suppress_keys()` sweep.
      envVars = envVarsWithoutKeyCaseInsensitive(envVars, key);
    }
    next.envVars = envVars;
  }

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
