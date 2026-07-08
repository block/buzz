import type { EnvVarsValue } from "./EnvVarsEditor";
import { getProviderApiKeyEnvVar } from "./personaDialogPickers";

/**
 * Pure env-var update helpers shared by the persona / create-agent /
 * edit-agent dialogs. Every function returns the SAME reference when nothing
 * changes, so `setEnvVars(fn(current))` skips a no-op re-render.
 */

/** Set `envKey` to `value`, or remove it when `value` is empty. */
export function envVarsWithProviderApiKey(
  current: EnvVarsValue,
  envKey: string,
  value: string,
): EnvVarsValue {
  if ((current[envKey] ?? "") === value) {
    return current;
  }

  const next = { ...current };
  if (value.length > 0) {
    next[envKey] = value;
  } else {
    delete next[envKey];
  }
  return next;
}

/** Remove `envKey` when present. */
export function envVarsWithoutKey(
  current: EnvVarsValue,
  envKey: string,
): EnvVarsValue {
  if (!(envKey in current)) {
    return current;
  }

  const next = { ...current };
  delete next[envKey];
  return next;
}

/**
 * Clear the previous provider's managed API key when switching providers.
 * No-op when the previous provider has no managed key or the next provider
 * uses the same one.
 */
export function envVarsClearingManagedApiKey(
  current: EnvVarsValue,
  previousProvider: string,
  nextProvider: string,
): EnvVarsValue {
  const previousEnvVar = getProviderApiKeyEnvVar(previousProvider);
  const nextEnvVar = getProviderApiKeyEnvVar(nextProvider);
  if (previousEnvVar && previousEnvVar !== nextEnvVar) {
    return envVarsWithoutKey(current, previousEnvVar);
  }
  return current;
}

/**
 * Apply an Advanced-section env-vars edit while preserving the managed
 * provider API key (which is edited via its own field, not the editor).
 */
export function envVarsMergingAdvancedEdit(
  current: EnvVarsValue,
  nextAdvancedEnvVars: EnvVarsValue,
  managedEnvKey: string | null,
): EnvVarsValue {
  if (!managedEnvKey || !(managedEnvKey in current)) {
    return nextAdvancedEnvVars;
  }

  return {
    ...nextAdvancedEnvVars,
    [managedEnvKey]: current[managedEnvKey],
  };
}
