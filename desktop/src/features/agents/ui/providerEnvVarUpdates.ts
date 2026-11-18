import type { EnvVarsValue } from "./EnvVarsEditor";
import { getProviderApiKeyEnvVar } from "./agentConfigOptions";

/**
 * Pure env-var update helpers shared by the persona / create-agent /
 * edit-agent dialogs. Every function returns the SAME reference when nothing
 * changes, so `setEnvVars(fn(current))` skips a no-op re-render.
 */

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

/** Remove every case-insensitive (ASCII) match of `envKey` when present. */
export function envVarsWithoutKeyCaseInsensitive(
  current: EnvVarsValue,
  envKey: string,
): EnvVarsValue {
  const lower = envKey.toLowerCase();
  const matches = Object.keys(current).filter((k) => k.toLowerCase() === lower);
  if (matches.length === 0) {
    return current;
  }
  const next = { ...current };
  for (const match of matches) {
    delete next[match];
  }
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
